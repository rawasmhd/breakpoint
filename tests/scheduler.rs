//! Scenario tests for the scheduler with a fake clock and presence feed.

use std::time::Duration;

use breakpoint::core::presence::{AwayReason, POLL_EVERY, Presence};
use breakpoint::core::scheduler::{
    Effect, Event, Nudge, Prompt, SatisfyCause, Scheduler, SkipCause, away_left, countdown_text,
};

const MIN: Duration = Duration::from_secs(60);
const SEC: Duration = Duration::from_secs(1);

/// A fake clock and presence feed: polls the scheduler every [`POLL_EVERY`] like the platform,
/// with the user either at the keyboard, gone, or in a state that an event or a veto decides,
/// and records what came out and when.
struct Sim {
    sched: Scheduler,
    now: Duration,
    last_input: Duration,
    /// A presence that events or vetoes decide, in force instead of the input-only verdict.
    held: Option<Presence>,
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
            held: None,
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

    /// The user is `Away` for `reason` for `d`, with no input; then the user is back.
    fn lock(&mut self, reason: AwayReason, d: Duration) {
        let since = self.last_input;
        self.hold(Presence::Away { since, reason }, d);
    }

    /// The presence is `presence` for `d`, with no input.
    fn hold(&mut self, presence: Presence, d: Duration) {
        self.held = Some(presence);
        self.leave(d);
        self.held = None;
    }

    /// The user touches the keyboard again after an absence. Returns the moment active time
    /// resumes: the poll that notices the input.
    fn come_back(&mut self) -> Duration {
        self.work(POLL_EVERY);
        self.now
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
        let presence = self
            .held
            .unwrap_or_else(|| Presence::from_last_input(self.now, self.last_input));
        let out = self.sched.poll(self.now, presence);
        self.record(out);
    }

    fn skip(&mut self) {
        let out = self.sched.skip(self.now);
        self.record(out);
    }

    fn snooze(&mut self) {
        let out = self.sched.snooze(self.now);
        self.record(out);
    }

    fn record(&mut self, out: Vec<Effect>) {
        self.effects.extend(out.into_iter().map(|e| (self.now, e)));
    }

    /// The times at which `nudge` fired.
    fn fired_at(&self, nudge: Nudge) -> Vec<Duration> {
        self.effects
            .iter()
            .filter(|(_, e)| matches!(e, Effect::Log(Event::Fired { nudge: n, .. }) if *n == nudge))
            .map(|&(t, _)| t)
            .collect()
    }

    /// The times at which the popup was shown.
    fn shown_at(&self) -> Vec<Duration> {
        self.effects
            .iter()
            .filter(|(_, e)| matches!(e, Effect::Show(_)))
            .map(|&(t, _)| t)
            .collect()
    }

    fn take(&mut self) -> Vec<Effect> {
        self.effects.drain(..).map(|(_, e)| e).collect()
    }

    fn pending_occ(&self, nudge: Nudge) -> Option<u64> {
        self.sched.pending(nudge).map(|o| o.occ)
    }
}

fn fired(nudge: Nudge, occ: u64) -> Effect {
    Effect::Log(Event::Fired { nudge, occ })
}

fn snoozed(nudge: Nudge, occ: u64) -> Effect {
    Effect::Log(Event::Snoozed { nudge, occ })
}

fn satisfied(nudge: Nudge, occ: u64, by: SatisfyCause) -> Effect {
    Effect::Log(Event::Satisfied { nudge, occ, by })
}

fn skipped(nudge: Nudge, occ: u64, by: SkipCause) -> Effect {
    Effect::Log(Event::Skipped { nudge, occ, by })
}

fn prompt(nudge: Nudge, snooze: bool) -> Prompt {
    let (minutes, line) = match nudge {
        Nudge::Stand => (2, "Stand, roll your shoulders, look at something far away."),
        Nudge::Pause => (5, "Step away from the screen for 5 minutes."),
    };
    Prompt {
        nudge,
        title: nudge.title(),
        line: line.into(),
        length: minutes * MIN,
        snooze,
    }
}

