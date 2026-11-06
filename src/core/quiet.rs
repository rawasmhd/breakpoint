//! The work week, Off, Overtime and the edges where quiet starts and ends (SPEC §7).
//!
//! Nudges fire only inside work hours on a work day; every other moment is quiet. The user can
//! turn breakpoint Off until the next work day's start, or on for Overtime until midnight. The
//! tray's top item, tooltip and icon come from one table, [`switch_item`].

use std::time::Duration;

use super::config::{Config, WorkDays, WorkHours};
use super::scheduler::{Effect, Scheduler};
use super::time::{LocalTime, TimeOfDay};

/// When nudges may fire: the work days and their hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkWeek {
    pub days: WorkDays,
    pub hours: WorkHours,
}

impl WorkWeek {
    pub fn from_config(config: &Config) -> Self {
        Self {
            days: config.work_days,
            hours: config.work_hours,
        }
    }

    /// Whether `now` is inside work hours on a work day: from the start, up to but not
    /// including the end.
    pub fn in_hours(&self, now: LocalTime) -> bool {
        let time = now.time_of_day();
        self.days.contains(now.weekday()) && self.hours.start <= time && time < self.hours.end
    }

    /// The first start of work hours on a work day after `now`. `None` only with no work day.
    pub fn next_start(&self, now: LocalTime) -> Option<LocalTime> {
        (0..=7)
            .map(|days| now.on_day_at(days, self.hours.start))
            .find(|start| start.unix > now.unix && self.days.contains(start.weekday()))
    }
}

/// The next local midnight after `now`, where Overtime ends.
pub fn midnight_after(now: LocalTime) -> LocalTime {
    now.on_day_at(1, TimeOfDay::new(0, 0).expect("00:00 is a time of day"))
}

/// Off or Overtime: the user's override of the work week, until its end moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// Quiet until the next work day's start, though it is work hours.
    Off { until: LocalTime },
    /// Nudging until midnight, though it is outside work hours.
    Overtime { until: LocalTime },
}

impl Switch {
    pub fn until(self) -> LocalTime {
        match self {
            Switch::Off { until } | Switch::Overtime { until } => until,
        }
    }
}

/// What pressing the tray's top item does (SPEC §7 table, "Pressing it").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    StartOff,
    /// Clocks restart: quiet ends.
    EndOff,
    /// Clocks restart: quiet ends.
    StartOvertime,
    EndOffStartOvertime,
    EndOvertime,
    EndOvertimeStartOff,
}

/// The tray's top item, its tooltip and its icon at one moment (SPEC §7, §8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchItem {
    /// What the top menu item reads.
    pub label: &'static str,
    pub press: Press,
    pub tooltip: String,
    /// The greyed icon, shown while quiet; the normal one otherwise.
    pub greyed: bool,
}

/// The SPEC §7 table: the top tray item, what pressing it does, the tooltip and the icon, for
/// the work week, the Off or Overtime in force (if any) and the time now. `switch` must be in
/// force at `now`, that is end after it.
pub fn switch_item(week: &WorkWeek, switch: Option<Switch>, now: LocalTime) -> SwitchItem {
    const TURN_OFF: &str = "Turn off until next work day";
    const TURN_ON: &str = "Turn on until midnight";
    const ON_UNTIL_MIDNIGHT: &str = "breakpoint: on until midnight";
    let in_hours = week.in_hours(now);
    let item = |label, press, tooltip: String, greyed| SwitchItem {
        label,
        press,
        tooltip,
        greyed,
    };
    let off_until = |until: LocalTime| format!("breakpoint: off until {}", day_and_time(until));
    match (switch, in_hours) {
        (None, true) => item(TURN_OFF, Press::StartOff, "breakpoint".into(), false),
        (Some(Switch::Off { until }), true) => {
            item("Turn back on", Press::EndOff, off_until(until), true)
        }
        (None, false) => {
            let tooltip = match week.next_start(now) {
                Some(start) => format!("breakpoint: quiet until {}", day_and_time(start)),
                None => "breakpoint: quiet".into(),
            };
            item(TURN_ON, Press::StartOvertime, tooltip, true)
        }
        (Some(Switch::Off { until }), false) => {
            item(TURN_ON, Press::EndOffStartOvertime, off_until(until), true)
        }
        (Some(Switch::Overtime { .. }), false) => item(
            "Turn off now",
            Press::EndOvertime,
            ON_UNTIL_MIDNIGHT.into(),
            false,
        ),
        (Some(Switch::Overtime { .. }), true) => item(
            TURN_OFF,
            Press::EndOvertimeStartOff,
            ON_UNTIL_MIDNIGHT.into(),
            false,
        ),
    }
}

