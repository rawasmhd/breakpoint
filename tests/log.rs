//! Scenario tests for the event log and the Today count: a real scheduler on a fake clock and
//! presence feed, with its log lines formatted as the platform writes them.

use std::time::Duration;

use breakpoint::core::config::Nudges;
use breakpoint::core::log::{self, Today};
use breakpoint::core::presence::{AwayReason, POLL_EVERY, Presence};
use breakpoint::core::scheduler::{Effect, Event, Nudge, Scheduler};
use breakpoint::core::time::LocalTime;

const MIN: Duration = Duration::from_secs(60);
const SEC: Duration = Duration::from_secs(1);

/// The example of SPEC §10.2, byte for byte.
const EXAMPLE: &str = r#"{"t":"2026-11-02T09:00:00+03:00","ev":"fired","nudge":"pause","occ":42}
{"t":"2026-11-02T09:00:00+03:00","ev":"fired","nudge":"stand","occ":43,"into":42}
{"t":"2026-11-02T09:00:00+03:00","ev":"shown","nudge":"water","occ":44}
{"t":"2026-11-02T09:00:40+03:00","ev":"snoozed","nudge":"pause","occ":42}
{"t":"2026-11-02T09:00:40+03:00","ev":"snoozed","nudge":"stand","occ":43}
{"t":"2026-11-02T09:07:02+03:00","ev":"satisfied","nudge":"pause","occ":42,"by":"lock"}
{"t":"2026-11-02T09:07:02+03:00","ev":"satisfied","nudge":"stand","occ":43,"by":"lock"}
{"t":"2026-11-02T17:00:00+03:00","ev":"skipped","nudge":"pause","occ":58,"by":"quiet"}
"#;

fn at(s: &str) -> LocalTime {
    LocalTime::parse_rfc3339(s).unwrap()
}

/// A day at the desk on the Balanced profile: the scheduler, polled like the platform polls it,
/// and every event it logs with the moment on its timeline.
struct Day {
    sched: Scheduler,
    now: Duration,
    last_input: Duration,
    events: Vec<(Duration, Event)>,
}

impl Day {
    fn new(last_occ: u64) -> Self {
        let mut sched = Scheduler::new(Nudges::default(), MIN);
        sched.continue_from(last_occ);
        let mut day = Day {
            sched,
            now: Duration::ZERO,
            last_input: Duration::ZERO,
            events: Vec::new(),
        };
        day.poll(Presence::Active);
        day
    }

    fn poll(&mut self, presence: Presence) {
        let out = self.sched.poll(self.now, presence);
        self.record(out);
    }

    fn record(&mut self, out: Vec<Effect>) {
        for effect in out {
            if let Effect::Log(event) = effect {
                self.events.push((self.now, event));
            }
        }
    }

    /// The user types for `d`.
    fn work(&mut self, d: Duration) {
        let end = self.now + d;
        while self.now < end {
            self.now = (self.now + POLL_EVERY).min(end);
            self.last_input = self.now;
            self.poll(Presence::Active);
        }
    }

    /// The user types until something fires; returns the occurrences that fired.
    fn work_until_fired(&mut self) -> Vec<(Nudge, u64)> {
        let from = self.events.len();
        loop {
            self.work(POLL_EVERY);
            let fired: Vec<_> = self.events[from..]
                .iter()
                .filter_map(|(_, e)| match *e {
                    Event::Fired { nudge, occ, .. } => Some((nudge, occ)),
                    _ => None,
                })
                .collect();
            if !fired.is_empty() {
                return fired;
            }
        }
    }

    /// The user walks away from the keyboard for `d`, then touches it again.
    fn walk_away(&mut self, d: Duration) {
        let end = self.now + d;
        while self.now < end {
            self.now = (self.now + POLL_EVERY).min(end);
            self.poll(Presence::from_last_input(self.now, self.last_input));
        }
        self.work(POLL_EVERY);
    }

    /// The user locks the screen now, for `d`.
    fn lock(&mut self, d: Duration) {
        let away = Presence::Away {
            since: self.last_input,
            reason: AwayReason::Lock,
        };
        let end = self.now + d;
        self.poll(away);
        while self.now < end {
            self.now = (self.now + POLL_EVERY).min(end);
            self.poll(away);
        }
        self.work(POLL_EVERY);
    }

    fn snooze(&mut self) {
        let out = self.sched.snooze(self.now);
        self.record(out);
    }

    /// The log, with the timeline's `nine` read as 09:00:00 on 2 November 2026.
    fn log(&self, nine: Duration) -> String {
        let nine_wall = at("2026-11-02T09:00:00+03:00");
        self.events
            .iter()
            .map(|&(when, ref event)| {
                let secs = when.as_secs() as i64 - nine.as_secs() as i64;
                let t = LocalTime {
                    unix: nine_wall.unix + secs,
                    ..nine_wall
                };
                log::line(t, event)
            })
            .collect()
    }
}