fn show(nudge: Nudge) -> Effect {
    Effect::Show(prompt(nudge, true))
}

/// A Sim with the first Stand/Move (occ 1) just fired and its popup taken.
fn stand_pending() -> Sim {
    let mut sim = Sim::new();
    sim.work(30 * MIN);
    assert_eq!(sim.take(), [fired(Nudge::Stand, 1), show(Nudge::Stand)]);
    sim
}

/// A Sim at 60 active minutes with the first Stand/Move skipped, so the Pause (occ 2) and the
/// second Stand/Move (occ 3) have just fired together and the popup shows the Pause.
fn pause_pending() -> Sim {
    let mut sim = stand_pending();
    sim.skip();
    sim.work(30 * MIN);
    sim.take();
    assert_eq!(sim.pending_occ(Nudge::Pause), Some(2));
    assert_eq!(sim.pending_occ(Nudge::Stand), Some(3));
    sim
}

// SPEC §3: the nudges and their copy.

#[test]
fn the_popup_shows_each_nudges_copy() {
    assert_eq!(Nudge::Stand.title(), "Stand up and move");
    assert_eq!(
        Nudge::Stand.line(2),
        "Stand, roll your shoulders, look at something far away."
    );
    assert_eq!(Nudge::Pause.title(), "Pause");
    assert_eq!(
        Nudge::Pause.line(5),
        "Step away from the screen for 5 minutes."
    );
}

// SPEC §5.1 Firing.

#[test]
fn stand_fires_after_30_active_minutes_with_no_warning_before() {
    let mut sim = Sim::new();
    sim.work(30 * MIN - POLL_EVERY);
    assert!(sim.take().is_empty());

    sim.work(POLL_EVERY);
    assert_eq!(sim.take(), [fired(Nudge::Stand, 1), show(Nudge::Stand)]);
}

#[test]
fn pause_fires_after_60_active_minutes_and_shows_in_front() {
    let mut sim = stand_pending();
    sim.skip();
    sim.take();
    sim.work(30 * MIN - POLL_EVERY);
    assert!(sim.take().is_empty());

    sim.work(POLL_EVERY);
    assert_eq!(
        sim.take(),
        [
            fired(Nudge::Pause, 2),
            fired(Nudge::Stand, 3),
            show(Nudge::Pause)
        ]
    );
}

#[test]
fn the_clock_restarts_at_firing_so_the_next_comes_one_cadence_later() {
    let mut sim = stand_pending();
    sim.work(MIN);
    sim.skip();
    sim.work(29 * MIN);

    assert_eq!(sim.fired_at(Nudge::Stand), [60 * MIN]);
    assert_eq!(sim.pending_occ(Nudge::Stand), Some(3));
}

#[test]
fn the_next_falling_due_while_pending_expires_the_old_and_shows_the_new() {
    let mut sim = stand_pending();
    sim.work(25 * MIN);
    assert!(sim.take().is_empty());

    // Stand/Move at 60 falls due with the Pause; the old Stand/Move expires.
    sim.work(5 * MIN);
    assert_eq!(
        sim.take(),
        [
            fired(Nudge::Pause, 2),
            skipped(Nudge::Stand, 1, SkipCause::Next),
            fired(Nudge::Stand, 3),
            show(Nudge::Pause)
        ]
    );
}

#[test]
fn an_expired_pause_is_replaced_by_the_new_one() {
    let mut sim = pause_pending();
    sim.skip();
    sim.take();
    // Stand/Move at 90 comes and is left pending; so is nothing else.
    sim.work(30 * MIN);
    sim.skip();
    sim.take();
    sim.work(30 * MIN);
    sim.take();
    assert_eq!(sim.pending_occ(Nudge::Pause), Some(5));

    sim.work(60 * MIN);
    let out = sim.take();
    assert!(out.contains(&skipped(Nudge::Pause, 5, SkipCause::Next)));
    assert!(out.contains(&show(Nudge::Pause)));
    assert!(!out.contains(&Effect::Close), "replaced, not closed");
}

