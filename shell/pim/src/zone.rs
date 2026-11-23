//! Time zones (calendar-rule0.md "Time zones"): an appointment keeps the
//! wall time of the zone it was made in, with the zone's name
//! ([`crate::store::tag::ZONE`]); every view shows it in the device's zone.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5 kept appointments in
//! UTC with one device-wide zone of 2005's rules (the 2007 US change broke
//! every phone that wasn't patched); desktops carry the whole tz database
//! with every past change (about 400 KB). The row asked for times kept in
//! UTC; RFC 5545 and Google's calendar keep the wall time and the zone's
//! name instead, and so does ours: a weekly 9 AM meeting stays at 9 AM
//! across a clock change, which a time kept in UTC gets wrong for half the
//! year. The zones' rules from now on (one POSIX rule each, deduped: 63
//! rules for 526 names, 7.8 KB, tools/zones/gen.py from tzdata 2026e) are
//! all a calendar needs; a file's own VTIMEZONE is read into the same
//! rule, so a zone named only by Outlook ("Pacific Standard Time") is
//! matched to its tzdata name, never guessed.

use crate::date;

#[path = "zone_table.rs"]
mod table;

/// When a clock change happens: the `week`th (5: the last) `weekday`
/// (0 Sunday) of `month`, `at` minutes after that day's midnight in the
/// time before the change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Edge {
    /// 1..=12 (0: no change).
    pub month: u8,
    /// 1..=5, 5 the last.
    pub week: u8,
    /// 0 Sunday ..= 6 Saturday.
    pub weekday: u8,
    /// Minutes after midnight (may be negative or past a day: POSIX).
    pub at: i16,
}

/// A zone's rule: its standard offset, its summer offset (the same when
/// it has none) and when summer starts and ends. Offsets are minutes east
/// of UTC.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rule {
    /// Standard time's offset.
    pub std: i16,
    /// Summer time's offset (`std` when there is none).
    pub dst: i16,
    /// Summer time starts.
    pub on: Edge,
    /// Summer time ends.
    pub off: Edge,
}

const NO_EDGE: Edge = Edge {
    month: 0,
    week: 0,
    weekday: 0,
    at: 0,
};

/// UTC.
pub const UTC: Rule = Rule::fixed(0);

impl Edge {
    /// The day count of the edge in `year`.
    #[must_use]
    pub const fn day(self, year: i32) -> i32 {
        let first = date::days_from(date::Date {
            year,
            month: self.month,
            day: 1,
        });
        // date::weekday: 0 Monday; ours 0 Sunday.
        let wd = ((date::weekday(first) + 1) % 7) as i32;
        let mut d = first + (self.weekday as i32 - wd).rem_euclid(7);
        if self.week >= 5 {
            let last = first + date::days_in_month(year, self.month) as i32 - 1;
            while d + 7 <= last {
                d += 7;
            }
        } else {
            d += 7 * (self.week as i32 - 1);
        }
        d
    }
}

impl Rule {
    /// A zone of one offset all year.
    #[must_use]
    pub const fn fixed(minutes: i16) -> Self {
        Self {
            std: minutes,
            dst: minutes,
            on: NO_EDGE,
            off: NO_EDGE,
        }
    }

    /// Whether it changes its clocks.
    #[must_use]
    pub const fn has_summer(&self) -> bool {
        self.std != self.dst && self.on.month != 0 && self.off.month != 0
    }

    /// Its offset at `utc` (minutes from 1970 in UTC).
    #[must_use]
    #[inline(never)]
    pub fn offset_at(&self, utc: i64) -> i32 {
        let std = i32::from(self.std);
        if !self.has_summer() {
            return std;
        }
        let dst = i32::from(self.dst);
        let year = date::date_of((utc + i64::from(std)).div_euclid(1440) as i32).year;
        let on = i64::from(self.on.day(year)) * 1440 + i64::from(self.on.at) - i64::from(std);
        let off = i64::from(self.off.day(year)) * 1440 + i64::from(self.off.at) - i64::from(dst);
        let summer = if on < off {
            utc >= on && utc < off
        } else {
            // The south: summer runs over the new year.
            utc >= on || utc < off
        };
        if summer { dst } else { std }
    }

    /// The UTC moment of `wall` (local minutes from 1970) in this zone. A
    /// time the spring change skips is read with the offset before it (RFC
    /// 5545 3.3.5: 2:30 AM becomes 3:30 AM); a time the autumn change
    /// repeats is its first (summer) one.
    #[must_use]
    pub fn utc_of(&self, wall: i64) -> i64 {
        let (std, dst) = (i64::from(self.std), i64::from(self.dst));
        let as_dst = wall - dst;
        if self.has_summer() && i64::from(self.offset_at(as_dst)) == dst {
            return as_dst;
        }
        wall - std
    }

