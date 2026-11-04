//! The presence state machine, fed by polls and events that the platform layer supplies.
//!
//! Times are offsets on one monotonic timeline whose origin the platform picks (start-up).
//! [`Tracker`] holds what the session and power events said; each poll combines that with the
//! last input and, only when the silence is long enough to matter, the vetoes (SPEC §4).
//!
//! SPEC §4 lets the vetoes be checked once input has stopped for 10 s. They only change the
//! verdict at [`AWAY_AFTER`], so that is when the tracker asks, which keeps within the rule.

use std::time::Duration;

/// How often the platform samples the last input (SPEC §4). Hard-coded, not a setting.
pub const POLL_EVERY: Duration = Duration::from_secs(5);

/// No input for this long is `Away` (SPEC §4). Hard-coded, not a setting.
pub const AWAY_AFTER: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Input within the last [`AWAY_AFTER`], or an event that says the user is back.
    Active,
    /// No input for [`AWAY_AFTER`], but a veto explains the silence: present, not typing.
    AtDesk,
    /// Not at the computer since `since`, the time of the last input (not when it was noticed).
    Away { since: Duration, reason: AwayReason },
    /// The machine woke itself and nobody has touched it since.
    Unknown,
}

/// Why the user is `Away`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwayReason {
    /// No input for [`AWAY_AFTER`] and no veto.
    NoInput,
    /// Windows' own user-inactive verdict (`GUID_SESSION_USER_PRESENCE`).
    UserInactive,
    /// The session was locked or disconnected (console or remote, fast user switching too).
    Lock,
    /// The machine is suspending.
    Suspend,
    /// The display was turned off (`GUID_SESSION_DISPLAY_STATUS`).
    DisplayOff,
}

impl AwayReason {
    /// Whether this counts as a lock (SPEC §5.2, §5.5): it satisfies a pending occurrence at
    /// once and restarts the Stand/Move clock. The others satisfy only after the length.
    pub fn is_lock(self) -> bool {
        match self {
            AwayReason::Lock | AwayReason::Suspend | AwayReason::DisplayOff => true,
            AwayReason::NoInput | AwayReason::UserInactive => false,
        }
    }

    /// The `by` of a `satisfied` log line (SPEC §5.5).
    pub fn logged_as(self) -> &'static str {
        match self {
            AwayReason::NoInput | AwayReason::UserInactive => "away",
            AwayReason::Lock => "lock",
            AwayReason::Suspend => "suspend",
            AwayReason::DisplayOff => "display_off",
        }
    }
}

impl Presence {
    /// Classifies one poll on input alone, with no vetoes and no events: `now` and the last
    /// input, both on the core's timeline.
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
            Presence::Active | Presence::AtDesk => true,
            Presence::Away { .. } | Presence::Unknown => false,
        }
    }

    /// Whether this is an `Away` that counts as a lock (see [`AwayReason::is_lock`]).
    pub fn is_lock(self) -> bool {
        matches!(self, Presence::Away { reason, .. } if reason.is_lock())
    }
}

/// A session or power event (SPEC §4). Events win over the poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Lock,
    Unlock,
    /// Console or remote disconnect, including fast user switching away.
    Disconnect,
    /// Console or remote connect.
    Connect,
    Suspend,
    /// A resume triggered by the user (`PBT_APMRESUMESUSPEND`).
    Resume,
    /// A resume on its own (`PBT_APMRESUMEAUTOMATIC`): the machine may have woken itself.
    ResumeAutomatic,
    DisplayOff,
    DisplayOn,
    /// Windows decided the user is inactive.
    UserInactive,
    /// Windows saw the user again.
    UserPresent,
}

/// What explains a long silence, asked for by [`Tracker::poll`] only once input has stopped
/// for [`AWAY_AFTER`]. The platform answers from the shell, the power manager and the media
/// controls.
pub trait Vetoes {
    /// Full screen, busy or presentation mode (`SHQueryUserNotificationState`).
    fn shell_busy(&mut self) -> bool;
    /// Some app holds the display on (`ES_DISPLAY_REQUIRED`).
    fn display_required(&mut self) -> bool;
    /// Media is playing; `None` when the media-controls call failed.
    fn media_playing(&mut self) -> Option<bool>;
}