#[test]
fn a_shorter_minute_scales_the_cadence() {
    // BREAKPOINT_MINUTE_SECS=2: 30 "minutes" are a minute of real activity.
    let mut sim = Sim::with_minute(2 * SEC);
    sim.work(60 * SEC);
    assert_eq!(sim.fired_at(Nudge::Stand), [60 * SEC]);
}

// SPEC §5.2 Presence resets clocks.

#[test]
fn a_short_absence_under_90_s_still_counts_as_active() {
    let mut sim = Sim::new();
    sim.work(20 * MIN);
    sim.leave(MIN);
    sim.work(9 * MIN);
    assert_eq!(sim.fired_at(Nudge::Stand), [30 * MIN]);
}

#[test]
fn away_shorter_than_the_stand_length_pauses_the_clock_without_restarting_it() {
    let mut sim = Sim::new();
    sim.work(20 * MIN);
    // 90 s of silence count as active; the next 20 s are Away and do not.
    sim.leave(110 * SEC);
    sim.work(8 * MIN);
    assert!(sim.fired_at(Nudge::Stand).is_empty());

    sim.work(MIN);
    let back = 20 * MIN + 110 * SEC;
    // 21.5 min on the clock at the return, 8.5 more to go, plus the poll that notices the input.
    assert_eq!(
        sim.fired_at(Nudge::Stand),
        [back + 8 * MIN + 30 * SEC + POLL_EVERY]
    );
}

#[test]
fn away_for_the_stand_length_restarts_the_stand_clock_with_nothing_pending() {
    let mut sim = Sim::new();
    sim.work(20 * MIN);
    sim.leave(2 * MIN);
    let back = sim.come_back();
    sim.work(30 * MIN);
    assert_eq!(sim.fired_at(Nudge::Stand), [back + 30 * MIN]);
}

#[test]
fn any_lock_restarts_the_stand_clock_with_nothing_pending() {
    let mut sim = Sim::new();
    sim.work(20 * MIN);
    sim.lock(AwayReason::Lock, 10 * SEC);
    let back = sim.come_back();
    sim.work(30 * MIN);
    assert_eq!(sim.fired_at(Nudge::Stand), [back + 30 * MIN]);
}

#[test]
fn away_for_the_pause_length_restarts_the_focus_period() {
    let mut sim = Sim::new();
    sim.work(50 * MIN);
    sim.skip();
    sim.take();
    // Lunch: the Pause clock starts again from zero at the return.
    sim.leave(60 * MIN);
    let back = sim.come_back();
    sim.work(60 * MIN);
    assert_eq!(sim.fired_at(Nudge::Pause), [back + 60 * MIN]);
}

#[test]
fn away_shorter_than_the_pause_length_does_not_restart_the_focus_period() {
    let mut sim = Sim::new();
    sim.work(50 * MIN);
    // 3 min away resets Stand/Move but not Pause: 1.5 min of it counted as active.
    sim.leave(3 * MIN);
    let back = sim.now;
    sim.work(10 * MIN);
    assert_eq!(
        sim.fired_at(Nudge::Pause),
        [back + 8 * MIN + 30 * SEC + POLL_EVERY]
    );
}

#[test]
fn a_short_lock_with_nothing_pending_does_not_restart_the_focus_period() {
    let mut sim = Sim::new();
    sim.work(50 * MIN);
    sim.skip();
    // The poll that sees the lock still counts the 5 s before it as active.
    sim.lock(AwayReason::Lock, 10 * SEC);
    let back = sim.come_back();
    sim.work(10 * MIN);
    assert_eq!(sim.fired_at(Nudge::Pause), [back + 10 * MIN - POLL_EVERY]);
}