    /// `wall` in this zone as the wall time in zone `to`.
    #[must_use]
    pub fn to(&self, wall: i64, to: &Self) -> i64 {
        let utc = self.utc_of(wall);
        utc + i64::from(to.offset_at(utc))
    }
}

/// A zone of the table: where its entry lies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Zone(u16);

/// The zones, in the table's order.
pub fn all() -> impl Iterator<Item = Zone> {
    let mut at = 0usize;
    core::iter::from_fn(move || {
        let n = usize::from(*table::NAMES.get(at + 4)?);
        let z = Zone(at as u16);
        at += 5 + n;
        Some(z)
    })
}

impl Zone {
    fn entry(self) -> &'static [u8] {
        let at = usize::from(self.0);
        let n = usize::from(table::NAMES[at + 4]);
        &table::NAMES[at..at + 5 + n]
    }

    /// Its rule.
    #[must_use]
    pub fn rule(self) -> Rule {
        table::RULES[usize::from(self.entry()[0])]
    }

    /// Its country (ISO 3166, "--" for UTC).
    #[must_use]
    pub fn country(self) -> [u8; 2] {
        let e = self.entry();
        [e[1], e[2]]
    }

    /// Its tzdata name ("America/New_York") into `out`: the length.
    #[must_use]
    #[inline(never)]
    pub fn name(self, out: &mut [u8]) -> usize {
        let e = self.entry();
        let area = table::AREAS[usize::from(e[3])].as_bytes();
        let mut n = 0;
        for &c in area.iter().chain(&e[5..]) {
            if let Some(o) = out.get_mut(n) {
                *o = c;
                n += 1;
            }
        }
        n
    }

    /// Its place as people say it ("New York", "Port-au-Prince"): the
    /// name's last part, underscores as spaces.
    #[must_use]
    #[inline(never)]
    pub fn place(self, out: &mut [u8]) -> usize {
        let e = &self.entry()[5..];
        let last = e
            .iter()
            .rposition(|&c| c == b'/')
            .map_or(e, |k| &e[k + 1..]);
        let mut n = 0;
        for &c in last {
            if let Some(o) = out.get_mut(n) {
                *o = if c == b'_' { b' ' } else { c };
                n += 1;
            }
        }
        n
    }
}

/// The zone of a tzdata name, case ignored ("america/new_york" too).
#[must_use]
#[inline(never)]
pub fn find(name: &[u8]) -> Option<Zone> {
    let mut buf = [0u8; 48];
    all().find(|z| {
        let n = z.name(&mut buf);
        buf[..n].eq_ignore_ascii_case(name)
    })
}

/// The zones a rule or an offset is matched to first: the US's, most
/// people first (US defaults), then the most-populous zone of each other
/// rule people meet in files (so Outlook's "W. Europe Standard Time" is
/// Berlin, not Andorra, which shares its rule); any zone is still found by
/// its own name.
const PREFERRED: [&[u8]; 30] = [
    b"America/New_York",
    b"America/Chicago",
    b"America/Denver",
    b"America/Los_Angeles",
    b"America/Phoenix",
    b"America/Anchorage",
    b"Pacific/Honolulu",
    b"America/Puerto_Rico",
    b"America/Halifax",
    b"America/St_Johns",
    b"America/Mexico_City",
    b"America/Sao_Paulo",
    b"America/Bogota",
    b"Europe/London",
    b"Europe/Berlin",
    b"Europe/Athens",
    b"Europe/Moscow",
    b"Europe/Istanbul",
    b"Africa/Lagos",
    b"Africa/Johannesburg",
    b"Africa/Cairo",
    b"Asia/Dubai",
    b"Asia/Karachi",
    b"Asia/Kolkata",
    b"Asia/Dhaka",
    b"Asia/Bangkok",
    b"Asia/Shanghai",
    b"Asia/Tokyo",
    b"Australia/Sydney",
    b"Pacific/Auckland",
];

/// The zone a rule is (a file's VTIMEZONE matched to its tzdata name): a
/// [`PREFERRED`] zone first, then the table's order.
#[must_use]
#[inline(never)]
pub fn of_rule(r: &Rule) -> Option<Zone> {
    PREFERRED
        .iter()
        .filter_map(|n| find(n))
        .chain(all())
        .find(|z| z.rule() == *r)
}

/// The device's likely zone from what the dock's PC said (its offset now,
/// at `utc` minutes from 1970): a US zone of that offset first, else the
/// first of the table with it now; a fixed zone of that offset if none.
#[must_use]
#[inline(never)]
pub fn guess(utc: i64, offset: i32) -> Option<Zone> {
    PREFERRED
        .iter()
        .filter_map(|n| find(n))
        .chain(all())
        .find(|z| z.rule().offset_at(utc) == offset)
}

#[cfg(test)]
#[path = "zone_tests.rs"]
mod tests;
