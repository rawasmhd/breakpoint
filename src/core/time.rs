//! Wall-clock time for the core without a date-time crate: a moment with its UTC offset, the
//! weekday and time of day it falls on, and RFC 3339 for `config.json` and the event log.
//!
//! The platform supplies the current [`LocalTime`]; the core only does calendar arithmetic.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Weekday {
    Sun,
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
}

impl Weekday {
    /// Sunday first, as the work week is shown (SPEC §9.2).
    pub const ALL: [Weekday; 7] = [
        Weekday::Sun,
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
    ];

    /// The day's name in `config.json`: `sun` to `sat`.
    pub fn key(self) -> &'static str {
        match self {
            Weekday::Sun => "sun",
            Weekday::Mon => "mon",
            Weekday::Tue => "tue",
            Weekday::Wed => "wed",
            Weekday::Thu => "thu",
            Weekday::Fri => "fri",
            Weekday::Sat => "sat",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|day| day.key() == key)
    }
}

/// A time of day in whole minutes, 00:00 to 23:59.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeOfDay(u16);

impl TimeOfDay {
    pub const fn new(hour: u8, minute: u8) -> Option<Self> {
        if hour < 24 && minute < 60 {
            Some(Self(hour as u16 * 60 + minute as u16))
        } else {
            None
        }
    }

    pub fn hour(self) -> u8 {
        (self.0 / 60) as u8
    }

    pub fn minute(self) -> u8 {
        (self.0 % 60) as u8
    }

    /// Minutes since midnight.
    pub fn minutes(self) -> u32 {
        self.0.into()
    }

    /// Parses exactly `HH:mm`, 24-hour.
    pub fn parse(s: &str) -> Option<Self> {
        let (h, m) = s.split_once(':')?;
        Self::new(two_digits(h)?, two_digits(m)?)
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour(), self.minute())
    }
}

/// A calendar date and time of day to the second, with no zone attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Civil {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Civil {
    fn is_valid(&self) -> bool {
        (1..=12).contains(&self.month)
            && (1..=days_in_month(self.year, self.month)).contains(&self.day)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
    }

    /// Seconds since 1970-01-01T00:00:00 of the same zone.
    fn seconds(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day) * 86_400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
    }

    fn from_seconds(secs: i64) -> Self {
        let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        let (year, month, day) = civil_from_days(days);
        Civil {
            year,
            month,
            day,
            hour: (rem / 3600) as u8,
            minute: (rem / 60 % 60) as u8,
            second: (rem % 60) as u8,
        }
    }
}

/// A moment and the UTC offset in force there, to the second.
///
/// Two values compare equal only with the same offset too; compare moments with
/// [`LocalTime::unix`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalTime {
    /// Seconds since the Unix epoch, UTC.
    pub unix: i64,
    /// Local time minus UTC, in minutes (+180 for +03:00).
    pub offset_minutes: i32,
}

impl LocalTime {
    /// The same moment read on a UTC clock and on the local clock.
    pub fn from_utc_and_local(utc: Civil, local: Civil) -> Self {
        let unix = utc.seconds();
        Self {
            unix,
            offset_minutes: ((local.seconds() - unix) / 60) as i32,
        }
    }

    /// The date and time on the local clock.
    pub fn civil(self) -> Civil {
        Civil::from_seconds(self.unix + i64::from(self.offset_minutes) * 60)
    }

    pub fn weekday(self) -> Weekday {
        let days = (self.unix + i64::from(self.offset_minutes) * 60).div_euclid(86_400);
        // 1970-01-01 was a Thursday.
        Weekday::ALL[(days + 4).rem_euclid(7) as usize]
    }

    pub fn time_of_day(self) -> TimeOfDay {
        let c = self.civil();
        TimeOfDay(c.hour as u16 * 60 + c.minute as u16)
    }

    /// Parses an RFC 3339 date-time such as `2026-11-02T09:00:00+03:00`. Fractions of a second
    /// are dropped; `Z` is offset zero.
    pub fn parse_rfc3339(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() < 20
            || b[4] != b'-'
            || b[7] != b'-'
            || !matches!(b[10], b'T' | b't')
            || b[13] != b':'
            || b[16] != b':'
        {
            return None;
        }
        let local = Civil {
            year: s.get(0..4)?.parse().ok().filter(|_| digits(&s[0..4]))?,
            month: two_digits(s.get(5..7)?)?,
            day: two_digits(s.get(8..10)?)?,
            hour: two_digits(s.get(11..13)?)?,
            minute: two_digits(s.get(14..16)?)?,
            second: two_digits(s.get(17..19)?)?,
        };
        if !local.is_valid() {
            return None;
        }
        let mut rest = &s[19..];
        if let Some(frac) = rest.strip_prefix('.') {
            let len = frac.bytes().take_while(u8::is_ascii_digit).count();
            if len == 0 {
                return None;
            }
            rest = &frac[len..];
        }
        let offset_minutes = match rest {
            "Z" | "z" => 0,
            _ => {
                let sign = match rest.as_bytes().first()? {
                    b'+' => 1,
                    b'-' => -1,
                    _ => return None,
                };
                let offset = TimeOfDay::parse(rest.get(1..)?)?;
                sign * offset.minutes() as i32
            }
        };
        Some(Self {
            unix: local.seconds() - i64::from(offset_minutes) * 60,
            offset_minutes,
        })
    }
}

