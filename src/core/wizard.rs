//! The first-run wizard's decisions (SPEC §9.1): the text it shows and what its picks become.
//!
//! The wizard asks two things, the profile and Start with Windows, and shows the work week the
//! defaults give. Start, closing it, and Cancel in the settings window that Change… opens all
//! accept the picks: the defaults with the picked profile's cadences, lengths and levels. The
//! platform only shows the lines and reads the controls back.

use super::config::{Config, Enforcement, Profile};
use super::settings::{Form, Settings};

/// What the wizard asks. The default is what it preselects: Balanced, Start with Windows on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picks {
    pub profile: Profile,
    pub start_with_windows: bool,
}

impl Default for Picks {
    fn default() -> Self {
        Self {
            profile: Profile::Balanced,
            start_with_windows: true,
        }
    }
}

impl Picks {
    /// The config that accepting the picks writes: the defaults, with the picked profile's
    /// numbers and levels. All three nudges stay on; enforcement is never asked.
    pub fn config(&self) -> Config {
        let mut config = Config::default();
        config.nudges.apply(self.profile);
        config
    }

    /// What the settings window starts with after Change…: the picks and defaults for the rest.
    pub fn form(&self) -> Form {
        Form::new(&self.config())
    }

    /// The picks as a Save would hand them over, so that accepting them goes the same way.
    pub fn settings(&self) -> Settings {
        let config = self.config();
        Settings {
            work_days: config.work_days,
            work_hours: config.work_hours,
            nudges: config.nudges,
        }
    }
}

/// One plain line of a profile's numbers, the text of its radio button, e.g. *Balanced: stand
/// every 30 min, pause every 60 (Pause insists). Water rides along hourly.*
pub fn profile_line(profile: Profile) -> String {
    let n = profile.nudges();
    let firm = |level| level == Enforcement::Firm;
    let insists = match (firm(n.stand.enforcement), firm(n.pause.enforcement)) {
        (false, false) => "",
        (false, true) => " (Pause insists)",
        (true, false) => " (Stand/Move insists)",
        (true, true) => " (both insist)",
    };
    let water = match n.water.every_minutes {
        60 => "hourly".to_string(),
        every => format!("every {every} min"),
    };
    format!(
        "{}: stand every {} min, pause every {}{insists}. Water rides along {water}.",
        name(profile),
        n.stand.every_minutes,
        n.pause.every_minutes,
    )
}

/// The read-only work-week line, e.g. *Nudges Sunday to Thursday, 08:00 to 17:00*. A run of
/// days in a row, Sunday first, is named by its ends; any other set is listed.
pub fn work_week_line(config: &Config) -> String {
    let days: Vec<_> = config.work_days.iter().collect();
    let in_a_row = days
        .windows(2)
        .all(|pair| pair[1] as usize == pair[0] as usize + 1);
    let named = match days.as_slice() {
        [] => "on no day".to_string(),
        [day] => day.name().to_string(),
        [first, .., last] if in_a_row => format!("{} to {}", first.name(), last.name()),
        _ => {
            let names: Vec<_> = days.iter().map(|day| day.name()).collect();
            names.join(", ")
        }
    };
    format!(
        "Nudges {named}, {} to {}",
        config.work_hours.start, config.work_hours.end
    )
}

fn name(profile: Profile) -> &'static str {
    match profile {
        Profile::Gentle => "Gentle",
        Profile::Balanced => "Balanced",
        Profile::Strict => "Strict",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::time::{TimeOfDay, Weekday};

    #[test]
    fn the_preselected_picks_are_balanced_and_start_with_windows() {
        let picks = Picks::default();
        assert_eq!(picks.profile, Profile::Balanced);
        assert!(picks.start_with_windows);
        assert_eq!(picks.config(), Config::default());
    }

    #[test]
    fn each_profile_becomes_the_defaults_with_its_numbers() {
        for profile in Profile::ALL {
            let picks = Picks {
                profile,
                start_with_windows: false,
            };
            let config = picks.config();
            assert_eq!(config.nudges, profile.nudges(), "{profile:?}");
            assert_eq!(config.nudges.profile(), Some(profile));
            assert!(config.nudges.stand.on && config.nudges.pause.on && config.nudges.water.on);
            let defaults = Config::default();
            assert_eq!(config.work_days, defaults.work_days);
            assert_eq!(config.work_hours, defaults.work_hours);
            assert_eq!((config.off_until, config.overtime_until), (None, None));
        }
    }

    #[test]
    fn the_settings_and_the_form_carry_the_picks() {
        let picks = Picks {
            profile: Profile::Strict,
            start_with_windows: true,
        };
        let settings = picks.settings();
        assert_eq!(settings.nudges, Profile::Strict.nudges());
        assert_eq!(settings.work_days, Config::default().work_days);
        assert_eq!(settings.work_hours, Config::default().work_hours);
        let form = picks.form();
        assert_eq!(form.profile(), Some(Profile::Strict));
        assert_eq!(form.validate(), Ok(settings), "Save unchanged is the picks");
    }

    #[test]
    fn each_profile_has_one_line_of_its_numbers() {
        assert_eq!(
            profile_line(Profile::Gentle),
            "Gentle: stand every 45 min, pause every 90. Water rides along hourly."
        );
        assert_eq!(
            profile_line(Profile::Balanced),
            "Balanced: stand every 30 min, pause every 60 (Pause insists). Water rides along hourly."
        );
        assert_eq!(
            profile_line(Profile::Strict),
            "Strict: stand every 30 min, pause every 30 (both insist). Water rides along hourly."
        );
    }

    #[test]
    fn the_default_work_week_line() {
        assert_eq!(
            work_week_line(&Config::default()),
            "Nudges Sunday to Thursday, 08:00 to 17:00"
        );
    }

    #[test]
    fn other_work_weeks_read_plainly() {
        let line = |days: &[Weekday], start: (u8, u8), end: (u8, u8)| {
            let mut config = Config::default();
            for day in Weekday::ALL {
                config.work_days.set(day, days.contains(&day));
            }
            config.work_hours.start = TimeOfDay::new(start.0, start.1).unwrap();
            config.work_hours.end = TimeOfDay::new(end.0, end.1).unwrap();
            work_week_line(&config)
        };
        use Weekday::*;
        assert_eq!(
            line(&[Mon, Tue, Wed, Thu, Fri], (9, 0), (17, 30)),
            "Nudges Monday to Friday, 09:00 to 17:30"
        );
        assert_eq!(
            line(&[Wed], (8, 0), (12, 0)),
            "Nudges Wednesday, 08:00 to 12:00"
        );
        assert_eq!(
            line(&[Sun, Tue, Sat], (8, 0), (17, 0)),
            "Nudges Sunday, Tuesday, Saturday, 08:00 to 17:00"
        );
        assert_eq!(
            line(&Weekday::ALL, (8, 0), (17, 0)),
            "Nudges Sunday to Saturday, 08:00 to 17:00"
        );
    }
}
