//! The settings window's form: filled from config, the profile it shows, validation, and what a
//! Save applies at once (SPEC §9.2).
//!
//! The platform only copies controls into a [`Form`] and back. Which profile the drop-down shows,
//! what picking one fills in, which field a Save refuses and how a valid Save changes the
//! scheduler and the quiet rules are all decided here.

use std::ops::RangeInclusive;
use std::time::Duration;

use super::config::{
    AwayNudge, Config, Enforcement, Nudges, PAUSE_EVERY, PAUSE_LENGTH, Profile, STAND_EVERY,
    STAND_LENGTH, WATER_EVERY, Water, WorkDays, WorkHours,
};
use super::quiet::{Quiet, WorkWeek};
use super::scheduler::{Effect, Scheduler};
use super::time::{LocalTime, TimeOfDay};

/// The settings window's fields as the user left them. Minutes are the edit boxes' text, which
/// may be anything until a Save validates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub work_days: WorkDays,
    pub start: TimeOfDay,
    pub end: TimeOfDay,
    pub stand: AwayRow,
    pub pause: AwayRow,
    pub water: WaterRow,
}

/// The Stand/Move or Pause row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwayRow {
    pub on: bool,
    pub every: String,
    pub length: String,
    pub enforcement: Enforcement,
}

/// The Water row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterRow {
    pub on: bool,
    pub every: String,
}

/// A field a Save can refuse, top to bottom as the window shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// No work day checked; the focus goes to the first checkbox.
    WorkDays,
    /// The end of work hours is not after the start; the focus goes to the "to" picker.
    WorkHoursEnd,
    StandEvery,
    StandLength,
    PauseEvery,
    PauseLength,
    WaterEvery,
}

/// Why a Save was refused: the field to focus and the one message box to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    pub field: Field,
    pub message: String,
}

/// What a valid Save writes: the settings window's fields in config form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub work_days: WorkDays,
    pub work_hours: WorkHours,
    pub nudges: Nudges,
}

impl Form {
    /// The form as `config` fills it.
    pub fn new(config: &Config) -> Self {
        let away = |n: AwayNudge| AwayRow {
            on: n.on,
            every: n.every_minutes.to_string(),
            length: n.length_minutes.to_string(),
            enforcement: n.enforcement,
        };
        Self {
            work_days: config.work_days,
            start: config.work_hours.start,
            end: config.work_hours.end,
            stand: away(config.nudges.stand),
            pause: away(config.nudges.pause),
            water: WaterRow {
                on: config.nudges.water.on,
                every: config.nudges.water.every_minutes.to_string(),
            },
        }
    }

    /// What the profile drop-down shows: the profile whose cadences, lengths and levels the
    /// form holds exactly; `None` is Custom. A box that holds no number matches no profile.
    pub fn profile(&self) -> Option<Profile> {
        let away = |row: &AwayRow| {
            Some(AwayNudge {
                on: row.on,
                every_minutes: minutes(&row.every)?,
                length_minutes: minutes(&row.length)?,
                enforcement: row.enforcement,
            })
        };
        let nudges = Nudges {
            stand: away(&self.stand)?,
            pause: away(&self.pause)?,
            water: Water {
                on: self.water.on,
                every_minutes: minutes(&self.water.every)?,
            },
        };
        nudges.profile()
    }

    /// Picking `profile` in the drop-down: fills its cadences, lengths and levels in, leaves the
    /// on/off boxes and the work week alone, and saves nothing.
    pub fn apply(&mut self, profile: Profile) {
        let preset = profile.nudges();
        let fill = |row: &mut AwayRow, n: AwayNudge| {
            row.every = n.every_minutes.to_string();
            row.length = n.length_minutes.to_string();
            row.enforcement = n.enforcement;
        };
        fill(&mut self.stand, preset.stand);
        fill(&mut self.pause, preset.pause);
        self.water.every = preset.water.every_minutes.to_string();
    }

    /// Checks the form top to bottom; the first field that is wrong refuses the Save. A nudge
    /// that is off still has its numbers checked, since they are saved too.
    pub fn validate(&self) -> Result<Settings, Invalid> {
        if self.work_days.is_empty() {
            return Err(Invalid {
                field: Field::WorkDays,
                message: "Check at least one work day.".into(),
            });
        }
        if self.end <= self.start {
            return Err(Invalid {
                field: Field::WorkHoursEnd,
                message: "Work hours: \"to\" must be after \"from\".".into(),
            });
        }
        let stand_every = check(&self.stand.every, Field::StandEvery, STAND_EVERY)?;
        let stand_length = check(&self.stand.length, Field::StandLength, STAND_LENGTH)?;
        let pause_every = check(&self.pause.every, Field::PauseEvery, PAUSE_EVERY)?;
        let pause_length = check(&self.pause.length, Field::PauseLength, PAUSE_LENGTH)?;
        let water_every = check(&self.water.every, Field::WaterEvery, WATER_EVERY)?;
        Ok(Settings {
            work_days: self.work_days,
            work_hours: WorkHours {
                start: self.start,
                end: self.end,
            },
            nudges: Nudges {
                stand: AwayNudge {
                    on: self.stand.on,
                    every_minutes: stand_every,
                    length_minutes: stand_length,
                    enforcement: self.stand.enforcement,
                },
                pause: AwayNudge {
                    on: self.pause.on,
                    every_minutes: pause_every,
                    length_minutes: pause_length,
                    enforcement: self.pause.enforcement,
                },
                water: Water {
                    on: self.water.on,
                    every_minutes: water_every,
                },
            },
        })
    }
}

