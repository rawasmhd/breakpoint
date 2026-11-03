//! Clocks in active time, firing, combining, Water riding along, snooze, Firm repeats and expiry.
//!
//! A pure state machine: the platform feeds it the time (an offset on one monotonic timeline),
//! presence polls and user actions, and carries out the [`Effect`]s it returns. So far only
//! Stand/Move, with the Balanced cadence hard-coded until config lands.

use std::time::Duration;

use super::presence::Presence;

/// Balanced's Stand/Move cadence (SPEC §3), in active minutes.
pub const STAND_EVERY_MINUTES: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nudge {
    Stand,
}

impl Nudge {
    /// The popup content for a newly fired occurrence of this nudge (SPEC §3, nudge text).
    pub fn prompt(self) -> Prompt {
        match self {
            Nudge::Stand => Prompt {
                title: "Stand up and move",
                line: "Stand, roll your shoulders, look at something far away.",
            },
        }
    }
}

/// What the popup shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prompt {
    pub title: &'static str,
    pub line: &'static str,
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
    Skipped {
        nudge: Nudge,
        occ: u64,
        by: SkipCause,
    },
}

/// What the platform must do after a call into the scheduler, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Log(Event),
    /// Show the popup with this content, replacing whatever is on screen.
    Show(Prompt),
    /// Close the popup: its occurrence ended.
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occurrence {
    pub nudge: Nudge,
    pub occ: u64,
}

#[derive(Debug)]
pub struct Scheduler {
    stand_every: Duration,
    /// Active time since the Stand/Move clock last restarted.
    stand_clock: Duration,
    /// The time and presence of the last call, which decide whether the time since was active.
    last: Option<(Duration, Presence)>,
    pending: Option<Occurrence>,
    next_occ: u64,
}

impl Scheduler {
    /// `minute` is how long one minute of the cadences is: 60 s, except under the debug-build
    /// `BREAKPOINT_MINUTE_SECS` dev aid.
    pub fn new(minute: Duration) -> Self {
        Self {
            stand_every: minute * STAND_EVERY_MINUTES,
            stand_clock: Duration::ZERO,
            last: None,
            pending: None,
            next_occ: 1,
        }
    }

    /// The occurrence waiting to be satisfied or skipped, if any.
    pub fn pending(&self) -> Option<Occurrence> {
        self.pending
    }

    /// A presence poll at `now`. The time since the previous call counts as active if the
    /// presence then did, so the silence before `Away` is declared stays active (SPEC §4).
    pub fn poll(&mut self, now: Duration, presence: Presence) -> Vec<Effect> {
        self.advance(now);
        self.last = Some((now, presence));
        let mut effects = Vec::new();
        if self.stand_clock >= self.stand_every {
            self.stand_clock = Duration::ZERO;
            if let Some(old) = self.pending.take() {
                effects.push(Effect::Log(skipped(old, SkipCause::Next)));
            }
            let occ = Occurrence {
                nudge: Nudge::Stand,
                occ: self.next_occ,
            };
            self.next_occ += 1;
            self.pending = Some(occ);
            effects.push(Effect::Log(Event::Fired {
                nudge: occ.nudge,
                occ: occ.occ,
            }));
            effects.push(Effect::Show(occ.nudge.prompt()));
        }
        effects
    }

    /// The user pressed Skip. Only the pending occurrence ends; its clock keeps running, so the
    /// next one comes one cadence after this one fired (SPEC §5.5).
    pub fn skip(&mut self, now: Duration) -> Vec<Effect> {
        self.end(now, SkipCause::User)
    }

    /// breakpoint is quitting: the pending occurrence expires.
    pub fn quit(&mut self, now: Duration) -> Vec<Effect> {
        self.end(now, SkipCause::Exit)
    }

    fn end(&mut self, now: Duration, by: SkipCause) -> Vec<Effect> {
        self.advance(now);
        match self.pending.take() {
            Some(occ) => vec![Effect::Log(skipped(occ, by)), Effect::Close],
            None => Vec::new(),
        }
    }

    /// Adds the time since the last call to the clocks if it was active time.
    fn advance(&mut self, now: Duration) {
        if let Some((then, presence)) = self.last {
            if presence.counts_as_active() {
                self.stand_clock += now.saturating_sub(then);
            }
            self.last = Some((now, presence));
        }
    }
}

fn skipped(occ: Occurrence, by: SkipCause) -> Event {
    Event::Skipped {
        nudge: occ.nudge,
        occ: occ.occ,
        by,
    }
}
