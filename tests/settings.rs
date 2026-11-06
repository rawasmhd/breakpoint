//! Scenario tests for a Settings Save applied at once (SPEC §9.2), with a fake wall clock and
//! presence feed driving the quiet rules and the scheduler as the app does. Default config:
//! Sunday to Thursday, 08:00 to 17:00. 2026-11-03 is a Tuesday and 2026-11-06 a Friday.

use std::time::Duration;

use breakpoint::core::config::{Config, Enforcement, Nudges};
use breakpoint::core::presence::{POLL_EVERY, Presence};
use breakpoint::core::quiet::{Quiet, Switch};
use breakpoint::core::scheduler::{Effect, Event, Nudge, Scheduler, SkipCause};
use breakpoint::core::settings::{self, Form};
use breakpoint::core::time::{LocalTime, Weekday};

fn at(s: &str) -> LocalTime {
    LocalTime::parse_rfc3339(&format!("{s}+03:00")).unwrap()
}

struct Sim {
    config: Config,
    sched: Scheduler,
    quiet: Quiet,
    start: LocalTime,
    now: Duration,
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
        let mut sim = Sim {
            sched: Scheduler::new(config.nudges, Duration::from_secs(60)),
            quiet: Quiet::new(&config),
            config,
            start: at(start),
            now: Duration::ZERO,
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
        let until = at(until);
        while self.wall().unix < until.unix {
            self.now += POLL_EVERY;
            self.poll();
        }
    }

    fn poll(&mut self) {
        let wall = self.wall();
        let out = self.quiet.sync(wall, &mut self.sched, self.now);
        self.record(out);
        let out = self.sched.poll(self.now, Presence::Active);
        self.record(out);
    }

    /// The user opens Settings, changes it with `edit` and presses Save.
    fn save(&mut self, edit: impl FnOnce(&mut Form)) -> Vec<Effect> {
        let mut form = Form::new(&self.config);
        edit(&mut form);
        let valid = form.validate().expect("a valid Save");
        let wall = self.wall();
        let out = settings::save(
            &valid,
            &mut self.config,
            &mut self.quiet,
            &mut self.sched,
            wall,
            self.now,
        );
        self.record(out.clone());
        out
    }

    fn record(&mut self, out: Vec<Effect>) {
        let wall = self.wall();
        self.effects.extend(out.into_iter().map(|e| (wall, e)));
    }

    /// The wall-clock times, as `HH:MM`, at which `nudge` fired.
    fn fired_at(&self, nudge: Nudge) -> Vec<String> {
        self.effects
            .iter()
            .filter(|(_, e)| matches!(e, Effect::Log(Event::Fired { nudge: n, .. }) if *n == nudge))
            .map(|(t, _)| {
                let c = t.civil();
                format!("{:02}:{:02}", c.hour, c.minute)
            })
            .collect()
    }

    fn occ(&self, nudge: Nudge) -> u64 {
        self.sched.pending(nudge).expect("pending").occ
    }
}

fn skipped(nudge: Nudge, occ: u64, by: SkipCause) -> Effect {
    Effect::Log(Event::Skipped { nudge, occ, by })
}

#[test]
fn turning_stand_move_off_while_it_is_pending_logs_a_settings_expiry() {
    let mut sim = Sim::new("2026-11-03T09:00:00");
    sim.work_until("2026-11-03T09:31:00");
    let occ = sim.occ(Nudge::Stand);

    let out = sim.save(|f| f.stand.on = false);
    assert_eq!(
        out,
        [
            skipped(Nudge::Stand, occ, SkipCause::Settings),
            Effect::Close
        ]
    );
    assert_eq!(sim.sched.pending(Nudge::Stand), None);
    assert!(!sim.config.nudges.stand.on);

    // It stays off; the Pause carries on at 10:00.
    sim.work_until("2026-11-03T11:01:00");
    assert_eq!(sim.fired_at(Nudge::Stand), ["09:30"]);
    assert_eq!(sim.fired_at(Nudge::Pause), ["10:00", "11:00"]);
}

#[test]
fn unchecking_todays_work_day_during_hours_expires_pending_with_quiet() {
    let mut sim = Sim::new("2026-11-03T09:00:00");
    sim.work_until("2026-11-03T10:01:00");
    let (stand, pause) = (sim.occ(Nudge::Stand), sim.occ(Nudge::Pause));

    let out = sim.save(|f| f.work_days.set(Weekday::Tue, false));
    assert_eq!(
        out,
        [
            skipped(Nudge::Pause, pause, SkipCause::Quiet),
            skipped(Nudge::Stand, stand, SkipCause::Quiet),
            Effect::Close,
        ]
    );
    assert!(sim.sched.is_quiet());
    assert!(!sim.config.work_days.contains(Weekday::Tue));

    sim.work_until("2026-11-03T16:59:00");
    assert_eq!(sim.fired_at(Nudge::Pause), ["10:00"]);
}

#[test]
fn checking_todays_day_ends_quiet_and_every_clock_starts_from_zero() {
    // Friday 10:00 is the weekend: quiet. Checking Friday at 10:00 starts nudging from zero.
    let mut sim = Sim::new("2026-11-06T09:00:00");
    sim.work_until("2026-11-06T10:00:00");
    assert!(sim.sched.is_quiet());
    sim.save(|f| f.work_days.set(Weekday::Fri, true));
    assert!(!sim.sched.is_quiet());
    sim.work_until("2026-11-06T11:01:00");
    assert_eq!(sim.fired_at(Nudge::Stand), ["10:30", "11:00"]);
    assert_eq!(sim.fired_at(Nudge::Pause), ["11:00"]);
}