#[test]
fn a_satisfied_pause_starts_a_fresh_focus_period_even_after_a_short_lock() {
    let mut sim = pause_pending();
    sim.work(10 * MIN);
    sim.lock(AwayReason::Lock, 10 * SEC);
    let back = sim.come_back();
    sim.work(60 * MIN);
    assert_eq!(sim.fired_at(Nudge::Pause), [back + 60 * MIN]);
}

#[test]
fn a_satisfied_stand_restarts_its_own_clock() {
    let mut sim = stand_pending();
    sim.work(5 * MIN);
    sim.leave(2 * MIN);
    assert_eq!(sim.pending_occ(Nudge::Stand), None);
    let back = sim.come_back();
    sim.work(30 * MIN);
    assert_eq!(sim.fired_at(Nudge::Stand), [back + 30 * MIN]);
}

#[test]
fn a_pause_returned_from_early_stays_pending_and_its_clock_keeps_running() {
    let mut sim = pause_pending();
    sim.leave(4 * MIN);
    let back = sim.come_back();
    assert_eq!(sim.pending_occ(Nudge::Pause), Some(2));
    // 90 s of the 4 min counted as active; the clock was not restarted.
    sim.work(60 * MIN);
    assert_eq!(sim.fired_at(Nudge::Pause), [back + 60 * MIN - 90 * SEC]);
}

// SPEC §5.5 Ending: satisfied.

#[test]
fn away_for_the_stand_length_from_the_last_input_satisfies() {
    let mut sim = stand_pending();
    sim.work(MIN);
    let last_input = sim.now;
    sim.leave(2 * MIN - POLL_EVERY);
    assert!(sim.take().is_empty(), "satisfied early");

    sim.leave(POLL_EVERY);
    assert_eq!(sim.now - last_input, 2 * MIN);
    assert_eq!(
        sim.take(),
        [
            satisfied(Nudge::Stand, 1, SatisfyCause::Away),
            Effect::Close
        ]
    );
}

#[test]
fn coming_back_early_leaves_it_pending() {
    let mut sim = stand_pending();
    sim.leave(100 * SEC);
    sim.work(MIN);
    assert!(sim.take().is_empty());
    assert_eq!(sim.pending_occ(Nudge::Stand), Some(1));
}

#[test]
fn two_short_absences_do_not_add_up() {
    let mut sim = stand_pending();
    sim.leave(100 * SEC);
    sim.work(POLL_EVERY);
    sim.leave(100 * SEC);
    sim.work(POLL_EVERY);
    assert!(sim.take().is_empty());
    assert_eq!(sim.pending_occ(Nudge::Stand), Some(1));
}

#[test]
fn a_lock_satisfies_at_once() {
    let mut sim = stand_pending();
    sim.lock(AwayReason::Lock, POLL_EVERY);
    assert_eq!(
        sim.take(),
        [
            satisfied(Nudge::Stand, 1, SatisfyCause::Lock),
            Effect::Close
        ]
    );
}

#[test]
fn suspend_and_display_off_satisfy_at_once() {
    for (reason, by) in [
        (AwayReason::Suspend, SatisfyCause::Suspend),
        (AwayReason::DisplayOff, SatisfyCause::DisplayOff),
    ] {
        let mut sim = pause_pending();
        sim.lock(reason, POLL_EVERY);
        assert_eq!(
            sim.take(),
            [
                satisfied(Nudge::Pause, 2, by),
                satisfied(Nudge::Stand, 3, by),
                Effect::Close
            ]
        );
    }
}

#[test]
fn a_pause_needs_the_pause_length_while_the_stand_with_it_needs_only_the_stand_length() {
    let mut sim = pause_pending();
    let last_input = sim.now;
    sim.leave(2 * MIN);
    assert_eq!(
        sim.take(),
        [satisfied(Nudge::Stand, 3, SatisfyCause::Away)],
        "the Pause stays on screen"
    );
    assert_eq!(sim.pending_occ(Nudge::Pause), Some(2));

    sim.leave(3 * MIN);
    assert_eq!(sim.now - last_input, 5 * MIN);
    assert_eq!(
        sim.take(),
        [
            satisfied(Nudge::Pause, 2, SatisfyCause::Away),
            Effect::Close
        ]
    );
}

