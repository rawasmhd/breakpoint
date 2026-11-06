//! Scenario tests for the quiet rules, Off and Overtime driving the scheduler, with a fake wall
//! clock and presence feed. Default config: Sunday to Thursday, 08:00 to 17:00. 2026-11-03 is a
//! Tuesday and 2026-11-05 a Thursday.

use std::time::Duration;

use breakpoint::core::config::{self, Config, Nudges};
use breakpoint::core::presence::{POLL_EVERY, Presence};
use breakpoint::core::quiet::{Press, Quiet, Switch};
use breakpoint::core::scheduler::{Effect, Event, Nudge, Scheduler, SkipCause};
use breakpoint::core::time::LocalTime;

const MIN: Duration = Duration::from_secs(60);

fn at(s: &str) -> LocalTime {
    LocalTime::parse_rfc3339(&format!("{s}+03:00")).unwrap()
}

/// What the app does on each poll: sync the quiet rules, then poll the scheduler.
struct Sim {
    sched: Scheduler,
    quiet: Quiet,
    /// The wall clock at `Duration::ZERO` on the scheduler's timeline.
    start: LocalTime,
    now: Duration,
    last_input: Duration,
    effects: Vec<(LocalTime, Effect)>,
}

impl Sim {
    fn new(start: &str) -> Self {
        Self::with(start, Nudges::default())
    }

    fn with(start: &str, nudges: Nudges) -> Self {
        let config = Config {
            nudges,
            ..Config::default()
        };
        Self::from_config(at(start), &config)
    }

    fn from_config(start: LocalTime, config: &Config) -> Self {
        let mut sim = Sim {
            sched: Scheduler::new(config.nudges, MIN),
            quiet: Quiet::new(config),
            start,
            now: Duration::ZERO,
            last_input: Duration::ZERO,
            effects: Vec::new(),
        };
        sim.poll();
        sim
    }

    fn wall(&self) -> LocalTime {
        LocalTime {
            unix: self.start.unix + self.now.as_secs() as i64,
            ..self.start
        }
    }

    /// The user keeps typing until `until` on the wall clock.
    fn work_until(&mut self, until: &str) {
        self.run_until(until, true);
    }

    /// The user leaves the keyboard alone until `until` on the wall clock.
    fn leave_until(&mut self, until: &str) {
        self.run_until(until, false);
    }

    fn run_until(&mut self, until: &str, typing: bool) {
        let until = at(until);
        while self.wall().unix < until.unix {
            self.now += POLL_EVERY;
            if typing {
                self.last_input = self.now;
            }
            self.poll();
        }
    }

    fn poll(&mut self) {
        let wall = self.wall();
        let out = self.quiet.sync(wall, &mut self.sched, self.now);
        self.record(out);
        let presence = Presence::from_last_input(self.now, self.last_input);
        let out = self.sched.poll(self.now, presence);
        self.record(out);
    }

    /// The user presses the tray's top item.
    fn press(&mut self) {
        let wall = self.wall();
        self.quiet.press(wall);
        let out = self.quiet.sync(wall, &mut self.sched, self.now);
        self.record(out);
    }

    fn record(&mut self, out: Vec<Effect>) {
        let wall = self.wall();
        self.effects.extend(out.into_iter().map(|e| (wall, e)));
    }

    /// The wall-clock times, as `HH:MM:SS`, at which `nudge` fired.
    fn fired_at(&self, nudge: Nudge) -> Vec<String> {
        self.at_times(|e| matches!(e, Effect::Log(Event::Fired { nudge: n, .. }) if *n == nudge))
    }

    /// The wall-clock times at which an occurrence expired with cause `quiet`.
    fn expired_quiet_at(&self) -> Vec<String> {
        self.at_times(|e| {
            matches!(
                e,
                Effect::Log(Event::Skipped {
                    by: SkipCause::Quiet,
                    ..
                })
            )
        })
    }

    fn at_times(&self, pick: impl Fn(&Effect) -> bool) -> Vec<String> {
        self.effects
            .iter()
            .filter(|(_, e)| pick(e))
            .map(|(t, _)| {
                let c = t.civil();
                format!("{:02}:{:02}:{:02}", c.hour, c.minute, c.second)
            })
            .collect()
    }
}

/// Only Pause on.
fn pause_only() -> Nudges {
    let mut n = Nudges::default();
    n.stand.on = false;
    n.water.on = false;
    n
}

