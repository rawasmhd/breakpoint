//! Event records, JSONL formatting and rebuilding the Today count (SPEC §10.2, §10.3).
//!
//! Pure: the platform supplies the wall-clock time of each event and the text of the log files,
//! and writes the lines this module formats.

use std::collections::HashMap;

use serde::Deserialize;

use super::scheduler::{Event, Nudge, SatisfyCause, SkipCause};
use super::time::LocalTime;

/// One log line for `event` at `t`, with its trailing `\n`. Fields come in the order of SPEC
/// §10.2 (`t`, `ev`, `nudge`, `occ`, `by`, `into`); absent ones are left out, never `null`.
pub fn line(t: LocalTime, event: &Event) -> String {
    let (ev, nudge, occ) = match *event {
        Event::Fired { nudge, occ, .. } => ("fired", nudge_key(nudge), occ),
        Event::Snoozed { nudge, occ } => ("snoozed", nudge_key(nudge), occ),
        Event::Satisfied { nudge, occ, .. } => ("satisfied", nudge_key(nudge), occ),
        Event::Skipped { nudge, occ, .. } => ("skipped", nudge_key(nudge), occ),
        Event::Shown { occ } => ("shown", "water", occ),
    };
    let mut line = format!(r#"{{"t":"{t}","ev":"{ev}","nudge":"{nudge}","occ":{occ}"#);
    match *event {
        Event::Satisfied { by, .. } => line += &format!(r#","by":"{}""#, satisfy_key(by)),
        Event::Skipped { by, .. } => line += &format!(r#","by":"{}""#, skip_key(by)),
        Event::Fired {
            into: Some(into), ..
        } => line += &format!(r#","into":{into}"#),
        _ => {}
    }
    line += "}\n";
    line
}

/// The name of the month's log file that an event at `t` goes in, e.g. `2026-11.jsonl`.
pub fn file_name(t: LocalTime) -> String {
    let c = t.civil();
    format!("{:04}-{:02}.jsonl", c.year, c.month)
}

/// The newest month among log file names, by name; other names are ignored.
pub fn newest_file<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    names.into_iter().filter(|n| is_file_name(n)).max()
}

/// The highest `occ` in a log file's text, skipping lines that are not JSON objects with an
/// `occ`. At start-up the scheduler continues from the newest file's (SPEC §10.2).
pub fn last_occ(text: &str) -> Option<u64> {
    #[derive(Deserialize)]
    struct Occ {
        occ: u64,
    }
    text.lines()
        .filter_map(|l| serde_json::from_str::<Occ>(l).ok())
        .map(|l| l.occ)
        .max()
}

/// Reads one log line back, or `None` if it is malformed. Unknown fields are ignored.
pub fn parse(line: &str) -> Option<(LocalTime, Event)> {
    #[derive(Deserialize)]
    struct Line {
        t: String,
        ev: String,
        nudge: String,
        occ: u64,
        by: Option<String>,
        into: Option<u64>,
    }
    let l: Line = serde_json::from_str(line).ok()?;
    let t = LocalTime::parse_rfc3339(&l.t)?;
    let occ = l.occ;
    let by = l.by.as_deref();
    let event = match (l.ev.as_str(), l.nudge.as_str()) {
        ("shown", "water") => Event::Shown { occ },
        (ev, nudge) => {
            let nudge = match nudge {
                "stand" => Nudge::Stand,
                "pause" => Nudge::Pause,
                _ => return None,
            };
            match ev {
                "fired" => Event::Fired {
                    nudge,
                    occ,
                    into: l.into,
                },
                "snoozed" => Event::Snoozed { nudge, occ },
                "satisfied" => Event::Satisfied {
                    nudge,
                    occ,
                    by: SatisfyCause::ALL
                        .into_iter()
                        .find(|&c| Some(satisfy_key(c)) == by)?,
                },
                "skipped" => Event::Skipped {
                    nudge,
                    occ,
                    by: SkipCause::ALL
                        .into_iter()
                        .find(|&c| Some(skip_key(c)) == by)?,
                },
                _ => return None,
            }
        }
    };
    Some((t, event))
}

/// A local calendar date, which is what decides the day an occurrence counts toward.
type Date = (i32, u8, u8);

fn date(t: LocalTime) -> Date {
    let c = t.civil();
    (c.year, c.month, c.day)
}

/// The Today count (SPEC §10.3): Stand/Move and Pause occurrences fired today that were taken
/// (satisfied) or skipped. Water does not count, snoozes change nothing, and each part of a
/// combined nudge counts on its own. It starts again at local midnight.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Today {
    /// The day the counts are for.
    day: Option<Date>,
    taken: u32,
    skipped: u32,
    /// The day each occurrence still open fired on.
    open: HashMap<u64, Date>,
}

impl Today {
    /// The count for the day of `now`, rebuilt from the current month's log file. Only end
    /// events whose `fired` line is dated that day count; an occurrence a crash left open is
    /// ignored, and so are malformed lines.
    pub fn rebuild(text: &str, now: LocalTime) -> Self {
        let mut today = Today {
            day: Some(date(now)),
            ..Today::default()
        };
        for (t, event) in text.lines().filter_map(parse) {
            today.record(t, &event);
        }
        // What is still open was left by a crash; the occs go on from above it.
        today.open.clear();
        today
    }

    /// Counts `event`, logged at `t`.
    pub fn record(&mut self, t: LocalTime, event: &Event) {
        let (occ, taken) = match *event {
            Event::Fired { occ, .. } => {
                self.open.insert(occ, date(t));
                return;
            }
            Event::Satisfied { occ, .. } => (occ, true),
            Event::Skipped { occ, .. } => (occ, false),
            Event::Snoozed { .. } | Event::Shown { .. } => return,
        };
        let Some(fired) = self.open.remove(&occ) else {
            return;
        };
        // A later day starts the count again; an earlier one is past.
        if self.day.is_none_or(|day| fired > day) {
            *self = Today {
                day: Some(fired),
                open: std::mem::take(&mut self.open),
                ..Today::default()
            };
        }
        if self.day == Some(fired) {
            if taken {
                self.taken += 1;
            } else {
                self.skipped += 1;
            }
        }
    }

    /// Taken and skipped for the day of `now`: zero once midnight has passed.
    pub fn counts(&self, now: LocalTime) -> (u32, u32) {
        if self.day == Some(date(now)) {
            (self.taken, self.skipped)
        } else {
            (0, 0)
        }
    }

    /// The tray line, exactly `Today: 7 taken, 2 skipped`, with no plural changes.
    pub fn text(&self, now: LocalTime) -> String {
        let (taken, skipped) = self.counts(now);
        format!("Today: {taken} taken, {skipped} skipped")
    }
}

fn is_file_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 13
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && &name[7..] == ".jsonl"
}

fn nudge_key(nudge: Nudge) -> &'static str {
    match nudge {
        Nudge::Stand => "stand",
        Nudge::Pause => "pause",
    }
}