#[test]
fn windows_user_inactive_verdict_satisfies_only_after_the_length() {
    let mut sim = stand_pending();
    sim.lock(AwayReason::UserInactive, MIN);
    sim.come_back();
    assert!(sim.take().is_empty(), "satisfied at once like a lock");
    assert_eq!(sim.pending_occ(Nudge::Stand), Some(1));

    sim.lock(AwayReason::UserInactive, 2 * MIN);
    assert_eq!(
        sim.take(),
        [
            satisfied(Nudge::Stand, 1, SatisfyCause::Away),
            Effect::Close
        ]
    );
}

#[test]
fn at_desk_counts_as_active_time_and_never_satisfies() {
    let mut sim = Sim::new();
    sim.work(10 * MIN);
    // A video plays with no input: present, not typing.
    sim.hold(Presence::AtDesk, 30 * MIN);
    assert_eq!(sim.fired_at(Nudge::Stand), [30 * MIN]);
    assert_eq!(
        sim.pending_occ(Nudge::Stand),
        Some(1),
        "satisfied while AtDesk"
    );
}

#[test]
fn unknown_neither_advances_nor_restarts_the_clocks() {
    let mut sim = Sim::new();
    sim.work(20 * MIN);
    // The machine woke itself and nobody touched it.
    sim.hold(Presence::Unknown, 60 * MIN);
    let back = sim.come_back();
    sim.work(10 * MIN);
    assert_eq!(sim.fired_at(Nudge::Stand), [back + 10 * MIN - POLL_EVERY]);
}

// SPEC §5.5 Ending: skipped.

#[test]
fn skip_ends_it_as_skipped_by_the_user_and_the_next_comes_as_usual() {
    let mut sim = stand_pending();
    sim.work(20 * MIN);
    assert_eq!(sim.pending_occ(Nudge::Stand), Some(1));

    sim.skip();
    assert_eq!(
        sim.take(),
        [skipped(Nudge::Stand, 1, SkipCause::User), Effect::Close]
    );
    assert_eq!(sim.sched.prompt(), None);
    sim.work(10 * MIN);
    assert_eq!(sim.fired_at(Nudge::Stand), [60 * MIN]);
}

#[test]
fn skip_with_nothing_pending_does_nothing() {
    let mut sim = Sim::new();
    sim.work(10 * MIN);
    sim.skip();
    assert!(sim.take().is_empty());
}

#[test]
fn quit_expires_the_pending_occurrences() {
    let mut sim = pause_pending();
    let out = sim.sched.quit(sim.now);
    assert_eq!(
        out,
        [
            skipped(Nudge::Pause, 2, SkipCause::Exit),
            skipped(Nudge::Stand, 3, SkipCause::Exit),
            Effect::Close
        ]
    );
}

// SPEC §5.6 Snooze.

#[test]
fn snooze_hides_the_popup_and_reprompts_after_5_active_minutes_with_skip_only() {
    let mut sim = stand_pending();
    sim.snooze();
    assert_eq!(sim.take(), [snoozed(Nudge::Stand, 1), Effect::Close]);
    let snoozed_at = sim.now;

    sim.work(5 * MIN - POLL_EVERY);
    assert!(sim.take().is_empty());
    sim.work(POLL_EVERY);
    assert_eq!(sim.take(), [Effect::Show(prompt(Nudge::Stand, false))]);
    assert_eq!(sim.now, snoozed_at + 5 * MIN);
    assert_eq!(sim.sched.prompt(), Some(prompt(Nudge::Stand, false)));

    // Prompted again once only.
    sim.work(20 * MIN);
    assert!(sim.take().is_empty());
}

