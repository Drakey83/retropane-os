//! What the Today screen's calendar panel shows, from the one store
//! (calendar-rule0.md and tasks-rule0.md: "Today screen's calendar panel
//! fed from the store", "Today screen's tasks line fed from the store").
//!
//! Found: Windows Mobile's Today plug-ins each opened their app's database
//! and walked it on every repaint of the Today screen; Android's calendar
//! widget asks a content provider and is told of changes. Ours keeps the
//! answer ([`Feed`]) and works it again only when the store changes, the
//! day turns or the minute moves (an appointment ends): one pass over the
//! records a change, not one a frame, and nothing is copied out of the
//! store but the two rows and three numbers drawn.

use rp_pim::date;
use rp_pim::repeat::Rule;
use rp_pim::store::{Kind, Store, tag};
use rp_pim::zone;

/// The rows the panel has room for.
pub(crate) const ROWS: usize = 2;
/// A row's title bytes kept (the panel cuts it with "..").
const TITLE: usize = 40;

/// One row: an appointment, a birthday, or (the tasks row) a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    /// Its start (minutes from midnight); `None` all day.
    pub at: Option<u16>,
    /// It is tomorrow's (nothing more today left room for it).
    pub tomorrow: bool,
    title: [u8; TITLE],
    n: u8,
}

impl Row {
    const EMPTY: Self = Self {
        at: None,
        tomorrow: false,
        title: [0; TITLE],
        n: 0,
    };

    fn of(at: Option<u16>, tomorrow: bool, s: &[u8]) -> Self {
        let mut r = Self {
            at,
            tomorrow,
            ..Self::EMPTY
        };
        let n = s.len().min(TITLE);
        r.title[..n].copy_from_slice(&s[..n]);
        r.n = n as u8;
        r
    }

    /// Its title.
    pub fn title(&self) -> &[u8] {
        &self.title[..usize::from(self.n)]
    }
}

/// What the panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Feed {
    /// The store, the day and the minute it was worked out for.
    key: Option<(u32, i32, u16, i32)>,
    /// The store is there and loaded (else the panel says it is coming).
    pub ready: bool,
    /// What is left of today, in order (all day first), then tomorrow's
    /// first when today has no more.
    pub rows: [Row; ROWS],
    pub n: u8,
    /// Today's left that did not fit.
    pub more: u8,
    /// Open tasks due today or before, and how many of them are late.
    pub due: u16,
    pub late: u16,
}

impl Feed {
    pub(crate) const fn new() -> Self {
        Self {
            key: None,
            ready: false,
            rows: [Row::EMPTY; ROWS],
            n: 0,
            more: 0,
            due: 0,
            late: 0,
        }
    }

    /// Worked again from `store` if it changed, the day turned or the
    /// minute moved since; `None` (no store yet) shows as not ready.
    pub(crate) fn update(
        &mut self,
        store: Option<&Store<'_>>,
        today: i32,
        minute: u16,
        utc_offset: i32,
    ) {
        let Some(s) = store.filter(|s| s.loaded()) else {
            *self = Self::new();
            return;
        };
        let key = (s.changes(), today, minute, utc_offset);
        if self.key == Some(key) {
            return;
        }
        *self = feed(s, today, minute, utc_offset);
        self.key = Some(key);
    }
}

/// One candidate row while sorting: (day offset, all day first, start,
/// the row).
type Cand = (u8, u16, Row);

/// Keeps the earliest `ROWS` candidates (sorted by day, then start; all day
/// first) and counts the rest of today.
struct Pick {
    best: [Option<Cand>; ROWS + 1],
    today: u16,
}

impl Pick {
    fn add(&mut self, c: Cand) {
        if c.0 == 0 {
            self.today += 1;
        }
        let k = |c: &Cand| (c.0, c.1);
        // Insertion into the few kept.
        let mut c = Some(c);
        for slot in &mut self.best {
            match (slot.as_ref(), c.as_ref()) {
                (None, Some(_)) => {
                    *slot = c.take();
                    return;
                }
                (Some(s), Some(n)) if k(n) < k(s) => {
                    core::mem::swap(slot, &mut c);
                }
                _ => {}
            }
        }
    }
}