#[test]
fn a_day_on_balanced_writes_the_example_lines_of_spec_10_2() {
    // The newest file ends at occ 40.
    let mut day = Day::new(40);

    // 08:30: a Stand/Move, taken by walking away.
    assert_eq!(day.work_until_fired(), [(Nudge::Stand, 41)]);
    day.walk_away(3 * MIN);

    // 09:00: Pause and Stand/Move together, Water riding along.
    assert_eq!(
        day.work_until_fired(),
        [(Nudge::Pause, 42), (Nudge::Stand, 43)]
    );
    let nine = day.now;
    day.work(40 * SEC);
    day.snooze();
    // 09:07:02: the user locks the screen; it satisfies both at once.
    day.work(7 * MIN + 2 * SEC - 40 * SEC);
    day.lock(MIN);

    // Three more hours of taking every nudge: a Stand/Move alone at half past, then the
    // combined nudge on the hour, so each hour uses four ids.
    for hour in 0..3 {
        let first = 45 + 4 * hour;
        assert_eq!(day.work_until_fired(), [(Nudge::Stand, first)]);
        day.walk_away(3 * MIN);
        assert_eq!(
            day.work_until_fired(),
            [(Nudge::Pause, first + 1), (Nudge::Stand, first + 2)]
        );
        day.lock(MIN);
    }
    // A long lunch, sized so that the afternoon's Pause fires a quarter of an hour before 17:00.
    let back = day.now;
    day.walk_away(nine + 7 * 60 * MIN + 45 * MIN - back - 63 * MIN);
    assert_eq!(day.work_until_fired(), [(Nudge::Stand, 57)]);
    day.walk_away(3 * MIN);
    assert_eq!(
        day.work_until_fired(),
        [(Nudge::Pause, 58), (Nudge::Stand, 59)]
    );
    let seventeen = nine + 8 * 60 * MIN;
    assert!(day.now > seventeen - 30 * MIN && day.now < seventeen);

    // The user ignores it and works on; at 17:00 quiet time starts.
    day.work(seventeen - POLL_EVERY - day.now);
    day.now = seventeen;
    let out = day.sched.set_quiet(day.now, true);
    day.record(out);

    let log = day.log(nine);
    let lines: Vec<&str> = log.split_inclusive('\n').collect();
    let example: Vec<&str> = EXAMPLE.split_inclusive('\n').collect();
    let first = lines
        .iter()
        .position(|l| l.contains(r#""occ":42"#))
        .unwrap();
    assert_eq!(lines[first..first + 7], example[..7]);
    let last = lines.iter().position(|l| l.contains("17:00:00")).unwrap();
    assert_eq!(lines[last], example[7]);
    assert_eq!(
        lines[last + 1],
        "{\"t\":\"2026-11-02T17:00:00+03:00\",\"ev\":\"skipped\",\"nudge\":\"stand\",\"occ\":59,\"by\":\"quiet\"}\n"
    );

    // Today, live and rebuilt from the file: 41, 42+43, 45, 46+47, 49, 50+51, 53, 54+55 and 57
    // taken; 58 and 59 skipped. Water is not counted.
    let evening = at("2026-11-02T17:00:05+03:00");
    let mut live = Today::default();
    for line in &lines {
        let (t, event) = log::parse(line.trim_end()).unwrap();
        live.record(t, &event);
    }
    assert_eq!(live.text(evening), "Today: 13 taken, 2 skipped");
    assert_eq!(
        Today::rebuild(&log, evening).text(evening),
        "Today: 13 taken, 2 skipped"
    );
    assert_eq!(log::last_occ(&log), Some(60));
}

#[test]
fn today_rebuild_ignores_an_occurrence_a_crash_left_open() {
    // 42 and 43 were taken; then 50 and 51 fired and breakpoint crashed with them pending.
    let file = format!(
        "{}{}",
        &EXAMPLE[..EXAMPLE.find(r#"{"t":"2026-11-02T17"#).unwrap()],
        r#"{"t":"2026-11-02T15:00:00+03:00","ev":"fired","nudge":"pause","occ":50}
{"t":"2026-11-02T15:00:00+03:00","ev":"fired","nudge":"stand","occ":51,"into":50}
"#
    );
    let now = at("2026-11-02T15:30:00+03:00");
    let mut today = Today::rebuild(&file, now);
    assert_eq!(today.text(now), "Today: 2 taken, 0 skipped");

    // After the restart the ids go on above 51; their ends count, and nothing of 50 or 51 ever
    // does.
    assert_eq!(log::last_occ(&file), Some(51));
    today.record(
        now,
        &Event::Skipped {
            nudge: Nudge::Pause,
            occ: 50,
            by: breakpoint::core::scheduler::SkipCause::Exit,
        },
    );
    today.record(
        now,
        &Event::Fired {
            nudge: Nudge::Stand,
            occ: 52,
            into: None,
        },
    );
    today.record(
        now,
        &Event::Skipped {
            nudge: Nudge::Stand,
            occ: 52,
            by: breakpoint::core::scheduler::SkipCause::User,
        },
    );
    assert_eq!(today.text(now), "Today: 2 taken, 1 skipped");
}

#[test]
fn today_counts_by_the_day_an_occurrence_fired_and_resets_at_midnight() {
    let file = r#"{"t":"2026-11-01T16:00:00+03:00","ev":"fired","nudge":"stand","occ":1}
{"t":"2026-11-01T16:03:00+03:00","ev":"satisfied","nudge":"stand","occ":1,"by":"away"}
{"t":"2026-11-02T23:58:00+03:00","ev":"fired","nudge":"pause","occ":2}
{"t":"2026-11-02T23:58:30+03:00","ev":"snoozed","nudge":"pause","occ":2}
{"t":"2026-11-03T00:04:00+03:00","ev":"skipped","nudge":"pause","occ":2,"by":"user"}
{"t":"2026-11-03T00:05:00+03:00","ev":"fired","nudge":"stand","occ":3}
{"t":"2026-11-03T00:05:00+03:00","ev":"shown","nudge":"water","occ":4}
"#;
    // A restart at 23:59 on the 2nd, the file as it was then: Pause 2 is still open.
    let before = at("2026-11-02T23:59:00+03:00");
    let then = &file[..file.find(r#"{"t":"2026-11-03"#).unwrap()];
    assert_eq!(
        Today::rebuild(then, before).text(before),
        "Today: 0 taken, 0 skipped"
    );

    // Live: Pause 2 fired on the 2nd and ended after midnight, so it counts for the 2nd.
    let mut today = Today::default();
    for (t, event) in file.lines().filter_map(log::parse) {
        today.record(t, &event);
    }
    let the_2nd = at("2026-11-02T23:59:59+03:00");
    let the_3rd = at("2026-11-03T00:06:00+03:00");
    assert_eq!(today.counts(the_2nd), (0, 1));
    assert_eq!(today.text(the_3rd), "Today: 0 taken, 0 skipped");

    // Stand/Move 3 fired on the 3rd: taking it starts the 3rd's count.
    let t = at("2026-11-03T00:08:00+03:00");
    today.record(
        t,
        &Event::Satisfied {
            nudge: Nudge::Stand,
            occ: 3,
            by: breakpoint::core::scheduler::SatisfyCause::Lock,
        },
    );
    assert_eq!(today.text(t), "Today: 1 taken, 0 skipped");
    assert_eq!(today.counts(the_2nd), (0, 0));

    // Rebuilt on the 3rd from the same file: Pause 2 belongs to the 2nd and Stand/Move 3 is open.
    assert_eq!(Today::rebuild(file, the_3rd).counts(the_3rd), (0, 0));
}

#[test]
fn today_rebuild_skips_malformed_lines() {
    let file = r#"{"t":"2026-11-02T09:00:00+03:00","ev":"fired","nudge":"stand","occ":7}
{"t":"2026-11-02T09:00:00+03:00","ev":"fired","nudge":"pause","occ":8}
{"t":"2026-11-02T09:03:00+03:00","ev":"satisfied","nudge":"stand","occ":7,"by":"away"
garbage
{"t":"2026-11-02T09:09:00+03:00","ev":"satisfied","nudge":"pause","occ":8,"by":"away","note":"later field"}
{"t":"2026-11-02T09:30:00+03:00","ev":"fired","nudge":"stand","occ":9}

{"t":"2026-11-02T09:31:00+03:00","ev":"skipped","nudge":"stand","occ":9,"by":"user"}"#;
    let now = at("2026-11-02T10:00:00+03:00");
    // Stand/Move 7's end is cut off, so 7 stays open and does not count.
    assert_eq!(
        Today::rebuild(file, now).text(now),
        "Today: 1 taken, 1 skipped"
    );
    assert_eq!(log::last_occ(file), Some(9));
}

#[test]
fn the_occ_continues_from_the_newest_file() {
    let october = r#"{"t":"2026-10-31T16:00:00+03:00","ev":"fired","nudge":"stand","occ":120}"#;
    let november = "{\"t\":\"2026-11-01T09:00:00+03:00\",\"ev\":\"fired\",\"nudge\":\"pause\",\"occ\":121}\n\
                    {\"t\":\"2026-11-01T09:00:00+03:00\",\"ev\":\"shown\",\"nudge\":\"water\",\"occ\":123}\n\
                    {\"t\":\"2026-11-01T09:00:00+03:00\",\"ev\":\"fired\",\"nudge\":\"stand\",\"occ\":122,\"into\":121}\n\
                    {broken";
    let files = [("2026-10.jsonl", october), ("2026-11.jsonl", november)];
    let newest = log::newest_file(files.iter().map(|(name, _)| *name)).unwrap();
    let text = files.iter().find(|(name, _)| *name == newest).unwrap().1;
    let last = log::last_occ(text).unwrap();
    assert_eq!(last, 123);

    let mut day = Day::new(last);
    assert_eq!(day.work_until_fired(), [(Nudge::Stand, 124)]);
}

#[test]
fn with_no_log_yet_the_first_occ_is_1() {
    assert_eq!(log::newest_file([]), None);
    let mut day = Day::new(0);
    assert_eq!(day.work_until_fired(), [(Nudge::Stand, 1)]);
}
