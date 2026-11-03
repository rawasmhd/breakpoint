//! The presence state machine, fed by polls and events that the platform layer supplies.
//!
//! Times are offsets on one monotonic timeline whose origin the platform picks (start-up).
//! So far only the poll: vetoes (`AtDesk`), events and `Unknown` come with the presence issue;
//! [`AwayReason`] already names the lock-type reasons the scheduler needs (SPEC §5.2, §5.5).

use std::time::Duration;

/// How often the platform samples the last input (SPEC §4). Hard-coded, not a setting.
pub const POLL_EVERY: Duration = Duration::from_secs(5);

/// No input for this long is `Away` (SPEC §4). Hard-coded, not a setting.
pub const AWAY_AFTER: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Input within the last [`AWAY_AFTER`].
    Active,
    /// Not at the computer since `since`, the time of the last input (not when it was noticed).
    Away { since: Duration, reason: AwayReason },
}

/// Why the user is `Away`: the `by` a satisfied occurrence is logged with (SPEC §4, §10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwayReason {
    /// No input for [`AWAY_AFTER`]; satisfies only after the stand or pause length.
    NoInput,
    /// Session lock or disconnect.
    Lock,
    Suspend,
    DisplayOff,
}

impl AwayReason {
    /// Whether this counts as a lock: it satisfies a pending occurrence at once and restarts the
    /// Stand/Move clock (SPEC §5.2, §5.5).
    pub fn is_lock(self) -> bool {
        !matches!(self, AwayReason::NoInput)
    }
}

impl Presence {
    /// Classifies one poll: `now` and the last input, both on the core's timeline.
    pub fn from_last_input(now: Duration, last_input: Duration) -> Self {
        if now.saturating_sub(last_input) < AWAY_AFTER {
            Presence::Active
        } else {
            Presence::Away {
                since: last_input,
                reason: AwayReason::NoInput,
            }
        }
    }

    /// Whether time spent in this state is active time and advances the cadences.
    pub fn counts_as_active(self) -> bool {
        match self {
            Presence::Active => true,
            Presence::Away { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn s(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn input_within_90_s_is_active() {
        assert_eq!(
            Presence::from_last_input(s(1000), s(1000)),
            Presence::Active
        );
        assert_eq!(Presence::from_last_input(s(1000), s(911)), Presence::Active);
    }

    #[test]
    fn ninety_s_without_input_is_away_since_the_last_input() {
        assert_eq!(
            Presence::from_last_input(s(1000), s(910)),
            Presence::Away {
                since: s(910),
                reason: AwayReason::NoInput
            }
        );
        assert_eq!(
            Presence::from_last_input(s(5000), s(910)),
            Presence::Away {
                since: s(910),
                reason: AwayReason::NoInput
            }
        );
    }
}
