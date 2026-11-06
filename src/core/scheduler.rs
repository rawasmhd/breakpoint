//! Clocks in active time, firing, combining, Water riding along, snooze, Firm repeats and expiry.
//!
//! A pure state machine: the platform feeds it the time (an offset on one monotonic timeline),
//! presence polls and user actions, and carries out the [`Effect`]s it returns. So far Stand/Move
//! and Pause with satisfaction, snooze and expiry, Soft and Firm enforcement, the combined nudge
//! and Water riding along, with their cadences, lengths, levels and on/off from config. Whether
//! it is quiet time comes from outside ([`super::quiet`]) through [`Scheduler::set_quiet`].

use std::time::Duration;

use super::config::{AwayNudge, Enforcement, Nudges};
use super::presence::{AwayReason, Presence};

/// How far a Snooze pushes an occurrence, in active minutes (SPEC §5.6). Fixed, not a setting.
pub const SNOOZE_MINUTES: u32 = 5;

/// How often a Firm occurrence's popup comes back while it is pending, in active minutes
/// (SPEC §5.7). Fixed, not a setting.
pub const FIRM_REPEAT_MINUTES: u32 = 5;

/// The combined-nudge window, in active minutes (SPEC §5.3): when the first of Stand/Move and
/// Pause falls due, the other fires with it if it is due within this. Fixed, not a setting.
pub const COMBINE_MINUTES: u32 = 2;

/// The line a due Water adds to the next newly fired popup (SPEC §3, §5.4).
pub const WATER_LINE: &str = "Drink some water.";

/// The title of Water alone, the popup shown when Stand/Move and Pause are both off (SPEC §3).
pub const WATER_TITLE: &str = "Drink some water";

/// How long Water alone stays on screen before it hides itself (SPEC §5.4). Real seconds, even
/// under the `BREAKPOINT_MINUTE_SECS` dev aid.
pub const WATER_ALONE_FOR: Duration = Duration::from_secs(7);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nudge {
    Stand,
    Pause,
}

impl Nudge {
    /// The popup title (SPEC §3, nudge text).
    pub fn title(self) -> &'static str {
        match self {
            Nudge::Stand => "Stand up and move",
            Nudge::Pause => "Pause",
        }
    }

    /// The popup line for an occurrence that fired with a length of `length_minutes`.
    pub fn line(self, length_minutes: u32) -> String {
        match self {
            Nudge::Stand => "Stand, roll your shoulders, look at something far away.".into(),
            Nudge::Pause => format!("Step away from the screen for {length_minutes} minutes."),
        }
    }

    /// The tray's name for it, as in "Pause pending" (SPEC §8).
    pub fn name(self) -> &'static str {
        match self {
            Nudge::Stand => "Stand/Move",
            Nudge::Pause => "Pause",
        }
    }
}

/// What the popup shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub nudge: Nudge,
    pub title: &'static str,
    pub line: String,
    /// The full length of the away countdown: the stand or pause length it fired with.
    pub length: Duration,
    /// Whether the Snooze button shows: not once the one snooze is used (SPEC §5.6).
    pub snooze: bool,
    /// Whether the riding Water line shows: when it rode on the occurrence's first show, and then
    /// on every show of it after that (SPEC §5.4, §5.7).
    pub water: bool,
}

/// How an occurrence was satisfied: the `by` of a `satisfied` log line (SPEC §10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatisfyCause {
    /// Away for the stand or pause length.
    Away,
    Lock,
    Suspend,
    DisplayOff,
}

impl SatisfyCause {
    pub const ALL: [SatisfyCause; 4] = [
        SatisfyCause::Away,
        SatisfyCause::Lock,
        SatisfyCause::Suspend,
        SatisfyCause::DisplayOff,
    ];
}

/// Why an occurrence ended skipped: the `by` of a `skipped` log line (SPEC §10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipCause {
    /// The user pressed Skip.
    User,
    /// Expired: the next occurrence of the same nudge fell due.
    Next,
    /// Expired: quiet time started (SPEC §7).
    Quiet,
    /// Expired: the user turned that nudge off in Settings.
    Settings,
    /// Expired: breakpoint quit (tray Quit, or Windows ending the session).
    Exit,
}

impl SkipCause {
    pub const ALL: [SkipCause; 5] = [
        SkipCause::User,
        SkipCause::Next,
        SkipCause::Quiet,
        SkipCause::Settings,
        SkipCause::Exit,
    ];
}