/// The presence state machine: remembers what events said and turns each poll into a
/// [`Presence`].
#[derive(Debug)]
pub struct Tracker {
    /// Locked or disconnected since. Only unlock or connect ends it: input on the lock screen
    /// does not.
    locked: Option<Duration>,
    /// Suspending since. A resume or later input ends it.
    asleep: Option<Duration>,
    /// Display off since. Display on or later input ends it.
    display_off: Option<Duration>,
    /// Windows' user-inactive verdict, with `since` at the last input. User-present or later
    /// input ends it.
    inactive: Option<Duration>,
    /// The machine resumed by itself at this time and nobody has touched it since.
    unattended: Option<Duration>,
    /// A user resume arrived since the last suspend. Windows documents it after the automatic
    /// resume, but has been seen sending it first, so either order counts as one human wake.
    woken_by_user: bool,
    /// Input older than this is ignored: set by every event that means the user is here, and
    /// by any resume, so a measurement from before does not count (SPEC §12.3).
    rearmed: Duration,
    /// False once the media-controls call has failed: no media veto for the rest of the session.
    media_veto: bool,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            locked: None,
            asleep: None,
            display_off: None,
            inactive: None,
            unattended: None,
            woken_by_user: false,
            rearmed: Duration::ZERO,
            media_veto: true,
        }
    }

    /// A session or power event at `now`; `last_input` is the platform's reading at that moment.
    /// The next [`poll`](Self::poll) reports the result.
    pub fn signal(&mut self, now: Duration, last_input: Duration, signal: Signal) {
        let since = self.last_input(last_input).min(now);
        match signal {
            Signal::Lock | Signal::Disconnect => {
                self.locked.get_or_insert(since);
            }
            Signal::Suspend => {
                self.asleep.get_or_insert(since);
                self.woken_by_user = false;
            }
            Signal::DisplayOff => {
                self.display_off.get_or_insert(since);
            }
            Signal::UserInactive => {
                self.inactive.get_or_insert(since);
            }
            Signal::Unlock | Signal::Connect => {
                self.locked = None;
                self.back(now);
            }
            Signal::Resume => {
                self.asleep = None;
                self.woken_by_user = true;
                self.back(now);
            }
            Signal::ResumeAutomatic => {
                self.asleep = None;
                // When a human woke the machine a user resume comes with this one; until it
                // does, nobody is known to be here.
                if !self.woken_by_user {
                    self.unattended.get_or_insert(now);
                }
                self.rearmed = self.rearmed.max(now);
            }
            Signal::DisplayOn => {
                self.display_off = None;
                self.back(now);
            }
            Signal::UserPresent => {
                self.inactive = None;
                self.back(now);
            }
        }
    }

    /// One poll at `now` with the platform's last input. `vetoes` is asked only when the
    /// answer can change the verdict, and the media veto only until its first failure.
    pub fn poll(
        &mut self,
        now: Duration,
        last_input: Duration,
        vetoes: &mut impl Vetoes,
    ) -> Presence {
        let last_input = self.last_input(last_input);
        // Input after an event ends the states that input can end.
        for since in [&mut self.asleep, &mut self.display_off, &mut self.inactive] {
            if since.is_some_and(|since| last_input > since) {
                *since = None;
            }
        }
        if self.unattended.is_some_and(|at| last_input > at) {
            self.unattended = None;
        }

        let held = [
            (self.locked, AwayReason::Lock),
            (self.asleep, AwayReason::Suspend),
            (self.display_off, AwayReason::DisplayOff),
        ];
        if let Some((since, reason)) = held
            .into_iter()
            .find_map(|(since, reason)| Some((since?, reason)))
        {
            return Presence::Away { since, reason };
        }
        if self.unattended.is_some() {
            return Presence::Unknown;
        }
        if let Some(since) = self.inactive {
            return Presence::Away {
                since,
                reason: AwayReason::UserInactive,
            };
        }

        match Presence::from_last_input(now, last_input) {
            Presence::Away { .. } if self.vetoed(vetoes) => Presence::AtDesk,
            presence => presence,
        }
    }

    fn vetoed(&mut self, vetoes: &mut impl Vetoes) -> bool {
        if vetoes.display_required() || vetoes.shell_busy() {
            return true;
        }
        if !self.media_veto {
            return false;
        }
        match vetoes.media_playing() {
            Some(playing) => playing,
            None => {
                self.media_veto = false;
                false
            }
        }
    }

    /// An event that means the user is here: they count as having just given input.
    fn back(&mut self, now: Duration) {
        self.unattended = None;
        self.rearmed = self.rearmed.max(now);
    }

    fn last_input(&self, last_input: Duration) -> Duration {
        last_input.max(self.rearmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn s(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    fn away(since: u64, reason: AwayReason) -> Presence {
        Presence::Away {
            since: s(since),
            reason,
        }
    }

    /// Vetoes with fixed answers that count how often each was asked.
    #[derive(Default)]
    struct Fake {
        shell: bool,
        display: bool,
        media: Option<bool>,
        asked: [u32; 3],
    }

    impl Fake {
        fn none() -> Self {
            Fake {
                media: Some(false),
                ..Fake::default()
            }
        }
    }

    impl Vetoes for Fake {
        fn shell_busy(&mut self) -> bool {
            self.asked[0] += 1;
            self.shell
        }
        fn display_required(&mut self) -> bool {
            self.asked[1] += 1;
            self.display
        }
        fn media_playing(&mut self) -> Option<bool> {
            self.asked[2] += 1;
            self.media
        }
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
            away(910, AwayReason::NoInput)
        );
        assert_eq!(
            Presence::from_last_input(s(5000), s(910)),
            away(910, AwayReason::NoInput)
        );
    }

    #[test]
    fn the_poll_alone_is_active_then_away_since_the_last_input() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        assert_eq!(t.poll(s(1000), s(1000), &mut v), Presence::Active);
        assert_eq!(t.poll(s(1089), s(1000), &mut v), Presence::Active);
        assert_eq!(
            t.poll(s(1090), s(1000), &mut v),
            away(1000, AwayReason::NoInput)
        );
        assert_eq!(t.poll(s(1095), s(1094), &mut v), Presence::Active);
    }

    #[test]
    fn vetoes_are_not_asked_before_the_silence_matters() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.poll(s(1005), s(1000), &mut v);
        t.poll(s(1015), s(1000), &mut v);
        t.poll(s(1085), s(1000), &mut v);
        assert_eq!(v.asked, [0, 0, 0]);
        t.poll(s(1090), s(1000), &mut v);
        assert_eq!(v.asked, [1, 1, 1]);
    }

    #[test]
    fn each_veto_makes_a_long_silence_at_desk() {
        for fake in [
            Fake {
                shell: true,
                ..Fake::none()
            },
            Fake {
                display: true,
                ..Fake::none()
            },
            Fake {
                media: Some(true),
                ..Fake::none()
            },
        ] {
            let mut v = fake;
            let mut t = Tracker::new();
            assert_eq!(t.poll(s(1180), s(1000), &mut v), Presence::AtDesk);
        }
    }

    #[test]
    fn at_desk_turns_away_since_the_last_input_when_the_veto_ends() {
        let mut t = Tracker::new();
        let mut v = Fake {
            media: Some(true),
            ..Fake::none()
        };
        assert_eq!(t.poll(s(1180), s(1000), &mut v), Presence::AtDesk);
        v.media = Some(false);
        assert_eq!(
            t.poll(s(1185), s(1000), &mut v),
            away(1000, AwayReason::NoInput)
        );
    }

    #[test]
    fn a_failed_media_call_drops_that_veto_for_the_session() {
        let mut t = Tracker::new();
        let mut v = Fake {
            media: None,
            ..Fake::none()
        };
        assert_eq!(
            t.poll(s(1090), s(1000), &mut v),
            away(1000, AwayReason::NoInput)
        );
        assert_eq!(v.asked[2], 1);
        // Even once the call would work, it is never asked again.
        v.media = Some(true);
        assert_eq!(
            t.poll(s(1095), s(1000), &mut v),
            away(1000, AwayReason::NoInput)
        );
        assert_eq!(v.asked[2], 1);
        // The other vetoes still work.
        v.shell = true;
        assert_eq!(t.poll(s(1100), s(1000), &mut v), Presence::AtDesk);
    }

    #[test]
    fn lock_is_away_at_once_since_the_last_input_and_counts_as_a_lock() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1010), s(1008), Signal::Lock);
        let p = t.poll(s(1010), s(1008), &mut v);
        assert_eq!(p, away(1008, AwayReason::Lock));
        assert!(p.is_lock());
        assert!(!p.counts_as_active());
    }

    #[test]
    fn input_on_the_lock_screen_does_not_end_the_lock() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1010), s(1008), Signal::Lock);
        assert_eq!(
            t.poll(s(1500), s(1499), &mut v),
            away(1008, AwayReason::Lock)
        );
    }

    #[test]
    fn unlock_is_active_even_without_input() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1010), s(1008), Signal::Lock);
        // Windows Hello face sign-in: no input at all.
        t.signal(s(2000), s(1008), Signal::Unlock);
        assert_eq!(t.poll(s(2000), s(1008), &mut v), Presence::Active);
        // Measured afresh from the unlock.
        assert_eq!(t.poll(s(2089), s(1008), &mut v), Presence::Active);
        assert_eq!(
            t.poll(s(2090), s(1008), &mut v),
            away(2000, AwayReason::NoInput)
        );
    }

    #[test]
    fn disconnect_is_a_lock_until_connect() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1010), s(1000), Signal::Disconnect);
        let p = t.poll(s(1015), s(1000), &mut v);
        assert_eq!(p, away(1000, AwayReason::Lock));
        assert!(p.is_lock());
        t.signal(s(3000), s(1000), Signal::Connect);
        assert_eq!(t.poll(s(3000), s(1000), &mut v), Presence::Active);
    }

    #[test]
    fn an_earlier_silence_keeps_its_since_when_a_lock_follows() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        assert_eq!(
            t.poll(s(1200), s(1000), &mut v),
            away(1000, AwayReason::NoInput)
        );
        t.signal(s(1300), s(1000), Signal::Lock);
        assert_eq!(
            t.poll(s(1300), s(1000), &mut v),
            away(1000, AwayReason::Lock)
        );
    }

    #[test]
    fn suspend_is_away_and_counts_as_a_lock() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1010), s(1009), Signal::Suspend);
        let p = t.poll(s(1010), s(1009), &mut v);
        assert_eq!(p, away(1009, AwayReason::Suspend));
        assert!(p.is_lock());
    }

    #[test]
    fn a_user_resume_is_active_and_rearms_the_idle_measurement() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1010), s(1009), Signal::Suspend);
        t.signal(s(5000), s(1009), Signal::ResumeAutomatic);
        t.signal(s(5000), s(1009), Signal::Resume);
        // The pre-sleep input reading is ignored: no instant Away after waking.
        assert_eq!(t.poll(s(5005), s(1009), &mut v), Presence::Active);
        assert_eq!(
            t.poll(s(5090), s(1009), &mut v),
            away(5000, AwayReason::NoInput)
        );
    }

    #[test]
    fn a_user_resume_before_the_automatic_one_is_still_active() {
        // Seen on a real wake: PBT_APMRESUMESUSPEND, then PBT_APMRESUMEAUTOMATIC.
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1010), s(1009), Signal::Suspend);
        t.signal(s(5000), s(1009), Signal::Resume);
        t.signal(s(5000), s(1009), Signal::ResumeAutomatic);
        assert_eq!(t.poll(s(5005), s(1009), &mut v), Presence::Active);
        // The next sleep starts afresh: an automatic wake alone is Unknown again.
        t.signal(s(6000), s(5900), Signal::Suspend);
        t.signal(s(9000), s(5900), Signal::ResumeAutomatic);
        assert_eq!(t.poll(s(9005), s(5900), &mut v), Presence::Unknown);
    }

    #[test]
    fn an_automatic_resume_alone_is_unknown_until_input() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1010), s(1009), Signal::Suspend);
        t.signal(s(5000), s(1009), Signal::ResumeAutomatic);
        assert_eq!(t.poll(s(5005), s(1009), &mut v), Presence::Unknown);
        // Still Unknown long after the 90 s: nobody is known to be here, nor known to be gone.
        assert_eq!(t.poll(s(5600), s(1009), &mut v), Presence::Unknown);
        assert!(!Presence::Unknown.counts_as_active());
        assert_eq!(v.asked, [0, 0, 0]);
        // Input arrives.
        assert_eq!(t.poll(s(5605), s(5603), &mut v), Presence::Active);
    }

    #[test]
    fn a_skewed_input_reading_after_resume_is_clamped_to_the_resume() {
        // The tick count and the core's clock can disagree across sleep; a reading that lands
        // before the resume is not input.
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1010), s(1009), Signal::Suspend);
        t.signal(s(5000), s(0), Signal::ResumeAutomatic);
        assert_eq!(t.poll(s(5005), s(0), &mut v), Presence::Unknown);
    }

    #[test]
    fn input_after_a_suspend_ends_it_if_the_resume_was_missed() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1010), s(1009), Signal::Suspend);
        assert_eq!(t.poll(s(5005), s(5004), &mut v), Presence::Active);
    }

    #[test]
    fn a_lock_held_across_sleep_stays_a_lock_after_waking() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1000), s(999), Signal::Lock);
        t.signal(s(1010), s(999), Signal::Suspend);
        t.signal(s(5000), s(999), Signal::ResumeAutomatic);
        t.signal(s(5000), s(999), Signal::Resume);
        assert_eq!(
            t.poll(s(5005), s(5004), &mut v),
            away(999, AwayReason::Lock)
        );
        t.signal(s(5020), s(5004), Signal::Unlock);
        assert_eq!(t.poll(s(5020), s(5004), &mut v), Presence::Active);
    }

    #[test]
    fn display_off_is_away_at_once_and_counts_as_a_lock() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1600), s(1000), Signal::DisplayOff);
        let p = t.poll(s(1600), s(1000), &mut v);
        assert_eq!(p, away(1000, AwayReason::DisplayOff));
        assert!(p.is_lock());
    }

    #[test]
    fn display_off_wins_over_a_veto() {
        let mut t = Tracker::new();
        let mut v = Fake {
            display: true,
            ..Fake::none()
        };
        t.signal(s(1600), s(1000), Signal::DisplayOff);
        assert_eq!(
            t.poll(s(1605), s(1000), &mut v),
            away(1000, AwayReason::DisplayOff)
        );
    }

    #[test]
    fn display_on_is_active() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1600), s(1000), Signal::DisplayOff);
        t.signal(s(2000), s(1000), Signal::DisplayOn);
        assert_eq!(t.poll(s(2000), s(1000), &mut v), Presence::Active);
    }

    #[test]
    fn input_ends_display_off_if_display_on_was_missed() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1600), s(1000), Signal::DisplayOff);
        assert_eq!(t.poll(s(2005), s(2003), &mut v), Presence::Active);
    }

    #[test]
    fn display_on_under_a_lock_is_still_the_lock() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1000), s(999), Signal::Lock);
        t.signal(s(1600), s(999), Signal::DisplayOff);
        t.signal(s(2000), s(999), Signal::DisplayOn);
        assert_eq!(
            t.poll(s(2000), s(1999), &mut v),
            away(999, AwayReason::Lock)
        );
    }

    #[test]
    fn user_inactive_is_away_since_the_last_input_like_no_input() {
        let mut t = Tracker::new();
        let mut v = Fake {
            media: Some(true),
            ..Fake::none()
        };
        // A podcast left playing vetoes the poll, but Windows' verdict wins.
        assert_eq!(t.poll(s(1100), s(1000), &mut v), Presence::AtDesk);
        t.signal(s(1300), s(1000), Signal::UserInactive);
        let p = t.poll(s(1300), s(1000), &mut v);
        assert_eq!(p, away(1000, AwayReason::UserInactive));
        assert!(!p.is_lock());
        assert_eq!(AwayReason::UserInactive.logged_as(), "away");
    }

    #[test]
    fn user_present_is_active() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1300), s(1000), Signal::UserInactive);
        t.signal(s(2000), s(1000), Signal::UserPresent);
        assert_eq!(t.poll(s(2000), s(1000), &mut v), Presence::Active);
    }

    #[test]
    fn input_ends_user_inactive() {
        let mut t = Tracker::new();
        let mut v = Fake::none();
        t.signal(s(1300), s(1000), Signal::UserInactive);
        assert_eq!(t.poll(s(2000), s(1999), &mut v), Presence::Active);
    }

    #[test]
    fn at_desk_and_active_are_active_time() {
        assert!(Presence::Active.counts_as_active());
        assert!(Presence::AtDesk.counts_as_active());
        assert!(!away(0, AwayReason::NoInput).counts_as_active());
    }

    #[test]
    fn reasons_map_to_lock_or_no_input_and_their_log_names() {
        use AwayReason::*;
        let table = [
            (NoInput, false, "away"),
            (UserInactive, false, "away"),
            (Lock, true, "lock"),
            (Suspend, true, "suspend"),
            (DisplayOff, true, "display_off"),
        ];
        for (reason, lock, logged) in table {
            assert_eq!(reason.is_lock(), lock, "{reason:?}");
            assert_eq!(reason.logged_as(), logged, "{reason:?}");
        }
    }
}