#[test]
fn snooze_is_allowed_once() {
    let mut sim = stand_pending();
    sim.snooze();
    sim.take();
    sim.snooze();
    assert!(sim.take().is_empty(), "a second snooze while snoozed");

    sim.work(5 * MIN);
    sim.take();
    sim.snooze();
    assert!(sim.take().is_empty(), "a snooze after the re-prompt");
}

#[test]
fn skip_after_the_reprompt_ends_it() {
    let mut sim = stand_pending();
    sim.snooze();
    sim.work(5 * MIN);
    sim.take();
    sim.skip();
    assert_eq!(
        sim.take(),
        [skipped(Nudge::Stand, 1, SkipCause::User), Effect::Close]
    );
}

#[test]
fn snooze_counts_active_time_only() {
    let mut sim = stand_pending();
    sim.snooze();
    let snoozed_at = sim.now;
    // 90 s of silence count; the next 20 s Away do not and are too short to satisfy.
    sim.leave(110 * SEC);
    sim.work(5 * MIN);
    assert_eq!(
        sim.shown_at(),
        [snoozed_at + 5 * MIN + 20 * SEC + POLL_EVERY]
    );
}

#[test]
fn walking_away_during_a_snooze_satisfies_and_cancels_the_reprompt() {
    let mut sim = stand_pending();
    sim.snooze();
    sim.take();
    sim.leave(2 * MIN);
    assert_eq!(
        sim.take(),
        [
            satisfied(Nudge::Stand, 1, SatisfyCause::Away),
            Effect::Close
        ]
    );
    sim.work(10 * MIN);
    assert!(sim.take().is_empty(), "re-prompted after satisfaction");
}

#[test]
fn snooze_acts_on_both_pending_occurrences() {
    let mut sim = pause_pending();
    sim.snooze();
    assert_eq!(
        sim.take(),
        [
            snoozed(Nudge::Pause, 2),
            snoozed(Nudge::Stand, 3),
            Effect::Close
        ]
    );
    sim.work(5 * MIN);
    assert_eq!(sim.take(), [Effect::Show(prompt(Nudge::Pause, false))]);
}

#[test]
fn snooze_with_nothing_pending_does_nothing() {
    let mut sim = Sim::new();
    sim.snooze();
    assert!(sim.take().is_empty());
}

#[test]
fn a_new_occurrence_after_an_expiry_can_be_snoozed_again() {
    let mut sim = stand_pending();
    sim.snooze();
    sim.work(25 * MIN);
    sim.skip();
    sim.take();
    sim.work(30 * MIN);
    sim.take();
    assert_eq!(sim.sched.prompt().map(|p| p.snooze), Some(true));
}

// SPEC §6: the popup's countdown.

#[test]
fn the_countdown_runs_from_the_last_input_and_restarts_on_input() {
    let length = 2 * MIN;
    assert_eq!(away_left(length, 100 * SEC, 100 * SEC, false), 2 * MIN);
    assert_eq!(away_left(length, 101 * SEC, 100 * SEC, false), 119 * SEC);
    assert_eq!(
        away_left(length, 400 * SEC, 100 * SEC, false),
        Duration::ZERO
    );
    // New input moves the last input, so it starts over.
    assert_eq!(away_left(length, 400 * SEC, 390 * SEC, false), 110 * SEC);
}

#[test]
fn the_countdown_holds_at_the_full_length_under_a_veto() {
    assert_eq!(away_left(5 * MIN, 400 * SEC, 100 * SEC, true), 5 * MIN);
}

#[test]
fn the_countdown_reads_minutes_and_seconds() {
    assert_eq!(countdown_text(2 * MIN), "Away 2:00 more and it counts");
    assert_eq!(
        countdown_text(119 * SEC - Duration::from_millis(300)),
        "Away 1:59 more and it counts"
    );
    assert_eq!(
        countdown_text(Duration::ZERO),
        "Away 0:00 more and it counts"
    );
}