#[test]
fn at_17_a_pending_pause_expires_with_cause_quiet() {
    let mut sim = Sim::with("2026-11-03T15:50:00", pause_only());
    sim.work_until("2026-11-03T16:55:00");
    assert_eq!(sim.fired_at(Nudge::Pause), ["16:50:00"]);
    let occ = sim.sched.pending(Nudge::Pause).unwrap().occ;

    sim.work_until("2026-11-03T17:00:00");
    assert_eq!(sim.sched.pending(Nudge::Pause), None);
    let at_17: Vec<_> = sim
        .effects
        .iter()
        .filter(|(t, _)| t.unix == at("2026-11-03T17:00:00").unix)
        .map(|(_, e)| e.clone())
        .collect();
    assert_eq!(
        at_17,
        [
            Effect::Log(Event::Skipped {
                nudge: Nudge::Pause,
                occ,
                by: SkipCause::Quiet
            }),
            Effect::Close,
        ]
    );

    // Nothing fires all evening, however long the user works.
    sim.work_until("2026-11-03T23:00:00");
    assert_eq!(sim.fired_at(Nudge::Pause), ["16:50:00"]);
}

#[test]
fn quiet_start_expires_a_combined_nudge_and_water_waits_for_the_morning() {
    // Water every 20 min rides at 16:00 and 16:30, is due again at 16:50 and waits; 17:00
    // expires the combined Stand/Move and Pause and drops that Water. In the morning Water
    // starts from zero with every other clock: due 08:20, riding on the 08:30 Stand/Move.
    let mut n = Nudges::default();
    n.water.every_minutes = 20;
    let mut sim = Sim::with("2026-11-03T15:30:00", n);
    sim.work_until("2026-11-03T16:59:55");
    assert!(sim.sched.pending(Nudge::Stand).is_some() && sim.sched.pending(Nudge::Pause).is_some());
    sim.work_until("2026-11-03T17:00:00");
    assert!(sim.sched.prompt().is_none());
    assert_eq!(sim.expired_quiet_at(), ["17:00:00", "17:00:00"]);

    sim.leave_until("2026-11-04T07:59:55");
    sim.work_until("2026-11-04T08:31:00");
    let water_at = sim.at_times(|e| matches!(e, Effect::Log(Event::Shown { .. })));
    assert_eq!(water_at, ["16:00:00", "16:30:00", "08:30:00"]);
}

#[test]
fn at_8_every_clock_restarts_from_zero() {
    // 20 active minutes on the Stand/Move clock at 17:00 do not carry over the night.
    let mut sim = Sim::new("2026-11-03T16:40:00");
    sim.work_until("2026-11-03T17:00:00");
    sim.leave_until("2026-11-04T07:55:00");
    sim.work_until("2026-11-04T09:01:00");
    assert_eq!(sim.fired_at(Nudge::Stand), ["08:30:00", "09:00:00"]);
    assert_eq!(sim.fired_at(Nudge::Pause), ["09:00:00"]);
}

#[test]
fn time_before_8_never_counts() {
    let mut sim = Sim::new("2026-11-03T07:00:00");
    assert!(sim.sched.is_quiet());
    sim.work_until("2026-11-03T08:31:00");
    assert_eq!(sim.fired_at(Nudge::Stand), ["08:30:00"]);
}

#[test]
fn overtime_started_at_7_runs_to_midnight_without_a_restart_at_8() {
    let mut sim = Sim::new("2026-11-03T06:59:00");
    sim.work_until("2026-11-03T07:00:00");
    assert_eq!(sim.quiet.item(sim.wall()).press, Press::StartOvertime);
    sim.press();
    assert!(!sim.sched.is_quiet());
    assert_eq!(
        sim.quiet.switch(sim.wall()),
        Some(Switch::Overtime {
            until: at("2026-11-04T00:00:00")
        })
    );

    // Stand/Move fires at 07:30 and stays pending across 08:00; the Pause comes 60 active
    // minutes after 07:00. A restart at 08:00 would have expired the Stand/Move and moved the
    // Pause to 09:00.
    sim.work_until("2026-11-03T07:59:55");
    assert_eq!(sim.fired_at(Nudge::Stand), ["07:30:00"]);
    sim.work_until("2026-11-03T08:01:00");
    assert_eq!(sim.expired_quiet_at(), Vec::<String>::new());
    assert_eq!(sim.fired_at(Nudge::Pause), ["08:00:00"]);
    let item = sim.quiet.item(sim.wall());
    assert_eq!(item.press, Press::EndOvertimeStartOff);
    assert_eq!(item.tooltip, "breakpoint: on until midnight");

    // It keeps nudging past 17:00, and goes quiet at midnight.
    sim.work_until("2026-11-03T17:31:00");
    assert!(sim.fired_at(Nudge::Stand).contains(&"17:30:00".to_string()));
    sim.work_until("2026-11-04T00:00:00");
    assert!(sim.sched.is_quiet());
    assert_eq!(sim.quiet.switch(sim.wall()), None);
    assert_eq!(sim.expired_quiet_at(), ["00:00:00", "00:00:00"]);
}