/// Something that happened to an occurrence; each becomes one event-log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Fired {
        nudge: Nudge,
        occ: u64,
        /// For a Stand/Move that joins a Pause in a combined nudge: the Pause's `occ`.
        into: Option<u64>,
    },
    Snoozed {
        nudge: Nudge,
        occ: u64,
    },
    Satisfied {
        nudge: Nudge,
        occ: u64,
        by: SatisfyCause,
    },
    Skipped {
        nudge: Nudge,
        occ: u64,
        by: SkipCause,
    },
    /// Water was shown, riding along or alone; it gets its own `occ`.
    Shown {
        occ: u64,
    },
}

/// What the platform must do after a call into the scheduler, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Log(Event),
    /// Show the popup with this content, replacing whatever is on screen, and play the Windows
    /// default notification sound (SPEC §5.7): a first show, a post-snooze re-prompt or a Firm
    /// repeat.
    Show(Prompt),
    /// Hide the popup: its occurrence ended, or was snoozed.
    Close,
    /// Show Water alone: a small silent popup titled [`WATER_TITLE`], with no buttons, that
    /// hides itself after [`WATER_ALONE_FOR`].
    ShowWater,
}

/// Where an occurrence stands with its one snooze (SPEC §5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snooze {
    Unused,
    /// Snoozed, with this much active time left before the re-prompt.
    Running {
        left: Duration,
    },
    /// Snoozed and re-prompted: Skip only from now on.
    Used,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occurrence {
    pub nudge: Nudge,
    pub occ: u64,
    /// The stand or pause length it fired with, kept even if Settings change it (SPEC §5.7).
    pub length_minutes: u32,
    pub length: Duration,
    /// The enforcement level it fired with, kept even if Settings change it (SPEC §5.7).
    pub level: Enforcement,
    pub snooze: Snooze,
    /// Whether a Water line rode on its popup, which a re-prompt or Firm repeat shows again.
    pub water: bool,
}

/// One nudge's numbers and state.
#[derive(Debug)]
struct Clock {
    nudge: Nudge,
    /// Off: the clock never fires.
    on: bool,
    every: Duration,
    length_minutes: u32,
    length: Duration,
    level: Enforcement,
    /// Active time since this clock last restarted.
    elapsed: Duration,
    pending: Option<Occurrence>,
}

impl Clock {
    /// Whether it is on and due within `within` from now.
    fn due(&self, within: Duration) -> bool {
        self.on && self.elapsed + within >= self.every
    }
}

/// Water's numbers and state. It has no occurrence to keep: a due Water waits for a popup to
/// ride on, and its clock restarts only when it is shown.
#[derive(Debug)]
struct WaterClock {
    on: bool,
    every: Duration,
    /// Active time since Water was last shown. Never reset by presence (SPEC §5.2).
    elapsed: Duration,
}

impl WaterClock {
    /// Due and waiting to be shown. A second due while waiting changes nothing.
    fn due(&self) -> bool {
        self.on && self.elapsed >= self.every
    }
}

#[derive(Debug)]
pub struct Scheduler {
    minute: Duration,
    /// Pause first: it is the one in front of a combined nudge, and fires first so that a
    /// Stand/Move firing with it can join it.
    clocks: [Clock; 2],
    water: WaterClock,
    /// The time and presence of the last call, which decide whether the time since was active.
    last: Option<(Duration, Presence)>,
    /// Active time since the popup was last shown, which times a Firm repeat.
    since_show: Duration,
    next_occ: u64,
    /// Quiet time (SPEC §7): nothing fires and no clock runs.
    quiet: bool,
}

impl Scheduler {
    /// `nudges` are the settings from config. `minute` is how long one of their minutes is:
    /// 60 s, except under the debug-build `BREAKPOINT_MINUTE_SECS` dev aid.
    pub fn new(nudges: Nudges, minute: Duration) -> Self {
        let clock = |nudge, settings: AwayNudge| Clock {
            nudge,
            on: settings.on,
            every: minute * settings.every_minutes,
            length_minutes: settings.length_minutes,
            length: minute * settings.length_minutes,
            level: settings.enforcement,
            elapsed: Duration::ZERO,
            pending: None,
        };
        Self {
            minute,
            clocks: [
                clock(Nudge::Pause, nudges.pause),
                clock(Nudge::Stand, nudges.stand),
            ],
            water: WaterClock {
                on: nudges.water.on,
                every: minute * nudges.water.every_minutes,
                elapsed: Duration::ZERO,
            },
            last: None,
            since_show: Duration::ZERO,
            next_occ: 1,
            quiet: false,
        }
    }

