//! When reminders are due (calendar-rule0.md and tasks-rule0.md: reminders
//! through the notification banner and sound, several per appointment).
//!
//! Found: Windows Mobile kept one alert per appointment and asked each
//! app to register it with the kernel's notification database
//! (`CeSetUserNotificationEx`), a second copy of the time that went stale
//! when a sync changed the record; iOS (`UNUserNotificationCenter`) and
//! Android (`AlarmManager`) do the same with a 64-pending cap or a
//! re-register on boot, and both allow several alerts (Google Calendar
//! five, iOS two).
//!
//! Ours keeps no second copy: the alerts are read from the one store
//! ([`crate::store`]) itself, so a record changed by the app, a sync or an
//! import reminds at its new time with nothing registered or forgotten.
//! The next alert is worked out once per change of the records (a scan of
//! a few comparisons a record, no list of occurrences expanded: a
//! repeating appointment's days are tested by [`crate::repeat::Rule::occurs`]
//! only near the window asked about), and the result is one minute: the
//! one time the device must be awake for (the clock's alarm when the OS
//! sleeps), nothing woken in between.

use crate::date::{self, DAY_MIN};
use crate::repeat::Rule;
use crate::store::{Kind, Rec, tag};

/// Alerts an appointment may have (Google Calendar's five) ([`tag::REMIND`] holds up to this many
/// leads, each a little-endian `u32` of minutes before its start).
pub const LEADS_MAX: usize = 5;

/// The longest lead looked for (four weeks): a window is widened by it.
pub const LEAD_MAX: u32 = 28 * DAY_MIN;

/// One alert: the record, when it sounds, and the start and end of the
/// occurrence it is for (a repeating appointment's day, not its first).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Alert {
    /// The record.
    pub id: u32,
    /// When it sounds (local minutes from 1970).
    pub at: u32,
    /// The occurrence's start (a task: its time).
    pub start: u32,
    /// When the occurrence is over (a task: the end of its day).
    pub end: u32,
}

/// An appointment's leads in minutes (none: no alert), from
/// [`tag::REMIND`]: one `u32` as before, or up to [`LEADS_MAX`].
pub fn leads<'a>(r: &Rec<'a>) -> impl Iterator<Item = u32> + 'a {
    r.text(tag::REMIND)
        .as_chunks::<4>()
        .0
        .iter()
        .take(LEADS_MAX)
        .map(|c| u32::from_le_bytes(*c).min(LEAD_MAX))
}

/// `leads` packed for [`tag::REMIND`] into `out`, latest-first and without
/// repeats (an empty slice: no alert).
pub fn pack(leads: &[u32], out: &mut [u8; 4 * LEADS_MAX]) -> usize {
    let mut v = [0u32; LEADS_MAX];
    let mut n = 0;
    for &l in leads {
        let l = l.min(LEAD_MAX);
        if n < LEADS_MAX && !v[..n].contains(&l) {
            v[n] = l;
            n += 1;
        }
    }
    v[..n].sort_unstable_by(|a, b| b.cmp(a));
    for (k, l) in v[..n].iter().enumerate() {
        out[4 * k..4 * k + 4].copy_from_slice(&l.to_le_bytes());
    }
    4 * n
}