fn satisfy_key(by: SatisfyCause) -> &'static str {
    match by {
        SatisfyCause::Away => "away",
        SatisfyCause::Lock => "lock",
        SatisfyCause::Suspend => "suspend",
        SatisfyCause::DisplayOff => "display_off",
    }
}

fn skip_key(by: SkipCause) -> &'static str {
    match by {
        SkipCause::User => "user",
        SkipCause::Next => "next",
        SkipCause::Quiet => "quiet",
        SkipCause::Settings => "settings",
        SkipCause::Exit => "exit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> LocalTime {
        LocalTime::parse_rfc3339(s).unwrap()
    }

    #[test]
    fn every_event_round_trips_through_its_line() {
        let t = at("2026-11-02T09:00:00+03:00");
        let mut events = vec![
            Event::Fired {
                nudge: Nudge::Pause,
                occ: 1,
                into: None,
            },
            Event::Fired {
                nudge: Nudge::Stand,
                occ: 2,
                into: Some(1),
            },
            Event::Snoozed {
                nudge: Nudge::Stand,
                occ: 2,
            },
            Event::Shown { occ: 3 },
        ];
        events.extend(SatisfyCause::ALL.map(|by| Event::Satisfied {
            nudge: Nudge::Pause,
            occ: 4,
            by,
        }));
        events.extend(SkipCause::ALL.map(|by| Event::Skipped {
            nudge: Nudge::Stand,
            occ: 5,
            by,
        }));
        for event in events {
            let line = line(t, &event);
            assert!(line.ends_with("}\n") && !line.contains("null"), "{line}");
            assert_eq!(parse(line.trim_end()), Some((t, event)), "{line}");
        }
    }

    #[test]
    fn the_settings_cause_is_written_for_issue_47() {
        let event = Event::Skipped {
            nudge: Nudge::Pause,
            occ: 9,
            by: SkipCause::Settings,
        };
        assert_eq!(
            line(at("2026-11-02T10:00:00+03:00"), &event),
            "{\"t\":\"2026-11-02T10:00:00+03:00\",\"ev\":\"skipped\",\"nudge\":\"pause\",\"occ\":9,\"by\":\"settings\"}\n"
        );
    }

    #[test]
    fn files_are_named_by_the_local_month() {
        assert_eq!(file_name(at("2026-11-30T23:59:59+03:00")), "2026-11.jsonl");
        // 21:30 UTC on 31 December is already January in +03:00.
        assert_eq!(file_name(at("2027-01-01T00:30:00+03:00")), "2027-01.jsonl");
    }

    #[test]
    fn the_newest_file_is_picked_by_name_ignoring_others() {
        let names = [
            "2026-10.jsonl",
            "2026-12.jsonl",
            "notes.txt",
            "2027-01.jsonl.bak",
            "2026-11.jsonl",
        ];
        assert_eq!(newest_file(names), Some("2026-12.jsonl"));
        assert_eq!(newest_file(["x.jsonl"]), None);
    }

    #[test]
    fn last_occ_is_the_highest_and_skips_malformed_lines() {
        let text = "{\"occ\":7}\nnot json\n{\"t\":\"x\",\"occ\":12,\"ev\":\"fired\"}\n{\"occ\":-1}\n{\"occ\":9}";
        assert_eq!(last_occ(text), Some(12));
        assert_eq!(last_occ(""), None);
    }

    #[test]
    fn unknown_fields_are_ignored_and_bad_values_make_a_line_malformed() {
        let ok = r#"{"t":"2026-11-02T09:00:00+03:00","ev":"fired","nudge":"pause","occ":1,"later":true}"#;
        assert!(parse(ok).is_some());
        for bad in [
            r#"{"t":"2026-11-02T09:00:00+03:00","ev":"fired","nudge":"walk","occ":1}"#,
            r#"{"t":"2026-11-02T09:00:00+03:00","ev":"skipped","nudge":"pause","occ":1}"#,
            r#"{"t":"2026-11-02T09:00:00+03:00","ev":"skipped","nudge":"pause","occ":1,"by":"bored"}"#,
            r#"{"t":"yesterday","ev":"fired","nudge":"pause","occ":1}"#,
            r#"{"t":"2026-11-02T09:00:00+03:00","ev":"fired","nudge":"pause"}"#,
            r#"{"t":"2026-11-02T09:00:00+03:00","ev":"fired","nudge":"pa"#,
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