#[test]
fn off_is_quiet_and_turning_back_on_restarts_the_clocks() {
    let mut sim = Sim::new("2026-11-03T09:00:00");
    sim.work_until("2026-11-03T09:31:00");
    assert!(sim.sched.pending(Nudge::Stand).is_some());

    sim.press();
    assert!(sim.sched.is_quiet());
    assert_eq!(sim.expired_quiet_at(), ["09:31:00"]);
    assert_eq!(
        sim.quiet.item(sim.wall()).tooltip,
        "breakpoint: off until Wednesday 08:00"
    );

    sim.work_until("2026-11-03T10:00:00");
    sim.press();
    assert!(!sim.sched.is_quiet());
    sim.work_until("2026-11-03T11:01:00");
    assert_eq!(
        sim.fired_at(Nudge::Stand),
        ["09:30:00", "10:30:00", "11:00:00"]
    );
    assert_eq!(sim.fired_at(Nudge::Pause), ["11:00:00"]);
}

#[test]
fn off_set_at_16_thursday_ends_sunday_at_8_and_nudges_resume() {
    let mut sim = Sim::new("2026-11-05T15:00:00");
    sim.work_until("2026-11-05T16:00:00");
    sim.press();
    assert_eq!(
        sim.quiet.switch(sim.wall()),
        Some(Switch::Off {
            until: at("2026-11-08T08:00:00")
        })
    );
    sim.leave_until("2026-11-08T07:59:55");
    assert!(sim.sched.is_quiet());
    sim.work_until("2026-11-08T08:31:00");
    assert_eq!(sim.quiet.switch(sim.wall()), None);
    assert_eq!(sim.quiet.item(sim.wall()).press, Press::StartOff);
    assert_eq!(
        sim.fired_at(Nudge::Stand),
        ["15:30:00", "16:00:00", "08:30:00"]
    );
    assert_eq!(sim.expired_quiet_at(), ["16:00:00", "16:00:00"]);
}

#[test]
fn off_survives_a_restart() {
    let dir = std::env::temp_dir().join(format!("breakpoint-test-quiet-{}", std::process::id()));
    let path = dir.join("config.json");
    let _ = std::fs::remove_dir_all(&dir);

    // Off pressed Tuesday 10:00 and saved, as the tray does.
    let mut config = Config::default();
    let mut quiet = Quiet::new(&config);
    quiet.press(at("2026-11-03T10:00:00"));
    quiet.store(&mut config);
    config::save(&path, &config).unwrap();

    // Restarted at 11:00 the same day: still Off until Wednesday 08:00.
    let loaded = config::load(&path, at("2026-11-03T11:00:00")).unwrap();
    assert!(!loaded.notice);
    let mut sim = Sim::from_config(at("2026-11-03T11:00:00"), &loaded.config);
    assert!(sim.sched.is_quiet());
    let item = sim.quiet.item(sim.wall());
    assert_eq!(item.press, Press::EndOff);
    assert_eq!(item.tooltip, "breakpoint: off until Wednesday 08:00");
    sim.work_until("2026-11-03T16:59:00");
    assert_eq!(sim.fired_at(Nudge::Stand), Vec::<String>::new());

    // Restarted after the end moment: it is ignored and nudges run.
    let loaded = config::load(&path, at("2026-11-04T09:00:00")).unwrap();
    assert_eq!(loaded.config.off_until, None);
    let sim = Sim::from_config(at("2026-11-04T09:00:00"), &loaded.config);
    assert!(!sim.sched.is_quiet());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn overtime_survives_a_restart() {
    let mut config = Config::default();
    let mut quiet = Quiet::new(&config);
    quiet.press(at("2026-11-05T19:00:00"));
    quiet.store(&mut config);
    let loaded = Config::from_json(&config.to_json(), at("2026-11-05T20:00:00"));
    let sim = Sim::from_config(at("2026-11-05T20:00:00"), &loaded.config);
    assert!(!sim.sched.is_quiet());
    assert_eq!(sim.quiet.item(sim.wall()).press, Press::EndOvertime);
}