    /// Continues the occurrence ids after `last`, the highest `occ` in the newest log file, so
    /// they keep increasing across the whole log (SPEC §10.2). Call before anything fires.
    pub fn continue_from(&mut self, last: u64) {
        self.next_occ = last + 1;
    }

    /// Whether it is quiet time now, as last told by [`Scheduler::set_quiet`].
    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Tells the scheduler whether `now` is quiet time (SPEC §7). Only a change does anything.
    /// Starting quiet expires every pending occurrence (cause `quiet`) and drops a waiting Water;
    /// while quiet no clock runs and nothing fires. Leaving quiet restarts every clock from zero,
    /// Water's included: a fresh focus period.
    pub fn set_quiet(&mut self, now: Duration, quiet: bool) -> Vec<Effect> {
        if quiet == self.quiet {
            return Vec::new();
        }
        if quiet {
            let effects = self.end(now, SkipCause::Quiet);
            self.drop_water();
            self.quiet = true;
            effects
        } else {
            self.advance(now);
            self.quiet = false;
            for clock in &mut self.clocks {
                clock.elapsed = Duration::ZERO;
            }
            self.water.elapsed = Duration::ZERO;
            self.since_show = Duration::ZERO;
            Vec::new()
        }
    }

    /// The pending occurrence of `nudge`, if any.
    pub fn pending(&self, nudge: Nudge) -> Option<Occurrence> {
        self.clock(nudge).pending
    }

    /// What the popup and the tray offer while anything is pending. A Stand/Move and a Pause
    /// pending together are one combined nudge, the Pause absorbing (SPEC §5.3): its title, line
    /// and countdown, with Snooze only if neither part is snoozed. With the Water line if it
    /// rode on this popup when it first showed: a Water that falls due later never joins it.
    pub fn prompt(&self) -> Option<Prompt> {
        let front = self.pendings().next()?;
        Some(Prompt {
            nudge: front.nudge,
            title: front.nudge.title(),
            line: front.nudge.line(front.length_minutes),
            length: front.length,
            snooze: self.pendings().all(|o| o.snooze == Snooze::Unused),
            water: self.pendings().any(|o| o.water),
        })
    }

    /// Sets the enforcement level of `nudge` from Settings. It applies from the next occurrence:
    /// a pending one keeps the level it fired with (SPEC §5.7).
    pub fn set_enforcement(&mut self, nudge: Nudge, level: Enforcement) {
        self.clock_mut(nudge).level = level;
    }

    /// Drops a due Water that is waiting for a popup, restarting its clock. Quiet start does
    /// this (SPEC §5.4).
    pub fn drop_water(&mut self) {
        if self.water.due() {
            self.water.elapsed = Duration::ZERO;
        }
    }