fn feed(s: &Store<'_>, today: i32, minute: u16, utc_offset: i32) -> Feed {
    let mut out = Feed::new();
    out.ready = true;
    let mut pick = Pick {
        best: [None; ROWS + 1],
        today: 0,
    };
    // Hidden calendars stay off the panel as they do off Calendar's views.
    let hidden = |cal: Option<u32>| {
        cal.and_then(|c| s.get(c))
            .is_some_and(|c| c.num(tag::HIDDEN) == Some(1))
    };
    let local = i64::from(today) * 1440 + i64::from(minute);
    let device = zone::device(s, local, utc_offset).0;
    for r in s.all(Kind::Appointment) {
        if hidden(r.num(tag::CAL)) {
            continue;
        }
        let start = r.num(tag::START).unwrap_or(0);
        let first = date::day_of(start);
        let wall = (start % date::DAY_MIN) as u16;
        let all_day = r.num(tag::ALL_DAY) == Some(1);
        let len = r.num(tag::MINUTES).unwrap_or(60).clamp(1, 24 * 60) as u16;
        let rule = r.num(tag::REPEAT).and_then(|v| Rule::unpack(v, first));
        // A time kept in another zone shows in the device's, as Calendar
        // shows it (rp_pim::zone::shown, one occurrence at a time: a clock
        // change between two moves one and not the other); a shift can
        // bring yesterday's occurrence into today or push one past it.
        let zone = r.text(tag::ZONE);
        for occ in today - 1..today + 2 {
            let on = match rule {
                Some(x) => x.occurs(first, occ),
                None => first == occ,
            };
            if !on {
                continue;
            }
            let (day, at) = if all_day || zone.is_empty() {
                (occ, wall)
            } else {
                let t = zone::shown(device, zone, date::start_of(occ) + u32::from(wall));
                (date::day_of(t), (t % date::DAY_MIN) as u16)
            };
            let Ok(off) = u8::try_from(day - today) else {
                continue;
            };
            // Today's that have ended are past.
            if off > 1 || (off == 0 && !all_day && at + len <= minute) {
                continue;
            }
            let row = Row::of((!all_day).then_some(at), off == 1, r.text(tag::TITLE));
            pick.add((off, if all_day { 0 } else { 1 + at }, row));
        }
    }
    // The Birthdays calendar's days, unless it is hidden.
    if !rp_pim::birthdays::calendar(s).is_some_and(|c| hidden(Some(c))) {
        rp_pim::birthdays::all(s, today, 2, |e| {
            let Some(r) = s.get(e.id) else { return };
            let mut t = [0u8; TITLE];
            let n = rp_pim::birthdays::title(&r, &e, &mut t);
            let off = (e.day - today) as u8;
            pick.add((off, 0, Row::of(None, off == 1, &t[..n])));
        });
    }
    // The earliest: today's first, tomorrow's in the room they leave.
    let mut n = 0;
    let mut shown_today = 0u16;
    for c in pick.best.iter().flatten().take(ROWS) {
        shown_today += u16::from(c.0 == 0);
        out.rows[n] = c.2;
        n += 1;
    }
    out.n = n as u8;
    out.more = pick.today.saturating_sub(shown_today).min(255) as u8;
    // The tasks: open ones (steps aside) due today or before.
    for r in s.all(Kind::Task) {
        if r.num(tag::DONE).is_some() || r.num(tag::PARENT).is_some() {
            continue;
        }
        let Some(due) = r.num(tag::DUE).map(|d| d as i32) else {
            continue;
        };
        if due > today {
            continue;
        }
        out.due = out.due.saturating_add(1);
        if due < today {
            out.late = out.late.saturating_add(1);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    const TODAY: i32 = 19_723;

    fn store() -> Store<'static> {
        let mem = vec![0u8; rp_pim::store::MEMORY].leak();
        let mut s = Store::new(mem);
        s.load_start(None);
        s
    }

    fn appt(s: &mut Store<'_>, title: &[u8], day: i32, at: u32, len: u32, more: &[(u8, &[u8])]) {
        let st = (date::start_of(day) + at).to_le_bytes();
        let ln = len.to_le_bytes();
        let mut f: vec::Vec<(u8, &[u8])> =
            vec![(tag::TITLE, title), (tag::START, &st), (tag::MINUTES, &ln)];
        f.extend_from_slice(more);
        s.put(Kind::Appointment, None, &f).unwrap();
    }

    fn titles(f: &Feed) -> vec::Vec<(&[u8], Option<u16>, bool)> {
        f.rows[..usize::from(f.n)]
            .iter()
            .map(|r| (r.title(), r.at, r.tomorrow))
            .collect()
    }

    #[test]
    fn the_panel_shows_what_is_left_of_today_in_order() {
        let mut s = store();
        let one = 1u32.to_le_bytes();
        appt(&mut s, b"Dentist", TODAY, 15 * 60, 60, &[]);
        appt(&mut s, b"Stand-up", TODAY, 9 * 60, 30, &[]);
        appt(&mut s, b"Holiday", TODAY, 0, 60, &[(tag::ALL_DAY, &one)]);
        appt(&mut s, b"Gym", TODAY + 1, 18 * 60, 60, &[]);
        let hid = s
            .put(
                Kind::Calendar,
                None,
                &[(tag::NAME, b"Old"), (tag::HIDDEN, &one)],
            )
            .unwrap();
        appt(
            &mut s,
            b"Secret",
            TODAY,
            10 * 60,
            60,
            &[(tag::CAL, &hid.to_le_bytes())],
        );
        let mut f = Feed::new();
        // 8 AM: all day first, then the stand-up; the dentist is more.
        f.update(Some(&s), TODAY, 8 * 60, 0);
        assert!(f.ready);
        assert_eq!(
            titles(&f),
            [
                (&b"Holiday"[..], None, false),
                (b"Stand-up", Some(540), false)
            ]
        );
        assert_eq!(f.more, 1);
        // 9:30: the stand-up has ended.
        f.update(Some(&s), TODAY, 9 * 60 + 30, 0);
        assert_eq!(
            titles(&f),
            [
                (&b"Holiday"[..], None, false),
                (b"Dentist", Some(900), false)
            ]
        );
        assert_eq!(f.more, 0);
    }

    #[test]
    fn with_nothing_left_today_tomorrows_first_shows() {
        let mut s = store();
        appt(&mut s, b"Dentist", TODAY, 15 * 60, 60, &[]);
        appt(&mut s, b"Gym", TODAY + 1, 18 * 60, 60, &[]);
        appt(&mut s, b"Car", TODAY + 1, 8 * 60, 60, &[]);
        let mut f = Feed::new();
        f.update(Some(&s), TODAY, 14 * 60, 0);
        // Today's one left, then tomorrow's first.
        assert_eq!(
            titles(&f),
            [
                (&b"Dentist"[..], Some(900), false),
                (b"Car", Some(480), true)
            ]
        );
        f.update(Some(&s), TODAY, 17 * 60, 0);
        assert_eq!(
            titles(&f),
            [(&b"Car"[..], Some(480), true), (b"Gym", Some(1080), true)]
        );
    }

    #[test]
    fn repeats_and_birthdays_count_and_no_store_is_not_ready() {
        use rp_pim::contact::Day;
        let mut s = store();
        let weekly = rp_pim::repeat::Rule {
            every: rp_pim::repeat::Every::Week,
            n: 1,
            days: 0,
            until: None,
            after: false,
        }
        .pack(TODAY - 7)
        .to_le_bytes();
        appt(
            &mut s,
            b"Choir",
            TODAY - 7,
            19 * 60,
            60,
            &[(tag::REPEAT, &weekly)],
        );
        let born = Day {
            year: Some(1990),
            month: 1,
            day: 1,
        }
        .field()
        .unwrap();
        s.put(
            Kind::Contact,
            None,
            &[(tag::FIRST, b"Ann"), (tag::BIRTHDAY, &born)],
        )
        .unwrap();
        let mut f = Feed::new();
        f.update(Some(&s), TODAY, 12 * 60, 0);
        assert_eq!(
            titles(&f),
            [
                (&b"Ann's 34th birthday"[..], None, false),
                (b"Choir", Some(1140), false)
            ]
        );
        f.update(None, TODAY, 12 * 60, 0);
        assert!(!f.ready);
    }

    #[test]
    fn the_tasks_line_counts_due_and_late_and_names_the_most_pressing() {
        let mut s = store();
        let d = |x: i32| (x as u32).to_le_bytes();
        let (t0, t1, t2, tf) = (d(TODAY), d(TODAY - 1), d(TODAY - 3), d(TODAY + 2));
        let high = 3u32.to_le_bytes();
        s.put(
            Kind::Task,
            None,
            &[
                (tag::TITLE, b"Call Jo"),
                (tag::DUE, &t0),
                (tag::PRIORITY, &high),
            ],
        )
        .unwrap();
        s.put(
            Kind::Task,
            None,
            &[(tag::TITLE, b"Pay rent"), (tag::DUE, &t1)],
        )
        .unwrap();
        s.put(Kind::Task, None, &[(tag::TITLE, b"Taxes"), (tag::DUE, &t2)])
            .unwrap();
        s.put(Kind::Task, None, &[(tag::TITLE, b"Later"), (tag::DUE, &tf)])
            .unwrap();
        let one = 1u32.to_le_bytes();
        s.put(
            Kind::Task,
            None,
            &[(tag::TITLE, b"Done"), (tag::DUE, &t1), (tag::DONE, &one)],
        )
        .unwrap();
        let mut f = Feed::new();
        f.update(Some(&s), TODAY, 600, 0);
        assert_eq!((f.due, f.late), (3, 2));
    }

    /// R376: an appointment kept in another zone shows at the device's
    /// wall time, as Calendar shows it, and on the device's day.
    #[test]
    fn a_zoned_appointment_shows_at_the_devices_time_and_day() {
        let mut s = store();
        s.put(
            Kind::Data,
            None,
            &[(tag::NAME, b"zone"), (tag::TEXT, b"America/Los_Angeles")],
        )
        .unwrap();
        let ny: &[(u8, &[u8])] = &[(tag::ZONE, b"America/New_York")];
        // 9 AM in New York is 6 AM on the device.
        appt(&mut s, b"Call", TODAY, 9 * 60, 30, ny);
        // 1 AM today in New York was 10 PM yesterday here: not shown.
        appt(&mut s, b"Night", TODAY, 60, 30, ny);
        // 2 AM tomorrow in New York is 11 PM today here.
        appt(&mut s, b"Late", TODAY + 1, 2 * 60, 30, ny);
        let mut f = Feed::new();
        f.update(Some(&s), TODAY, 5 * 60, -8 * 60);
        assert_eq!(
            titles(&f),
            [
                (&b"Call"[..], Some(6 * 60), false),
                (b"Late", Some(23 * 60), false)
            ]
        );
        // A time with no zone is the device's own, unchanged.
        let mut s = store();
        appt(&mut s, b"Here", TODAY, 9 * 60, 30, &[]);
        f.update(Some(&s), TODAY, 5 * 60, -8 * 60);
        assert_eq!(titles(&f), [(&b"Here"[..], Some(9 * 60), false)]);
    }
}