/// Whether nudges are silent: Off, or outside work hours with no Overtime. `switch` must be in
/// force at `now`.
pub fn is_quiet(week: &WorkWeek, switch: Option<Switch>, now: LocalTime) -> bool {
    match switch {
        Some(Switch::Off { .. }) => true,
        Some(Switch::Overtime { .. }) => false,
        None => !week.in_hours(now),
    }
}

/// e.g. `Sunday 08:00`, on the moment's own local clock.
fn day_and_time(t: LocalTime) -> String {
    format!("{} {}", t.weekday().name(), t.time_of_day())
}

/// Everything that decides quiet: the work week and the Off or Overtime in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quiet {
    week: WorkWeek,
    switch: Option<Switch>,
}

impl Quiet {
    /// The work week from `config`, and its Off or Overtime end moment if one is set.
    pub fn new(config: &Config) -> Self {
        let switch = match (config.off_until, config.overtime_until) {
            (Some(until), None) => Some(Switch::Off { until }),
            (None, Some(until)) => Some(Switch::Overtime { until }),
            // Config never keeps both; if it did, neither would count (SPEC §10.1).
            _ => None,
        };
        Self {
            week: WorkWeek::from_config(config),
            switch,
        }
    }

    /// Replaces the work days and hours, from a Settings Save. An Off or Overtime in force keeps
    /// its end moment (SPEC §9.2). Call [`Quiet::sync`] next to apply the change at once.
    pub fn set_week(&mut self, week: WorkWeek) {
        self.week = week;
    }

    /// The Off or Overtime in force at `now`; one whose end moment has come is not.
    pub fn switch(&self, now: LocalTime) -> Option<Switch> {
        self.switch.filter(|s| s.until().unix > now.unix)
    }

    pub fn is_quiet(&self, now: LocalTime) -> bool {
        is_quiet(&self.week, self.switch(now), now)
    }

    /// The tray's top item, tooltip and icon at `now`.
    pub fn item(&self, now: LocalTime) -> SwitchItem {
        switch_item(&self.week, self.switch(now), now)
    }

    /// The user pressed the tray's top item at `now`. At most one of Off and Overtime is in
    /// force after it: starting one clears the other.
    pub fn press(&mut self, now: LocalTime) {
        self.switch = match self.item(now).press {
            Press::StartOff | Press::EndOvertimeStartOff => {
                self.week.next_start(now).map(|until| Switch::Off { until })
            }
            Press::StartOvertime | Press::EndOffStartOvertime => Some(Switch::Overtime {
                until: midnight_after(now),
            }),
            Press::EndOff | Press::EndOvertime => None,
        };
    }

    /// Writes the Off or Overtime end moment into `config`, for `config.json` (SPEC §10.1).
    pub fn store(&self, config: &mut Config) {
        config.off_until = None;
        config.overtime_until = None;
        match self.switch {
            Some(Switch::Off { until }) => config.off_until = Some(until),
            Some(Switch::Overtime { until }) => config.overtime_until = Some(until),
            None => {}
        }
    }