#[test]
fn moving_the_end_of_work_hours_to_before_now_makes_it_quiet() {
    let mut sim = Sim::new("2026-11-03T15:00:00");
    sim.work_until("2026-11-03T15:31:00");
    let occ = sim.occ(Nudge::Stand);
    sim.save(|f| f.end = breakpoint::core::time::TimeOfDay::new(15, 0).unwrap());
    assert!(sim.sched.is_quiet());
    assert!(
        sim.effects
            .iter()
            .any(|(_, e)| *e == skipped(Nudge::Stand, occ, SkipCause::Quiet))
    );
}

#[test]
fn a_changed_cadence_restarts_its_clock_and_the_pending_one_expires_at_the_new_due() {
    let mut sim = Sim::new("2026-11-03T09:00:00");
    sim.work_until("2026-11-03T09:31:00");
    let occ = sim.occ(Nudge::Stand);

    // At 09:31, Stand/Move every 45: the next is due 10:16, and the pending one waits for it.
    assert_eq!(sim.save(|f| f.stand.every = "45".into()), []);
    assert_eq!(sim.occ(Nudge::Stand), occ, "still pending");
    sim.work_until("2026-11-03T10:17:00");
    assert_eq!(sim.fired_at(Nudge::Stand), ["09:30", "10:16"]);
    assert!(
        sim.effects
            .iter()
            .any(|(_, e)| *e == skipped(Nudge::Stand, occ, SkipCause::Next))
    );
    // The Pause was not touched: still 10:00.
    assert_eq!(sim.fired_at(Nudge::Pause), ["10:00"]);
}

#[test]
fn a_changed_length_restarts_its_clock_and_the_pending_one_keeps_its_length() {
    let mut sim = Sim::new("2026-11-03T09:00:00");
    sim.work_until("2026-11-03T09:20:00");
    sim.save(|f| f.stand.length = "4".into());
    sim.work_until("2026-11-03T09:51:00");
    assert_eq!(sim.fired_at(Nudge::Stand), ["09:50"]);
    let pending = sim.sched.pending(Nudge::Stand).unwrap();
    assert_eq!(pending.length_minutes, 4);

    // Back to 2 while that one is pending: it keeps the 4 it fired with.
    sim.save(|f| f.stand.length = "2".into());
    assert_eq!(sim.sched.pending(Nudge::Stand).unwrap().length_minutes, 4);
}

#[test]
fn an_enforcement_change_applies_from_the_next_occurrence_without_a_restart() {
    let mut sim = Sim::new("2026-11-03T09:00:00");
    sim.work_until("2026-11-03T09:31:00");
    assert_eq!(sim.save(|f| f.stand.enforcement = Enforcement::Firm), []);
    assert_eq!(
        sim.sched.pending(Nudge::Stand).unwrap().level,
        Enforcement::Soft,
        "the pending one keeps its level"
    );
    // No restart: the next is still 10:00, with the Pause, and fires Firm.
    sim.work_until("2026-11-03T10:01:00");
    assert_eq!(sim.fired_at(Nudge::Stand), ["09:30", "10:00"]);
    assert_eq!(
        sim.sched.pending(Nudge::Stand).unwrap().level,
        Enforcement::Firm
    );
}

#[test]
fn an_unchanged_save_changes_nothing() {
    let mut sim = Sim::new("2026-11-03T09:00:00");
    sim.work_until("2026-11-03T09:20:00");
    assert_eq!(sim.save(|_| {}), []);
    sim.work_until("2026-11-03T09:31:00");
    assert_eq!(sim.fired_at(Nudge::Stand), ["09:30"]);
}

#[test]
fn turning_pause_off_under_a_combined_nudge_shows_the_stand_move_left() {
    let mut sim = Sim::new("2026-11-03T09:00:00");
    sim.work_until("2026-11-03T10:01:00");
    let pause = sim.occ(Nudge::Pause);
    let out = sim.save(|f| f.pause.on = false);
    assert_eq!(out[0], skipped(Nudge::Pause, pause, SkipCause::Settings));
    assert!(matches!(&out[1], Effect::Show(p) if p.nudge == Nudge::Stand));
    assert_eq!(out.len(), 2);
}

#[test]
fn turning_a_nudge_on_starts_its_clock_from_the_save() {
    let mut n = Nudges::default();
    n.stand.on = false;
    let mut sim = Sim::with("2026-11-03T09:00:00", n);
    sim.work_until("2026-11-03T09:40:00");
    sim.save(|f| f.stand.on = true);
    sim.work_until("2026-11-03T10:41:00");
    assert_eq!(sim.fired_at(Nudge::Stand), ["10:10", "10:40"]);
}

#[test]
fn an_off_in_force_keeps_its_end_moment_through_a_save() {
    let mut sim = Sim::new("2026-11-03T09:00:00");
    sim.quiet.press(sim.wall());
    sim.quiet.store(&mut sim.config);
    let off = Some(Switch::Off {
        until: at("2026-11-04T08:00:00"),
    });
    assert_eq!(sim.quiet.switch(sim.wall()), off);

    // Making Wednesday a weekend day does not move the Off to Thursday.
    sim.save(|f| f.work_days.set(Weekday::Wed, false));
    assert_eq!(sim.quiet.switch(sim.wall()), off);
    assert_eq!(sim.config.off_until, Some(at("2026-11-04T08:00:00")));
}