    /// A presence poll at `now`. The time since the previous call counts as active if the
    /// presence then did, so the silence before `Away` is declared stays active (SPEC §4).
    pub fn poll(&mut self, now: Duration, presence: Presence) -> Vec<Effect> {
        self.advance(now);
        self.last = Some((now, presence));
        if self.quiet {
            return Vec::new();
        }
        let front = self.prompt().map(|p| p.nudge);
        let mut effects = Vec::new();
        let mut show = false;

        if let Presence::Away { since, reason } = presence {
            let away = now.saturating_sub(since);
            for clock in &mut self.clocks {
                // Satisfaction (SPEC §5.5): a lock at once, otherwise away for the length the
                // occurrence fired with, in one stretch from the last input.
                if let Some(occ) = clock.pending {
                    let by = match reason {
                        AwayReason::Lock => Some(SatisfyCause::Lock),
                        AwayReason::Suspend => Some(SatisfyCause::Suspend),
                        AwayReason::DisplayOff => Some(SatisfyCause::DisplayOff),
                        AwayReason::NoInput | AwayReason::UserInactive if away >= occ.length => {
                            Some(SatisfyCause::Away)
                        }
                        AwayReason::NoInput | AwayReason::UserInactive => None,
                    };
                    if let Some(by) = by {
                        clock.pending = None;
                        effects.push(Effect::Log(Event::Satisfied {
                            nudge: occ.nudge,
                            occ: occ.occ,
                            by,
                        }));
                        // A satisfied occurrence restarts its own clock (SPEC §5.2).
                        clock.elapsed = Duration::ZERO;
                    }
                }
                // Presence resets (SPEC §5.2): away for the length, or a lock for Stand/Move.
                let lock_resets = clock.nudge == Nudge::Stand && reason.is_lock();
                if away >= clock.length || lock_resets {
                    clock.elapsed = Duration::ZERO;
                }
            }
        }

        // The snooze ran out: prompt once more, Skip only (SPEC §5.6).
        for clock in &mut self.clocks {
            if let Some(occ) = &mut clock.pending
                && occ.snooze
                    == (Snooze::Running {
                        left: Duration::ZERO,
                    })
            {
                occ.snooze = Snooze::Used;
                show = true;
            }
        }

        // Firing (SPEC §5.1). The first of the two to fall due brings the other along if it is
        // due within the combined-nudge window (SPEC §5.3).
        let fired = if self.clocks.iter().any(|c| c.due(Duration::ZERO)) {
            let window = self.minute * COMBINE_MINUTES;
            self.clocks.each_ref().map(|c| c.due(window))
        } else {
            [false; 2]
        };
        for i in (0..fired.len()).filter(|&i| fired[i]) {
            self.fire(i, &mut effects);
        }
        // One part fired and joined the other, pending from before: if that one is snoozed, the
        // combined nudge is snoozed (SPEC §5.3), so the new part takes on its snooze.
        if let [true, false] | [false, true] = fired {
            let (new, old) = if fired[0] { (0, 1) } else { (1, 0) };
            if let Some(old) = self.clocks[old].pending
                && let Some(new) = &mut self.clocks[new].pending
            {
                new.snooze = old.snooze;
            }
        }
        let fired = fired.contains(&true);

        // Firm: the popup comes back every few active minutes while pending, but not while a
        // snooze runs (SPEC §5.7). A combined nudge is Firm if either part is (SPEC §5.3).
        let firm = self.pendings().any(|o| o.level == Enforcement::Firm);
        let snoozing = self
            .pendings()
            .any(|o| matches!(o.snooze, Snooze::Running { .. }));
        let repeat = firm && !snoozing && self.since_show >= self.minute * FIRM_REPEAT_MINUTES;

        // A due Water rides on the popup of a newly fired occurrence only (SPEC §5.4). Every
        // part of that popup keeps the line for its later shows, logged as shown once.
        if fired && self.water.due() {
            self.show_water(&mut effects);
            for clock in &mut self.clocks {
                if let Some(occ) = &mut clock.pending {
                    occ.water = true;
                }
            }
        }

        match self.prompt() {
            Some(prompt) if show || fired || repeat || front != Some(prompt.nudge) => {
                self.since_show = Duration::ZERO;
                effects.push(Effect::Show(prompt))
            }
            None if front.is_some() => effects.push(Effect::Close),
            _ => {}
        }

        // With Stand/Move and Pause both off, a due Water shows alone at once (SPEC §5.4).
        if self.water.due() && self.clocks.iter().all(|c| !c.on) {
            self.show_water(&mut effects);
            effects.push(Effect::ShowWater);
        }
        effects
    }

    /// The user pressed Snooze: every pending occurrence comes back in [`SNOOZE_MINUTES`]
    /// active minutes. Only once; after that the prompt has Skip only (SPEC §5.6).
    pub fn snooze(&mut self, now: Duration) -> Vec<Effect> {
        self.advance(now);
        if !self.prompt().is_some_and(|p| p.snooze) {
            return Vec::new();
        }
        let left = self.minute * SNOOZE_MINUTES;
        let mut effects = Vec::new();
        for clock in &mut self.clocks {
            if let Some(occ) = &mut clock.pending {
                occ.snooze = Snooze::Running { left };
                effects.push(Effect::Log(Event::Snoozed {
                    nudge: occ.nudge,
                    occ: occ.occ,
                }));
            }
        }
        effects.push(Effect::Close);
        effects
    }