impl Field {
    /// The field's name in a message box.
    pub fn name(self) -> &'static str {
        match self {
            Field::WorkDays => "Work days",
            Field::WorkHoursEnd => "Work hours",
            Field::StandEvery => "Stand/Move every",
            Field::StandLength => "Stand length",
            Field::PauseEvery => "Pause every",
            Field::PauseLength => "Pause length",
            Field::WaterEvery => "Water every",
        }
    }
}

/// A whole number of minutes in an edit box; surrounding spaces are allowed.
fn minutes(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn check(text: &str, field: Field, range: RangeInclusive<u32>) -> Result<u32, Invalid> {
    minutes(text)
        .filter(|n| range.contains(n))
        .ok_or_else(|| Invalid {
            field,
            message: format!(
                "{}: enter whole minutes from {} to {}.",
                field.name(),
                range.start(),
                range.end()
            ),
        })
}

/// Applies a valid Save at once (SPEC §9.2) and returns what the platform must carry out. The
/// caller then writes `config` to `config.json`.
///
/// The work week is re-evaluated with the quiet rules first: now quiet expires what is pending
/// with cause `quiet`, quiet just ended restarts every clock. An Off or Overtime in force keeps
/// its end moment. Then the nudges: a changed on/off, cadence or length restarts that clock,
/// turning a nudge off expires its pending occurrence with cause `settings`, and a level applies
/// from the next occurrence.
pub fn save(
    settings: &Settings,
    config: &mut Config,
    quiet: &mut Quiet,
    scheduler: &mut Scheduler,
    wall: LocalTime,
    now: Duration,
) -> Vec<Effect> {
    config.work_days = settings.work_days;
    config.work_hours = settings.work_hours;
    config.nudges = settings.nudges;
    quiet.set_week(WorkWeek::from_config(config));
    let mut effects = quiet.sync(wall, scheduler, now);
    effects.extend(scheduler.set_nudges(now, settings.nudges));
    quiet.store(config);
    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::time::Weekday;

    fn form() -> Form {
        Form::new(&Config::default())
    }

    fn refused(edit: impl FnOnce(&mut Form)) -> Field {
        let mut form = form();
        edit(&mut form);
        form.validate()
            .expect_err("the Save should be refused")
            .field
    }

    #[test]
    fn the_defaults_fill_the_form_and_save_unchanged() {
        let config = Config::default();
        let settings = Form::new(&config).validate().unwrap();
        assert_eq!(settings.work_days, config.work_days);
        assert_eq!(settings.work_hours, config.work_hours);
        assert_eq!(settings.nudges, config.nudges);
        assert_eq!(form().profile(), Some(Profile::Balanced));
    }

    #[test]
    fn no_work_day_is_refused_on_work_days() {
        assert_eq!(
            refused(|f| {
                for day in Weekday::ALL {
                    f.work_days.set(day, false);
                }
            }),
            Field::WorkDays
        );
    }

    #[test]
    fn an_end_not_after_the_start_is_refused_on_the_end() {
        for (start, end) in [((9, 0), (9, 0)), ((17, 0), (8, 0)), ((8, 1), (8, 0))] {
            assert_eq!(
                refused(|f| {
                    f.start = TimeOfDay::new(start.0, start.1).unwrap();
                    f.end = TimeOfDay::new(end.0, end.1).unwrap();
                }),
                Field::WorkHoursEnd,
                "{start:?} to {end:?}"
            );
        }
    }

    #[test]
    fn each_number_out_of_range_is_refused_on_its_own_field() {
        type Edit = fn(&mut Form, &str);
        let cases: [(Edit, Field, [&str; 2]); 5] = [
            (
                |f, v| f.stand.every = v.into(),
                Field::StandEvery,
                ["9", "241"],
            ),
            (
                |f, v| f.stand.length = v.into(),
                Field::StandLength,
                ["1", "6"],
            ),
            (
                |f, v| f.pause.every = v.into(),
                Field::PauseEvery,
                ["19", "241"],
            ),
            (
                |f, v| f.pause.length = v.into(),
                Field::PauseLength,
                ["1", "16"],
            ),
            (
                |f, v| f.water.every = v.into(),
                Field::WaterEvery,
                ["9", "241"],
            ),
        ];
        for (edit, field, values) in cases {
            for value in values {
                assert_eq!(refused(|f| edit(f, value)), field, "{value}");
            }
        }
    }

    #[test]
    fn the_range_edges_are_saved() {
        let mut f = form();
        (f.stand.every, f.stand.length) = ("10".into(), "5".into());
        (f.pause.every, f.pause.length) = ("240".into(), "2".into());
        f.water.every = "240".into();
        let n = f.validate().unwrap().nudges;
        assert_eq!((n.stand.every_minutes, n.stand.length_minutes), (10, 5));
        assert_eq!((n.pause.every_minutes, n.pause.length_minutes), (240, 2));
        assert_eq!(n.water.every_minutes, 240);

        let mut f = form();
        (f.stand.every, f.stand.length) = ("240".into(), "2".into());
        (f.pause.every, f.pause.length) = ("20".into(), "15".into());
        f.water.every = "10".into();
        assert!(f.validate().is_ok());
    }

    #[test]
    fn a_box_without_a_whole_number_is_refused() {
        for text in ["", " ", "abc", "3.5", "-10", "+30", "1e2", "99999999999"] {
            assert_eq!(
                refused(|f| f.pause.every = text.into()),
                Field::PauseEvery,
                "{text:?}"
            );
        }
        let mut f = form();
        f.stand.every = " 45 ".into();
        assert_eq!(f.validate().unwrap().nudges.stand.every_minutes, 45);
    }

    #[test]
    fn a_nudge_that_is_off_still_has_its_numbers_checked() {
        assert_eq!(
            refused(|f| {
                f.water.on = false;
                f.water.every = "5".into();
            }),
            Field::WaterEvery
        );
    }

    #[test]
    fn the_first_wrong_field_from_the_top_is_named() {
        assert_eq!(
            refused(|f| {
                f.water.every = "0".into();
                f.stand.length = "9".into();
                f.end = f.start;
            }),
            Field::WorkHoursEnd
        );
        assert_eq!(
            refused(|f| {
                f.water.every = "0".into();
                f.pause.length = "0".into();
            }),
            Field::PauseLength
        );
    }

    #[test]
    fn the_message_names_the_field_and_its_range() {
        let mut f = form();
        f.pause.length = "20".into();
        assert_eq!(
            f.validate().unwrap_err().message,
            "Pause length: enter whole minutes from 2 to 15."
        );
    }

    #[test]
    fn a_stand_cadence_that_does_not_divide_the_pause_cadence_is_allowed() {
        let mut f = form();
        (f.stand.every, f.pause.every) = ("25".into(), "60".into());
        assert!(f.validate().is_ok());
    }

    #[test]
    fn all_three_nudges_may_be_off() {
        let mut f = form();
        (f.stand.on, f.pause.on, f.water.on) = (false, false, false);
        let n = f.validate().unwrap().nudges;
        assert!(!n.stand.on && !n.pause.on && !n.water.on);
    }

    #[test]
    fn picking_a_profile_fills_numbers_and_levels_only() {
        let mut f = form();
        f.stand.on = false;
        f.work_days.set(Weekday::Fri, true);
        f.stand.every = "17".into();
        f.apply(Profile::Strict);
        assert_eq!(f.profile(), Some(Profile::Strict));
        assert!(!f.stand.on, "on/off left alone");
        assert!(f.work_days.contains(Weekday::Fri), "work week left alone");
        let mut expect = Profile::Strict.nudges();
        expect.stand.on = false;
        assert_eq!(f.validate().unwrap().nudges, expect);
    }

    #[test]
    fn editing_any_number_or_level_shows_custom() {
        let edits: [fn(&mut Form); 7] = [
            |f| f.stand.every = "31".into(),
            |f| f.stand.length = "3".into(),
            |f| f.stand.enforcement = Enforcement::Firm,
            |f| f.pause.every = "90".into(),
            |f| f.pause.enforcement = Enforcement::Soft,
            |f| f.water.every = "45".into(),
            |f| f.pause.length = "".into(),
        ];
        for edit in edits {
            let mut f = form();
            edit(&mut f);
            assert_eq!(f.profile(), None, "{f:?}");
        }
    }

    #[test]
    fn on_off_and_the_work_week_do_not_change_the_profile() {
        let mut f = form();
        f.apply(Profile::Gentle);
        (f.stand.on, f.water.on) = (false, false);
        f.work_days.set(Weekday::Sun, false);
        f.end = TimeOfDay::new(16, 0).unwrap();
        assert_eq!(f.profile(), Some(Profile::Gentle));
    }

    #[test]
    fn editing_back_to_a_preset_shows_it_again() {
        let mut f = form();
        f.stand.every = "45".into();
        f.pause.every = "90".into();
        assert_eq!(f.profile(), None);
        f.pause.enforcement = Enforcement::Soft;
        assert_eq!(f.profile(), Some(Profile::Gentle));
        f.stand.every = "045".into();
        assert_eq!(
            f.profile(),
            Some(Profile::Gentle),
            "the number is what counts"
        );
    }
}