/// Every alert of `r` with `from < at <= to`, each given to `f` (not in
/// time order). A task reminds at its time while it is not done; an
/// appointment at each lead before each occurrence.
pub fn alerts(r: &Rec<'_>, from: u32, to: u32, f: &mut impl FnMut(Alert)) {
    if to <= from {
        return;
    }
    match r.kind {
        Kind::Task => {
            if r.num(tag::DONE).is_some() {
                return;
            }
            if let Some(at) = r.num(tag::REMIND_AT)
                && at > from
                && at <= to
            {
                f(Alert {
                    id: r.id,
                    at,
                    start: at,
                    end: at.saturating_add(DAY_MIN),
                });
            }
        }
        Kind::Appointment => {
            let Some(first) = r.num(tag::START) else {
                return;
            };
            let all_day = r.num(tag::ALL_DAY) == Some(1);
            let len = if all_day {
                DAY_MIN
            } else {
                r.num(tag::MINUTES).unwrap_or(60)
            };
            let first_day = date::day_of(first);
            let minute = first % DAY_MIN;
            let rule = r.num(tag::REPEAT).and_then(|v| Rule::unpack(v, first_day));
            for lead in leads(r) {
                // The occurrences whose start is in (from + lead, to + lead].
                let (lo, hi) = (from.saturating_add(lead), to.saturating_add(lead));
                let mut emit = |start: u32| {
                    if start > lo && start <= hi {
                        f(Alert {
                            id: r.id,
                            at: start - lead,
                            start,
                            end: start.saturating_add(len),
                        });
                    }
                };
                match rule {
                    None => emit(first),
                    Some(rule) => {
                        // A window of a few days at most (the callers ask
                        // about a day or less); bounded whatever is asked.
                        let d0 = date::day_of(lo).max(first_day);
                        let d1 = date::day_of(hi).min(d0 + 366);
                        for day in d0..=d1 {
                            if rule.occurs(first_day, day) {
                                emit(date::start_of(day) + minute);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// The first alert of `r` after `after`, looking no further than `to`.
#[must_use]
pub fn next(r: &Rec<'_>, after: u32, to: u32) -> Option<u32> {
    let mut best: Option<u32> = None;
    alerts(r, after, to, &mut |a| {
        if best.is_none_or(|b| a.at < b) {
            best = Some(a.at);
        }
    });
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repeat::Every;
    use crate::store::Store;

    const DAY: i32 = 19_723; // Monday 1 January 2024

    fn store() -> Store<'static> {
        let mut s = Store::new(std::vec![0u8; crate::store::MEMORY].leak());
        s.load_start(None);
        s
    }

    fn appt(s: &mut Store<'_>, start: u32, leads: &[u32], repeat: Option<u32>) -> u32 {
        let mut buf = [0u8; 4 * LEADS_MAX];
        let n = pack(leads, &mut buf);
        let st = start.to_le_bytes();
        let rp = repeat.unwrap_or(0).to_le_bytes();
        let mut f: std::vec::Vec<(u8, &[u8])> = std::vec![
            (tag::TITLE, &b"A"[..]),
            (tag::START, &st),
            (tag::REMIND, &buf[..n])
        ];
        if repeat.is_some() {
            f.push((tag::REPEAT, &rp));
        }
        s.put(Kind::Appointment, None, &f).unwrap()
    }

    fn all(s: &Store<'_>, id: u32, from: u32, to: u32) -> std::vec::Vec<Alert> {
        let mut v = std::vec::Vec::new();
        alerts(&s.get(id).unwrap(), from, to, &mut |a| v.push(a));
        v.sort_by_key(|a| a.at);
        v
    }

    #[test]
    fn pack_orders_latest_first_drops_repeats_and_caps() {
        let mut b = [0u8; 4 * LEADS_MAX];
        assert_eq!(pack(&[], &mut b), 0);
        let n = pack(&[0, 15, 1440, 15, 60, 5, 30], &mut b);
        assert_eq!(n, 20, "five kept of six distinct");
        let got: std::vec::Vec<u32> = b
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect();
        assert_eq!(got, [1440, 60, 15, 5, 0]);
        let n = pack(&[u32::MAX], &mut b);
        assert_eq!(u32::from_le_bytes(b[..4].try_into().unwrap()), LEAD_MAX);
        assert_eq!(n, 4);
    }

    #[test]
    fn several_leads_each_remind_once_at_their_time() {
        let mut s = store();
        let start = date::start_of(DAY) + 15 * 60;
        let id = appt(&mut s, start, &[1440, 15, 0], None);
        let v = all(&s, id, start - 2 * DAY_MIN, start + 10);
        let at: std::vec::Vec<u32> = v.iter().map(|a| a.at).collect();
        assert_eq!(at, [start - 1440, start - 15, start]);
        assert!(v.iter().all(|a| a.start == start && a.end == start + 60));
        // The window is open below and closed above: each minute once.
        assert_eq!(all(&s, id, start - 15, start).len(), 1);
        assert_eq!(all(&s, id, start - 16, start - 15).len(), 1);
        assert!(all(&s, id, start, start + DAY_MIN).is_empty());
    }

    #[test]
    fn the_old_single_lead_still_reads() {
        let mut s = store();
        let start = date::start_of(DAY) + 9 * 60;
        let lead = 30u32.to_le_bytes();
        let st = start.to_le_bytes();
        let id = s
            .put(
                Kind::Appointment,
                None,
                &[(tag::START, &st), (tag::REMIND, &lead)],
            )
            .unwrap();
        assert_eq!(next(&s.get(id).unwrap(), 0, u32::MAX), Some(start - 30));
    }

    #[test]
    fn no_lead_no_alert_and_an_empty_window_none() {
        let mut s = store();
        let start = date::start_of(DAY) + 9 * 60;
        let id = appt(&mut s, start, &[], None);
        assert!(all(&s, id, 0, u32::MAX).is_empty());
        let id = appt(&mut s, start, &[0], None);
        assert!(all(&s, id, start, start).is_empty());
        assert!(all(&s, id, start + 1, start).is_empty());
    }

    #[test]
    fn a_repeating_appointment_reminds_on_each_occurrence() {
        let mut s = store();
        let start = date::start_of(DAY) + 10 * 60;
        let weekly = Rule {
            every: Every::Week,
            n: 1,
            days: 0,
            until: None,
        }
        .pack(DAY);
        let id = appt(&mut s, start, &[15], Some(weekly));
        // Three weeks on, the occurrence of that Monday, not the first.
        let week3 = start + 21 * DAY_MIN;
        let v = all(&s, id, week3 - DAY_MIN, week3);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].at, v[0].start), (week3 - 15, week3));
        // Nothing on the Tuesday.
        assert!(all(&s, id, week3, week3 + DAY_MIN).is_empty());
        // Before the first: nothing; next() from far before finds the first.
        assert!(all(&s, id, 0, start - 16).is_empty());
        assert_eq!(
            next(&s.get(id).unwrap(), start - DAY_MIN, start),
            Some(start - 15)
        );
    }

    #[test]
    fn a_lead_over_midnight_finds_the_next_days_occurrence() {
        let mut s = store();
        // Daily at 00:10, reminded an hour before: 23:10 the day before.
        let start = date::start_of(DAY) + 10;
        let daily = Rule {
            every: Every::Day,
            n: 1,
            days: 0,
            until: Some(DAY + 2),
        }
        .pack(DAY);
        let id = appt(&mut s, start, &[60], Some(daily));
        let v = all(&s, id, start, start + 3 * DAY_MIN);
        let at: std::vec::Vec<u32> = v.iter().map(|a| a.at).collect();
        assert_eq!(at, [start + DAY_MIN - 60, start + 2 * DAY_MIN - 60]);
    }

    #[test]
    fn an_all_day_event_lasts_its_day() {
        let mut s = store();
        let start = date::start_of(DAY);
        let st = start.to_le_bytes();
        let one = 1u32.to_le_bytes();
        let lead = 0u32.to_le_bytes();
        let id = s
            .put(
                Kind::Appointment,
                None,
                &[
                    (tag::START, &st),
                    (tag::ALL_DAY, &one),
                    (tag::REMIND, &lead),
                ],
            )
            .unwrap();
        let v = all(&s, id, start - 1, start);
        assert_eq!(v[0].end, start + DAY_MIN);
    }

    #[test]
    fn a_task_reminds_at_its_time_until_done() {
        let mut s = store();
        let at = date::start_of(DAY) + 15 * 60;
        let w = at.to_le_bytes();
        let id = s
            .put(
                Kind::Task,
                None,
                &[(tag::TITLE, b"Bank"), (tag::REMIND_AT, &w)],
            )
            .unwrap();
        let v = all(&s, id, at - 1, at);
        assert_eq!(
            v,
            [Alert {
                id,
                at,
                start: at,
                end: at + DAY_MIN
            }]
        );
        s.change(id, &[(tag::DONE, &w)]).unwrap();
        assert!(all(&s, id, at - 1, at).is_empty());
        // A task without a time, a note: none.
        let id = s.put(Kind::Task, None, &[(tag::TITLE, b"x")]).unwrap();
        assert!(all(&s, id, 0, u32::MAX).is_empty());
        let id = s.put(Kind::Note, None, &[(tag::REMIND_AT, &w)]).unwrap();
        assert!(all(&s, id, 0, u32::MAX).is_empty());
    }

    #[test]
    fn a_huge_window_over_a_repeat_stays_bounded() {
        let mut s = store();
        let start = date::start_of(DAY);
        let daily = Rule {
            every: Every::Day,
            n: 1,
            days: 0,
            until: None,
        }
        .pack(DAY);
        let id = appt(&mut s, start, &[0], Some(daily));
        assert!(all(&s, id, 0, u32::MAX).len() <= 367);
    }
}