    /// The user pressed Skip. Only the pending occurrences end; their clocks keep running, so
    /// the next comes one cadence after this one fired (SPEC §5.5).
    pub fn skip(&mut self, now: Duration) -> Vec<Effect> {
        self.end(now, SkipCause::User)
    }

    /// breakpoint is quitting: the pending occurrences expire.
    pub fn quit(&mut self, now: Duration) -> Vec<Effect> {
        self.end(now, SkipCause::Exit)
    }

    fn end(&mut self, now: Duration, by: SkipCause) -> Vec<Effect> {
        self.advance(now);
        let mut effects: Vec<Effect> = self
            .clocks
            .iter_mut()
            .filter_map(|clock| clock.pending.take())
            .map(|occ| Effect::Log(skipped(occ, by)))
            .collect();
        if !effects.is_empty() {
            effects.push(Effect::Close);
        }
        effects
    }

    /// Fires the nudge of `self.clocks[i]`, expiring its pending occurrence (SPEC §5.1). A
    /// Stand/Move that fires while a Pause is pending joins it (SPEC §5.3).
    fn fire(&mut self, i: usize, effects: &mut Vec<Effect>) {
        let occ = self.next_occ;
        self.next_occ += 1;
        let into = match self.clocks[i].nudge {
            Nudge::Stand => self.pending(Nudge::Pause).map(|p| p.occ),
            Nudge::Pause => None,
        };
        let clock = &mut self.clocks[i];
        clock.elapsed = Duration::ZERO;
        if let Some(old) = clock.pending.take() {
            effects.push(Effect::Log(skipped(old, SkipCause::Next)));
        }
        clock.pending = Some(Occurrence {
            nudge: clock.nudge,
            occ,
            length_minutes: clock.length_minutes,
            length: clock.length,
            level: clock.level,
            snooze: Snooze::Unused,
            water: false,
        });
        effects.push(Effect::Log(Event::Fired {
            nudge: clock.nudge,
            occ,
            into,
        }));
    }

    /// Logs Water as shown and restarts its clock (SPEC §5.4).
    fn show_water(&mut self, effects: &mut Vec<Effect>) {
        effects.push(Effect::Log(Event::Shown { occ: self.next_occ }));
        self.next_occ += 1;
        self.water.elapsed = Duration::ZERO;
    }

    /// Adds the time since the last call to the clocks, Water's included, and snoozes if it
    /// was active time. Quiet time is never active time (SPEC §4).
    fn advance(&mut self, now: Duration) {
        if let Some((then, presence)) = self.last {
            if presence.counts_as_active() && !self.quiet {
                let active = now.saturating_sub(then);
                for clock in &mut self.clocks {
                    clock.elapsed += active;
                    if let Some(Occurrence {
                        snooze: Snooze::Running { left },
                        ..
                    }) = &mut clock.pending
                    {
                        *left = left.saturating_sub(active);
                    }
                }
                self.water.elapsed += active;
                self.since_show += active;
            }
            self.last = Some((now, presence));
        }
    }

    fn clock(&self, nudge: Nudge) -> &Clock {
        self.clocks
            .iter()
            .find(|c| c.nudge == nudge)
            .expect("every nudge has a clock")
    }

    fn clock_mut(&mut self, nudge: Nudge) -> &mut Clock {
        self.clocks
            .iter_mut()
            .find(|c| c.nudge == nudge)
            .expect("every nudge has a clock")
    }

    /// The pending occurrences, the one in front first.
    fn pendings(&self) -> impl Iterator<Item = Occurrence> + '_ {
        self.clocks.iter().filter_map(|c| c.pending)
    }
}

/// The popup's live countdown (SPEC §6): how much longer the user must stay away, measured from
/// the last input, for an occurrence of `length` to count. Held at the full length while a veto
/// explains the silence (`held`); it restarts on any input because `last_input` moves.
pub fn away_left(length: Duration, now: Duration, last_input: Duration, held: bool) -> Duration {
    if held {
        length
    } else {
        length.saturating_sub(now.saturating_sub(last_input))
    }
}

/// The countdown line, e.g. "Away 1:59 more and it counts", in whole seconds rounded up.
pub fn countdown_text(left: Duration) -> String {
    let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    format!("Away {}:{:02} more and it counts", secs / 60, secs % 60)
}

fn skipped(occ: Occurrence, by: SkipCause) -> Event {
    Event::Skipped {
        nudge: occ.nudge,
        occ: occ.occ,
        by,
    }
}