    /// Brings the scheduler in line with the quiet rules at wall-clock time `wall`, which is
    /// `now` on the scheduler's timeline. Forgets an Off or Overtime that has ended, and tells
    /// the scheduler whether it is quiet; returns what the scheduler did at the edge, if any.
    pub fn sync(
        &mut self,
        wall: LocalTime,
        scheduler: &mut Scheduler,
        now: Duration,
    ) -> Vec<Effect> {
        self.switch = self.switch(wall);
        scheduler.set_quiet(now, self.is_quiet(wall))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::time::Weekday;

    /// 2026-11-01 is a Sunday, so 2026-11-03 is a Tuesday and 2026-11-05 a Thursday.
    fn at(s: &str) -> LocalTime {
        LocalTime::parse_rfc3339(&format!("{s}+03:00")).unwrap()
    }

    fn week() -> WorkWeek {
        WorkWeek::from_config(&Config::default())
    }

    fn off(until: &str) -> Option<Switch> {
        Some(Switch::Off { until: at(until) })
    }

    fn overtime(until: &str) -> Option<Switch> {
        Some(Switch::Overtime { until: at(until) })
    }

    /// Checks one row of the SPEC §7 table, that the icon is greyed exactly while quiet, and
    /// what pressing leaves in force.
    fn row(
        now: &str,
        switch: Option<Switch>,
        expect: (&'static str, Press, &str, bool),
        after_press: Option<Switch>,
    ) {
        let now = at(now);
        let (label, press, tooltip, greyed) = expect;
        let item = switch_item(&week(), switch, now);
        assert_eq!(
            item,
            SwitchItem {
                label,
                press,
                tooltip: tooltip.into(),
                greyed
            }
        );
        assert_eq!(is_quiet(&week(), switch, now), greyed);

        let mut config = Config::default();
        (config.off_until, config.overtime_until) = match switch {
            Some(Switch::Off { until }) => (Some(until), None),
            Some(Switch::Overtime { until }) => (None, Some(until)),
            None => (None, None),
        };
        let mut quiet = Quiet::new(&config);
        quiet.press(now);
        assert_eq!(quiet.switch(now), after_press);
    }

    #[test]
    fn row_in_hours_nudging() {
        row(
            "2026-11-03T10:00:00",
            None,
            (
                "Turn off until next work day",
                Press::StartOff,
                "breakpoint",
                false,
            ),
            off("2026-11-04T08:00:00"),
        );
    }

    #[test]
    fn row_in_hours_off() {
        row(
            "2026-11-03T10:00:00",
            off("2026-11-04T08:00:00"),
            (
                "Turn back on",
                Press::EndOff,
                "breakpoint: off until Wednesday 08:00",
                true,
            ),
            None,
        );
    }

    #[test]
    fn row_outside_hours_quiet() {
        row(
            "2026-11-05T18:00:00",
            None,
            (
                "Turn on until midnight",
                Press::StartOvertime,
                "breakpoint: quiet until Sunday 08:00",
                true,
            ),
            overtime("2026-11-06T00:00:00"),
        );
    }

    #[test]
    fn row_outside_hours_off_still_in_force() {
        row(
            "2026-11-05T18:00:00",
            off("2026-11-08T08:00:00"),
            (
                "Turn on until midnight",
                Press::EndOffStartOvertime,
                "breakpoint: off until Sunday 08:00",
                true,
            ),
            overtime("2026-11-06T00:00:00"),
        );
    }

    #[test]
    fn row_overtime_outside_hours() {
        row(
            "2026-11-05T18:00:00",
            overtime("2026-11-06T00:00:00"),
            (
                "Turn off now",
                Press::EndOvertime,
                "breakpoint: on until midnight",
                false,
            ),
            None,
        );
    }

    #[test]
    fn row_overtime_in_hours_started_before_work_began() {
        row(
            "2026-11-03T09:00:00",
            overtime("2026-11-04T00:00:00"),
            (
                "Turn off until next work day",
                Press::EndOvertimeStartOff,
                "breakpoint: on until midnight",
                false,
            ),
            off("2026-11-04T08:00:00"),
        );
    }

    #[test]
    fn the_weekend_is_outside_hours() {
        // Friday 10:00 and Saturday 10:00.
        for now in ["2026-11-06T10:00:00", "2026-11-07T10:00:00"] {
            let item = switch_item(&week(), None, at(now));
            assert_eq!(item.press, Press::StartOvertime, "{now}");
            assert_eq!(item.tooltip, "breakpoint: quiet until Sunday 08:00");
        }
    }

    #[test]
    fn work_hours_include_the_start_and_exclude_the_end() {
        let w = week();
        assert!(!w.in_hours(at("2026-11-03T07:59:59")));
        assert!(w.in_hours(at("2026-11-03T08:00:00")));
        assert!(w.in_hours(at("2026-11-03T16:59:59")));
        assert!(!w.in_hours(at("2026-11-03T17:00:00")));
        assert!(
            w.in_hours(at("2026-11-01T12:00:00")),
            "Sunday is a work day"
        );
    }

    #[test]
    fn before_work_on_a_work_day_is_quiet_until_that_mornings_start() {
        let item = switch_item(&week(), None, at("2026-11-03T07:00:00"));
        assert_eq!(item.tooltip, "breakpoint: quiet until Tuesday 08:00");
    }

    #[test]
    fn off_set_at_16_on_thursday_ends_sunday_at_8() {
        let mut quiet = Quiet::new(&Config::default());
        quiet.press(at("2026-11-05T16:00:00"));
        assert_eq!(
            quiet.switch(at("2026-11-05T16:00:00")),
            off("2026-11-08T08:00:00")
        );
        for now in [
            "2026-11-05T17:00:00",
            "2026-11-06T12:00:00",
            "2026-11-08T07:59:59",
        ] {
            assert!(quiet.is_quiet(at(now)), "{now}");
        }
        assert!(!quiet.is_quiet(at("2026-11-08T08:00:00")));
        assert_eq!(quiet.switch(at("2026-11-08T08:00:00")), None);
        assert_eq!(quiet.item(at("2026-11-08T08:00:00")).press, Press::StartOff);
    }

    #[test]
    fn off_skips_to_the_next_work_day_even_a_week_on() {
        let mut config = Config::default();
        for day in Weekday::ALL {
            config.work_days.set(day, day == Weekday::Tue);
        }
        let mut quiet = Quiet::new(&config);
        quiet.press(at("2026-11-03T10:00:00"));
        assert_eq!(
            quiet.switch(at("2026-11-03T10:00:00")),
            off("2026-11-10T08:00:00")
        );
    }

    #[test]
    fn overtime_runs_to_midnight_through_work_hours() {
        let mut quiet = Quiet::new(&Config::default());
        quiet.press(at("2026-11-03T07:00:00"));
        assert_eq!(
            quiet.switch(at("2026-11-03T07:00:00")),
            overtime("2026-11-04T00:00:00")
        );
        for now in [
            "2026-11-03T07:59:59",
            "2026-11-03T08:00:00",
            "2026-11-03T17:00:00",
            "2026-11-03T23:59:59",
        ] {
            assert!(!quiet.is_quiet(at(now)), "{now}");
        }
        assert!(quiet.is_quiet(at("2026-11-04T00:00:00")));
        assert_eq!(quiet.switch(at("2026-11-04T00:00:00")), None);
    }

    #[test]
    fn store_keeps_at_most_one_end_moment() {
        let mut config = Config::default();
        let mut quiet = Quiet::new(&config);
        quiet.press(at("2026-11-03T10:00:00"));
        quiet.store(&mut config);
        assert_eq!(config.off_until, Some(at("2026-11-04T08:00:00")));
        assert_eq!(config.overtime_until, None);

        // Outside hours with Off in force: pressing ends Off and starts Overtime.
        quiet.press(at("2026-11-03T18:00:00"));
        quiet.store(&mut config);
        assert_eq!(config.off_until, None);
        assert_eq!(config.overtime_until, Some(at("2026-11-04T00:00:00")));

        quiet.press(at("2026-11-03T18:30:00"));
        quiet.store(&mut config);
        assert_eq!((config.off_until, config.overtime_until), (None, None));
    }

    #[test]
    fn the_tooltip_reads_the_end_moment_on_its_own_clock() {
        // Off saved in another time zone shows the time written in the file.
        let until = LocalTime::parse_rfc3339("2026-11-04T08:00:00+01:00").unwrap();
        let item = switch_item(
            &week(),
            Some(Switch::Off { until }),
            at("2026-11-03T10:00:00"),
        );
        assert_eq!(item.tooltip, "breakpoint: off until Wednesday 08:00");
    }
}