/// RFC 3339 in whole seconds with a numeric offset, e.g. `2026-11-02T09:00:00+03:00`.
impl fmt::Display for LocalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.civil();
        let sign = if self.offset_minutes < 0 { '-' } else { '+' };
        let offset = self.offset_minutes.unsigned_abs();
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
            c.year,
            c.month,
            c.day,
            c.hour,
            c.minute,
            c.second,
            offset / 60,
            offset % 60
        )
    }
}

fn digits(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit())
}

fn two_digits(s: &str) -> Option<u8> {
    if s.len() == 2 && digits(s) {
        s.parse().ok()
    } else {
        None
    }
}

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar (Howard Hinnant's algorithm).
fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trips_with_its_offset() {
        let t = LocalTime::parse_rfc3339("2026-11-02T09:00:00+03:00").unwrap();
        assert_eq!(t.unix, 1_793_599_200);
        assert_eq!(t.offset_minutes, 180);
        assert_eq!(t.to_string(), "2026-11-02T09:00:00+03:00");

        let t = LocalTime::parse_rfc3339("1969-12-31T23:30:15-05:30").unwrap();
        assert_eq!(t.to_string(), "1969-12-31T23:30:15-05:30");
    }

    #[test]
    fn rfc3339_accepts_z_and_fractions() {
        let t = LocalTime::parse_rfc3339("2026-11-02T06:00:00.250Z").unwrap();
        assert_eq!(t.unix, 1_793_599_200);
        assert_eq!(t.to_string(), "2026-11-02T06:00:00+00:00");
    }

    #[test]
    fn rfc3339_rejects_malformed_input() {
        for s in [
            "",
            "2026-11-02",
            "2026-11-02T09:00:00",
            "2026-11-02 09:00:00+03:00",
            "2026-13-02T09:00:00+03:00",
            "2026-02-29T09:00:00+03:00",
            "2026-11-02T24:00:00+03:00",
            "2026-11-02T09:00:00+3:00",
            "2026-11-02T09:00:00.+03:00",
            "+026-11-02T09:00:00+03:00",
            "2026-11-02T09:00:00+03:00x",
        ] {
            assert_eq!(LocalTime::parse_rfc3339(s), None, "{s}");
        }
        assert!(LocalTime::parse_rfc3339("2028-02-29T09:00:00+03:00").is_some());
    }

    #[test]
    fn weekday_and_time_of_day_are_local() {
        // 2026-11-01 is a Sunday; 23:30 UTC on the Saturday is already Sunday 02:30 at +03:00.
        let t = LocalTime::parse_rfc3339("2026-10-31T23:30:00Z").unwrap();
        assert_eq!(t.weekday(), Weekday::Sat);
        let t = LocalTime {
            offset_minutes: 180,
            ..t
        };
        assert_eq!(t.weekday(), Weekday::Sun);
        assert_eq!(t.time_of_day(), TimeOfDay::new(2, 30).unwrap());
    }

    #[test]
    fn utc_and_local_give_the_offset() {
        let utc = Civil {
            year: 2026,
            month: 12,
            day: 31,
            hour: 22,
            minute: 15,
            second: 5,
        };
        let local = Civil {
            year: 2027,
            month: 1,
            day: 1,
            hour: 1,
            minute: 15,
            second: 5,
        };
        let t = LocalTime::from_utc_and_local(utc, local);
        assert_eq!(t.offset_minutes, 180);
        assert_eq!(t.civil(), local);
        assert_eq!(t.to_string(), "2027-01-01T01:15:05+03:00");
    }

    #[test]
    fn time_of_day_parses_only_hh_mm() {
        assert_eq!(TimeOfDay::parse("08:00"), TimeOfDay::new(8, 0));
        assert_eq!(TimeOfDay::parse("23:59").unwrap().to_string(), "23:59");
        for s in ["8:00", "24:00", "12:60", "12:5", "12-00", "", "1200"] {
            assert_eq!(TimeOfDay::parse(s), None, "{s}");
        }
    }
}
