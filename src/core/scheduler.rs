//! Clocks in active time, firing, combining, Water riding along, snooze, Firm repeats and expiry.
//!
//! A pure state machine: the platform feeds it the time (an offset on one monotonic timeline),
//! presence polls and user actions, and carries out the [`Effect`]s it returns. So far Stand/Move
//! and Pause with satisfaction, snooze and expiry, with their cadences, lengths and on/off from
//! config. Combining and Water come with their own issue; until then two pending occurrences
//! share the popup with Pause in front, and Snooze and Skip act on both.

use std::time::Duration;

use super::config::{AwayNudge, Nudges};
use super::presence::{AwayReason, Presence};

/// How far a Snooze pushes an occurrence, in active minutes (SPEC §5.6). Fixed, not a setting.
pub const SNOOZE_MINUTES: u32 = 5;

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

/// Why an occurrence ended skipped: the `by` of a `skipped` log line (SPEC §10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipCause {
    /// The user pressed Skip.
    User,
    /// Expired: the next occurrence of the same nudge fell due.
    Next,
    /// Expired: breakpoint quit.
    Exit,
}

/// Something that happened to an occurrence; each becomes one event-log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Fired {
        nudge: Nudge,
        occ: u64,
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
}

/// What the platform must do after a call into the scheduler, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Log(Event),
    /// Show the popup with this content, replacing whatever is on screen.
    Show(Prompt),
    /// Hide the popup: its occurrence ended, or was snoozed.
    Close,
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
    pub snooze: Snooze,
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
    /// Active time since this clock last restarted.
    elapsed: Duration,
    pending: Option<Occurrence>,
}

#[derive(Debug)]
pub struct Scheduler {
    minute: Duration,
    /// Pause first: it is the one in front when both are pending, and fires first.
    clocks: [Clock; 2],
    /// The time and presence of the last call, which decide whether the time since was active.
    last: Option<(Duration, Presence)>,
    next_occ: u64,
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
            elapsed: Duration::ZERO,
            pending: None,
        };
        Self {
            minute,
            clocks: [
                clock(Nudge::Pause, nudges.pause),
                clock(Nudge::Stand, nudges.stand),
            ],
            last: None,
            next_occ: 1,
        }
    }

    /// The pending occurrence of `nudge`, if any.
    pub fn pending(&self, nudge: Nudge) -> Option<Occurrence> {
        self.clock(nudge).pending
    }

    /// What the popup and the tray offer while anything is pending: the occurrence in front
    /// (Pause over Stand/Move), with Snooze only if nothing pending has used its snooze.
    pub fn prompt(&self) -> Option<Prompt> {
        let front = self.pendings().next()?;
        Some(Prompt {
            nudge: front.nudge,
            title: front.nudge.title(),
            line: front.nudge.line(front.length_minutes),
            length: front.length,
            snooze: self.pendings().all(|o| o.snooze == Snooze::Unused),
        })
    }

    /// A presence poll at `now`. The time since the previous call counts as active if the
    /// presence then did, so the silence before `Away` is declared stays active (SPEC §4).
    pub fn poll(&mut self, now: Duration, presence: Presence) -> Vec<Effect> {
        self.advance(now);
        self.last = Some((now, presence));
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

        for i in 0..self.clocks.len() {
            if self.clocks[i].on && self.clocks[i].elapsed >= self.clocks[i].every {
                self.fire(i, &mut effects);
                show = true;
            }
        }

        let after = self.prompt();
        match after {
            Some(prompt) if show || front != Some(prompt.nudge) => {
                effects.push(Effect::Show(prompt))
            }
            None if front.is_some() => effects.push(Effect::Close),
            _ => {}
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

    /// Fires the nudge of `self.clocks[i]`, expiring its pending occurrence (SPEC §5.1).
    fn fire(&mut self, i: usize, effects: &mut Vec<Effect>) {
        let occ = self.next_occ;
        self.next_occ += 1;
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
            snooze: Snooze::Unused,
        });
        effects.push(Effect::Log(Event::Fired {
            nudge: clock.nudge,
            occ,
        }));
    }

    /// Adds the time since the last call to the clocks and snoozes if it was active time.
    fn advance(&mut self, now: Duration) {
        if let Some((then, presence)) = self.last {
            if presence.counts_as_active() {
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
