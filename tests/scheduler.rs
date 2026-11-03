//! Scenario tests for the scheduler with a fake clock and presence feed.

use std::time::Duration;

use breakpoint::core::presence::{POLL_EVERY, Presence};
use breakpoint::core::scheduler::{Effect, Event, Nudge, Occurrence, Scheduler, SkipCause};

const MIN: Duration = Duration::from_secs(60);

/// A fake clock and presence feed: polls the scheduler every [`POLL_EVERY`] like the platform,
/// with the user either at the keyboard or gone, and records what came out and when.
struct Sim {
    sched: Scheduler,
    now: Duration,
    last_input: Duration,
    effects: Vec<(Duration, Effect)>,
}

impl Sim {
    fn new() -> Self {
        Self::with_minute(MIN)
    }

    fn with_minute(minute: Duration) -> Self {
        let mut sim = Sim {
            sched: Scheduler::new(minute),
            now: Duration::ZERO,
            last_input: Duration::ZERO,
            effects: Vec::new(),
        };
        sim.poll();
        sim
    }

    /// The user keeps typing for `d`.
    fn work(&mut self, d: Duration) {
        self.run(d, true);
    }

    /// The user leaves the keyboard alone for `d`.
    fn leave(&mut self, d: Duration) {
        self.run(d, false);
    }

    fn run(&mut self, d: Duration, typing: bool) {
        let end = self.now + d;
        while self.now < end {
            self.now += POLL_EVERY;
            if typing {
                self.last_input = self.now;
            }
            self.poll();
        }
    }

    fn poll(&mut self) {
        let presence = Presence::from_last_input(self.now, self.last_input);
        let out = self.sched.poll(self.now, presence);
        self.record(out);
    }

    fn skip(&mut self) {
        let out = self.sched.skip(self.now);
        self.record(out);
    }

    fn record(&mut self, out: Vec<Effect>) {
        self.effects.extend(out.into_iter().map(|e| (self.now, e)));
    }

    /// The times at which a Stand/Move fired.
    fn fired_at(&self) -> Vec<Duration> {
        self.effects
            .iter()
            .filter(|(_, e)| matches!(e, Effect::Log(Event::Fired { .. })))
            .map(|&(t, _)| t)
            .collect()
    }

    fn take(&mut self) -> Vec<Effect> {
        self.effects.drain(..).map(|(_, e)| e).collect()
    }
}

fn fired(occ: u64) -> Effect {
    Effect::Log(Event::Fired {
        nudge: Nudge::Stand,
        occ,
    })
}

fn skipped(occ: u64, by: SkipCause) -> Effect {
    Effect::Log(Event::Skipped {
        nudge: Nudge::Stand,
        occ,
        by,
    })
}

fn show_stand() -> Effect {
    Effect::Show(Nudge::Stand.prompt())
}

#[test]
fn stand_fires_after_30_active_minutes() {
    let mut sim = Sim::new();
    sim.work(30 * MIN - POLL_EVERY);
    assert!(sim.take().is_empty());

    sim.work(POLL_EVERY);
    assert_eq!(sim.take(), [fired(1), show_stand()]);
}

#[test]
fn the_popup_shows_the_stand_move_copy() {
    let prompt = Nudge::Stand.prompt();
    assert_eq!(prompt.title, "Stand up and move");
    assert_eq!(
        prompt.line,
        "Stand, roll your shoulders, look at something far away."
    );
}

#[test]
fn a_fired_occurrence_stays_pending_until_skip() {
    let mut sim = Sim::new();
    sim.work(30 * MIN);
    sim.take();

    sim.work(20 * MIN);
    assert!(sim.take().is_empty());
    assert_eq!(
        sim.sched.pending(),
        Some(Occurrence {
            nudge: Nudge::Stand,
            occ: 1
        })
    );

    sim.skip();
    assert_eq!(sim.take(), [skipped(1, SkipCause::User), Effect::Close]);
    assert_eq!(sim.sched.pending(), None);
}

#[test]
fn after_skip_the_next_fires_one_cadence_after_the_last_fired() {
    let mut sim = Sim::new();
    sim.work(30 * MIN);
    sim.work(MIN);
    sim.skip();
    sim.work(29 * MIN);

    assert_eq!(sim.fired_at(), [30 * MIN, 60 * MIN]);
    assert_eq!(
        sim.sched.pending().map(|o| o.occ),
        Some(2),
        "the second occurrence is pending"
    );
}

#[test]
fn skip_with_nothing_pending_does_nothing() {
    let mut sim = Sim::new();
    sim.work(10 * MIN);
    sim.skip();
    assert!(sim.take().is_empty());
}

#[test]
fn time_away_does_not_advance_the_cadence() {
    let mut sim = Sim::new();
    sim.work(20 * MIN);
    sim.leave(60 * MIN);
    // The 90 s of silence before Away is declared count as active (not backdated), so the
    // clock stands at 21.5 min: 8 more active minutes are not enough.
    sim.work(8 * MIN);
    assert!(sim.fired_at().is_empty(), "fired during the absence");

    sim.work(MIN);
    let fired = sim.fired_at();
    assert_eq!(fired.len(), 1);
    // 80 wall-clock minutes away, so only active time brought it here: 28.5 min of work plus
    // the 90 s, plus one poll to notice the return.
    let back = 80 * MIN;
    assert_eq!(fired[0], back + 8 * MIN + MIN / 2 + POLL_EVERY);
}

#[test]
fn a_short_absence_under_90_s_still_counts_as_active() {
    let mut sim = Sim::new();
    sim.work(20 * MIN);
    sim.leave(MIN);
    sim.work(9 * MIN);
    assert_eq!(sim.fired_at(), [30 * MIN]);
}

#[test]
fn the_next_falling_due_while_pending_expires_the_old_and_shows_the_new() {
    let mut sim = Sim::new();
    sim.work(30 * MIN);
    sim.take();

    sim.work(30 * MIN);
    assert_eq!(
        sim.take(),
        [skipped(1, SkipCause::Next), fired(2), show_stand()]
    );
}

#[test]
fn quit_expires_the_pending_occurrence() {
    let mut sim = Sim::new();
    sim.work(30 * MIN);
    sim.take();

    let out = sim.sched.quit(sim.now);
    assert_eq!(out, [skipped(1, SkipCause::Exit), Effect::Close]);
}

#[test]
fn a_shorter_minute_scales_the_cadence() {
    // BREAKPOINT_MINUTE_SECS=2: 30 "minutes" are a minute of real activity.
    let mut sim = Sim::with_minute(Duration::from_secs(2));
    sim.work(Duration::from_secs(60));
    assert_eq!(sim.fired_at(), [Duration::from_secs(60)]);
}
