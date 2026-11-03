//! The config model, defaults, profiles, validation and JSON load/save.
//!
//! `config.json` (SPEC §10.1) holds the settings window's fields plus the Off / Overtime end
//! moment. It is read once at start-up and rewritten whole on save. Reading never fails: each
//! value that is missing, out of range or malformed falls back to its default on its own, and
//! [`Loaded::notice`] says whether the user should be told.

use std::fs;
use std::io;
use std::ops::RangeInclusive;
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};

use super::time::{LocalTime, TimeOfDay, Weekday};

/// Allowed values, in whole minutes (SPEC §9.2).
pub const STAND_EVERY: RangeInclusive<u32> = 10..=240;
pub const PAUSE_EVERY: RangeInclusive<u32> = 20..=240;
pub const WATER_EVERY: RangeInclusive<u32> = 10..=240;
pub const STAND_LENGTH: RangeInclusive<u32> = 2..=5;
pub const PAUSE_LENGTH: RangeInclusive<u32> = 2..=15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub work_days: WorkDays,
    pub work_hours: WorkHours,
    pub nudges: Nudges,
    /// When Off ends, if it is in force.
    pub off_until: Option<LocalTime>,
    /// When Overtime ends, if it is in force. At most one of the two is set.
    pub overtime_until: Option<LocalTime>,
}

impl Default for Config {
    /// Balanced, Sunday to Thursday, 08:00 to 17:00, all three nudges on (SPEC §3, §7).
    fn default() -> Self {
        Self {
            work_days: WorkDays::default(),
            work_hours: WorkHours::default(),
            nudges: Nudges::default(),
            off_until: None,
            overtime_until: None,
        }
    }
}

/// The days nudges may fire on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkDays([bool; 7]);

impl WorkDays {
    pub fn contains(self, day: Weekday) -> bool {
        self.0[day as usize]
    }

    pub fn set(&mut self, day: Weekday, on: bool) {
        self.0[day as usize] = on;
    }

    pub fn is_empty(self) -> bool {
        !self.0.contains(&true)
    }

    /// The work days, Sunday first.
    pub fn iter(self) -> impl Iterator<Item = Weekday> {
        Weekday::ALL
            .into_iter()
            .filter(move |&day| self.contains(day))
    }
}

impl Default for WorkDays {
    fn default() -> Self {
        let mut days = Self([false; 7]);
        for day in [
            Weekday::Sun,
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
        ] {
            days.set(day, true);
        }
        days
    }
}

/// Work hours on each work day; `end` is always after `start` (no overnight hours in v1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkHours {
    pub start: TimeOfDay,
    pub end: TimeOfDay,
}

impl Default for WorkHours {
    fn default() -> Self {
        Self {
            start: TimeOfDay::new(8, 0).unwrap(),
            end: TimeOfDay::new(17, 0).unwrap(),
        }
    }
}

/// The three nudges' settings: what a profile fills in, plus each nudge's on/off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nudges {
    pub stand: AwayNudge,
    pub pause: AwayNudge,
    pub water: Water,
}

impl Default for Nudges {
    fn default() -> Self {
        Profile::Balanced.nudges()
    }
}

impl Nudges {
    /// Fills in `profile`'s cadences, lengths and levels, leaving each nudge's on/off alone.
    pub fn apply(&mut self, profile: Profile) {
        let preset = profile.nudges();
        let (stand, pause, water) = (self.stand.on, self.pause.on, self.water.on);
        *self = preset;
        self.stand.on = stand;
        self.pause.on = pause;
        self.water.on = water;
    }

    /// The profile whose cadences, lengths and levels these are exactly; `None` is Custom.
    pub fn profile(&self) -> Option<Profile> {
        Profile::ALL.into_iter().find(|&profile| {
            let mut preset = *self;
            preset.apply(profile);
            preset == *self
        })
    }
}

/// Stand/Move or Pause: a nudge satisfied by being away for its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwayNudge {
    pub on: bool,
    /// The cadence, in active minutes.
    pub every_minutes: u32,
    /// The stand or pause length, in minutes.
    pub length_minutes: u32,
    pub enforcement: Enforcement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Water {
    pub on: bool,
    /// The cadence, in active minutes.
    pub every_minutes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Enforcement {
    Soft,
    Firm,
}

/// A named preset of cadences, lengths and enforcement levels (SPEC §3). Not stored: read off
/// the numbers with [`Nudges::profile`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Gentle,
    Balanced,
    Strict,
}

impl Profile {
    pub const ALL: [Profile; 3] = [Profile::Gentle, Profile::Balanced, Profile::Strict];

    /// The preset with every nudge on.
    pub fn nudges(self) -> Nudges {
        use Enforcement::{Firm, Soft};
        let (stand_every, pause_every, stand_level, pause_level) = match self {
            Profile::Gentle => (45, 90, Soft, Soft),
            Profile::Balanced => (30, 60, Soft, Firm),
            Profile::Strict => (30, 30, Firm, Firm),
        };
        Nudges {
            stand: AwayNudge {
                on: true,
                every_minutes: stand_every,
                length_minutes: 2,
                enforcement: stand_level,
            },
            pause: AwayNudge {
                on: true,
                every_minutes: pause_every,
                length_minutes: 5,
                enforcement: pause_level,
            },
            water: Water {
                on: true,
                every_minutes: 60,
            },
        }
    }
}

/// What reading `config.json` gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub config: Config,
    /// The file could not be read, or some value in it fell back to its default: show the
    /// bad-config notice once.
    pub notice: bool,
}

/// Reads the config at `path`; `None` if there is no file yet (first run). The file is never
/// written here, so an unreadable one is left byte-for-byte as it was.
pub fn load(path: &Path, now: LocalTime) -> Option<Loaded> {
    match fs::read_to_string(path) {
        Ok(text) => Some(Config::from_json(&text, now)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(_) => Some(Loaded {
            config: Config::default(),
            notice: true,
        }),
    }
}

/// Writes `config` to `path` whole, creating the folder if needed. Goes through a temporary
/// file so a failed write never leaves half a config behind.
pub fn save(path: &Path, config: &Config) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, config.to_json())?;
    fs::rename(&tmp, path)
}

impl Config {
    /// Parses `config.json`. `now` decides whether an Off or Overtime end moment has passed.
    pub fn from_json(text: &str, now: LocalTime) -> Loaded {
        let parsed = serde_json::from_str::<Value>(text).ok();
        let root = parsed.as_ref().and_then(Value::as_object);
        // Unparseable, or not an object: every value below falls back.
        let mut r = Reader {
            notice: root.is_none(),
        };
        let d = Config::default();

        let work_days = r.value(root, "workDays", d.work_days, parse_work_days);
        let hours = object(root, "workHours");
        let mut work_hours = WorkHours {
            start: r.value(hours, "start", d.work_hours.start, parse_time),
            end: r.value(hours, "end", d.work_hours.end, parse_time),
        };
        if work_hours.end <= work_hours.start {
            r.notice = true;
            work_hours = d.work_hours;
        }
        let nudges = Nudges {
            stand: r.away_nudge(root, "stand", d.nudges.stand, STAND_EVERY, STAND_LENGTH),
            pause: r.away_nudge(root, "pause", d.nudges.pause, PAUSE_EVERY, PAUSE_LENGTH),
            water: {
                let water = object(root, "water");
                Water {
                    on: r.value(water, "on", d.nudges.water.on, Value::as_bool),
                    every_minutes: r.minutes(
                        water,
                        "everyMinutes",
                        d.nudges.water.every_minutes,
                        WATER_EVERY,
                    ),
                }
            },
        };
        let mut off_until = r.value(root, "offUntil", None, parse_moment);
        let mut overtime_until = r.value(root, "overtimeUntil", None, parse_moment);
        if off_until.is_some() && overtime_until.is_some() {
            r.notice = true;
            (off_until, overtime_until) = (None, None);
        }
        // An end moment already past is ignored silently.
        let future = |t: Option<LocalTime>| t.filter(|t| t.unix > now.unix);

        Loaded {
            config: Config {
                work_days,
                work_hours,
                nudges,
                off_until: future(off_until),
                overtime_until: future(overtime_until),
            },
            notice: r.notice,
        }
    }

    /// The whole `config.json`, in the shape and order of SPEC §10.1.
    pub fn to_json(&self) -> String {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct File {
            work_days: Vec<&'static str>,
            work_hours: Hours,
            stand: Away,
            pause: Away,
            water: WaterFile,
            off_until: Option<String>,
            overtime_until: Option<String>,
        }
        #[derive(Serialize)]
        struct Hours {
            start: String,
            end: String,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Away {
            on: bool,
            every_minutes: u32,
            length_minutes: u32,
            enforcement: Enforcement,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct WaterFile {
            on: bool,
            every_minutes: u32,
        }
        let away = |n: AwayNudge| Away {
            on: n.on,
            every_minutes: n.every_minutes,
            length_minutes: n.length_minutes,
            enforcement: n.enforcement,
        };
        let file = File {
            work_days: self.work_days.iter().map(Weekday::key).collect(),
            work_hours: Hours {
                start: self.work_hours.start.to_string(),
                end: self.work_hours.end.to_string(),
            },
            stand: away(self.nudges.stand),
            pause: away(self.nudges.pause),
            water: WaterFile {
                on: self.nudges.water.on,
                every_minutes: self.nudges.water.every_minutes,
            },
            off_until: self.off_until.map(|t| t.to_string()),
            overtime_until: self.overtime_until.map(|t| t.to_string()),
        };
        let mut json = serde_json::to_string_pretty(&file).expect("plain data always serializes");
        json.push('\n');
        json
    }
}

/// Reads values out of the parsed file, noting each one that falls back.
struct Reader {
    notice: bool,
}

impl Reader {
    /// `obj[key]` parsed, or `default` (noted) when the object, the key or a valid value is
    /// missing.
    fn value<T>(
        &mut self,
        obj: Option<&Map<String, Value>>,
        key: &str,
        default: T,
        parse: impl FnOnce(&Value) -> Option<T>,
    ) -> T {
        match obj.and_then(|o| o.get(key)).and_then(parse) {
            Some(v) => v,
            None => {
                self.notice = true;
                default
            }
        }
    }

    fn minutes(
        &mut self,
        obj: Option<&Map<String, Value>>,
        key: &str,
        default: u32,
        range: RangeInclusive<u32>,
    ) -> u32 {
        self.value(obj, key, default, |v| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| range.contains(n))
        })
    }

    fn away_nudge(
        &mut self,
        root: Option<&Map<String, Value>>,
        key: &str,
        d: AwayNudge,
        every: RangeInclusive<u32>,
        length: RangeInclusive<u32>,
    ) -> AwayNudge {
        let obj = object(root, key);
        AwayNudge {
            on: self.value(obj, "on", d.on, Value::as_bool),
            every_minutes: self.minutes(obj, "everyMinutes", d.every_minutes, every),
            length_minutes: self.minutes(obj, "lengthMinutes", d.length_minutes, length),
            enforcement: self.value(obj, "enforcement", d.enforcement, |v| match v.as_str()? {
                "soft" => Some(Enforcement::Soft),
                "firm" => Some(Enforcement::Firm),
                _ => None,
            }),
        }
    }
}

fn object<'a>(obj: Option<&'a Map<String, Value>>, key: &str) -> Option<&'a Map<String, Value>> {
    obj?.get(key)?.as_object()
}

/// A non-empty list of known day names; repeats are harmless.
fn parse_work_days(v: &Value) -> Option<WorkDays> {
    let mut days = WorkDays([false; 7]);
    for name in v.as_array()? {
        days.set(Weekday::from_key(name.as_str()?)?, true);
    }
    (!days.is_empty()).then_some(days)
}

fn parse_time(v: &Value) -> Option<TimeOfDay> {
    TimeOfDay::parse(v.as_str()?)
}

/// `null` or an RFC 3339 moment.
fn parse_moment(v: &Value) -> Option<Option<LocalTime>> {
    match v {
        Value::Null => Some(None),
        Value::String(s) => LocalTime::parse_rfc3339(s).map(Some),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 2026-11-03 10:00 at +03:00, a Tuesday.
    fn now() -> LocalTime {
        at("2026-11-03T10:00:00+03:00")
    }

    fn at(s: &str) -> LocalTime {
        LocalTime::parse_rfc3339(s).unwrap()
    }

    /// The SPEC §10.1 example, which is also the defaults.
    fn spec_example() -> Value {
        json!({
          "workDays": ["sun", "mon", "tue", "wed", "thu"],
          "workHours": { "start": "08:00", "end": "17:00" },
          "stand": { "on": true, "everyMinutes": 30, "lengthMinutes": 2, "enforcement": "soft" },
          "pause": { "on": true, "everyMinutes": 60, "lengthMinutes": 5, "enforcement": "firm" },
          "water": { "on": true, "everyMinutes": 60 },
          "offUntil": null,
          "overtimeUntil": null
        })
    }

    /// Reads the spec example after `edit`.
    fn read_edited(edit: impl FnOnce(&mut Value)) -> Loaded {
        let mut file = spec_example();
        edit(&mut file);
        Config::from_json(&file.to_string(), now())
    }

    /// Asserts that after `edit` the file reads as the defaults changed by `expect`, with the
    /// notice: whatever `edit` broke fell back, and everything else applied.
    fn assert_falls_back(edit: impl FnOnce(&mut Value), expect: impl FnOnce(&mut Config)) {
        let mut config = Config::default();
        expect(&mut config);
        assert_eq!(
            read_edited(edit),
            Loaded {
                config,
                notice: true
            }
        );
    }

    fn remove(f: &mut Value, key: &str) {
        f.as_object_mut().unwrap().remove(key);
    }

    #[test]
    fn defaults_are_balanced_sunday_to_thursday_8_to_17_all_on() {
        let c = Config::default();
        assert_eq!(c.nudges.profile(), Some(Profile::Balanced));
        assert_eq!(
            c.work_days.iter().collect::<Vec<_>>(),
            [
                Weekday::Sun,
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu
            ]
        );
        assert_eq!(c.work_hours.start.to_string(), "08:00");
        assert_eq!(c.work_hours.end.to_string(), "17:00");
        assert!(c.nudges.stand.on && c.nudges.pause.on && c.nudges.water.on);
        assert_eq!((c.off_until, c.overtime_until), (None, None));
    }

    #[test]
    fn the_defaults_write_the_spec_example_in_its_order() {
        let json = Config::default().to_json();
        assert_eq!(
            serde_json::from_str::<Value>(&json).unwrap(),
            spec_example()
        );
        let keys = [
            "workDays",
            "workHours",
            "stand",
            "pause",
            "water",
            "offUntil",
            "overtimeUntil",
        ];
        let positions: Vec<_> = keys.iter().map(|k| json.find(k).unwrap()).collect();
        assert!(positions.is_sorted(), "keys out of order:\n{json}");
        assert!(json.ends_with("}\n"));
    }

    #[test]
    fn the_spec_example_reads_as_the_defaults_without_a_notice() {
        assert_eq!(
            Config::from_json(&spec_example().to_string(), now()),
            Loaded {
                config: Config::default(),
                notice: false
            }
        );
    }

    #[test]
    fn round_trip() {
        let mut config = Config::default();
        config.nudges.apply(Profile::Gentle);
        config.nudges.water.on = false;
        config.nudges.pause.length_minutes = 15;
        config.work_days.set(Weekday::Sun, false);
        config.work_days.set(Weekday::Fri, true);
        config.work_hours = WorkHours {
            start: TimeOfDay::new(9, 30).unwrap(),
            end: TimeOfDay::new(18, 15).unwrap(),
        };
        config.off_until = Some(at("2026-11-04T08:00:00+03:00"));
        assert_eq!(
            Config::from_json(&config.to_json(), now()),
            Loaded {
                config: config.clone(),
                notice: false
            }
        );

        config.off_until = None;
        config.overtime_until = Some(at("2026-11-04T00:00:00-05:30"));
        assert_eq!(
            Config::from_json(&config.to_json(), now()),
            Loaded {
                config,
                notice: false
            }
        );
    }

    #[test]
    fn an_unparseable_file_gives_the_defaults_and_the_notice() {
        for text in [
            "",
            "{",
            "not json",
            "[]",
            "null",
            "42",
            "{\"workDays\": [\"sun\",}",
        ] {
            assert_eq!(
                Config::from_json(text, now()),
                Loaded {
                    config: Config::default(),
                    notice: true
                },
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_missing_value_falls_back_on_its_own() {
        assert_falls_back(
            |f| {
                remove(f, "workDays");
                f["stand"]["everyMinutes"] = json!(45);
            },
            |c| c.nudges.stand.every_minutes = 45,
        );
        assert_falls_back(
            |f| {
                remove(&mut f["pause"], "enforcement");
                f["pause"]["lengthMinutes"] = json!(10);
            },
            |c| c.nudges.pause.length_minutes = 10,
        );
        assert_falls_back(
            |f| {
                remove(&mut f["workHours"], "start");
                f["workHours"]["end"] = json!("16:00");
            },
            |c| c.work_hours.end = TimeOfDay::new(16, 0).unwrap(),
        );
        assert_falls_back(
            |f| {
                remove(f, "water");
                f["stand"]["on"] = json!(false);
            },
            |c| c.nudges.stand.on = false,
        );
        assert_falls_back(|f| remove(f, "offUntil"), |_| {});
    }

    #[test]
    fn an_out_of_range_value_falls_back_on_its_own() {
        for (nudge, key, value) in [
            ("stand", "everyMinutes", 9_i64),
            ("stand", "everyMinutes", 241),
            ("pause", "everyMinutes", 19),
            ("pause", "everyMinutes", 241),
            ("water", "everyMinutes", 9),
            ("water", "everyMinutes", 241),
            ("stand", "lengthMinutes", 1),
            ("stand", "lengthMinutes", 6),
            ("pause", "lengthMinutes", 1),
            ("pause", "lengthMinutes", 16),
            ("pause", "lengthMinutes", -5),
            ("pause", "lengthMinutes", 4_294_967_301),
        ] {
            let loaded = read_edited(|f| {
                f[nudge][key] = json!(value);
                f["water"]["on"] = json!(false);
            });
            let mut expect = Config::default();
            expect.nudges.water.on = false;
            assert_eq!(
                loaded,
                Loaded {
                    config: expect,
                    notice: true
                },
                "{nudge}.{key} = {value}"
            );
        }
    }

    #[test]
    fn the_range_edges_are_allowed() {
        for (nudge, key, value) in [
            ("stand", "everyMinutes", 10),
            ("stand", "everyMinutes", 240),
            ("pause", "everyMinutes", 20),
            ("pause", "everyMinutes", 240),
            ("water", "everyMinutes", 10),
            ("water", "everyMinutes", 240),
            ("stand", "lengthMinutes", 2),
            ("stand", "lengthMinutes", 5),
            ("pause", "lengthMinutes", 2),
            ("pause", "lengthMinutes", 15),
        ] {
            let loaded = read_edited(|f| f[nudge][key] = json!(value));
            assert!(!loaded.notice, "{nudge}.{key} = {value}");
            let written: Value = serde_json::from_str(&loaded.config.to_json()).unwrap();
            assert_eq!(written[nudge][key], json!(value));
        }
    }

    #[test]
    fn a_malformed_value_falls_back_on_its_own() {
        let edits: [fn(&mut Value); 15] = [
            |f| f["stand"]["everyMinutes"] = json!("30"),
            |f| f["stand"]["everyMinutes"] = json!(30.5),
            |f| f["stand"]["everyMinutes"] = json!(null),
            |f| f["stand"]["on"] = json!("yes"),
            |f| f["stand"]["enforcement"] = json!("hard"),
            |f| f["pause"]["enforcement"] = json!("Firm"),
            |f| f["workDays"] = json!(["sun", "mon", "funday"]),
            |f| f["workDays"] = json!([]),
            |f| f["workDays"] = json!("sun"),
            |f| f["workDays"] = json!(["sun", 1]),
            |f| f["workHours"]["start"] = json!("8:00"),
            |f| f["workHours"]["end"] = json!("24:00"),
            |f| f["workHours"] = json!("08:00-17:00"),
            |f| f["offUntil"] = json!("tomorrow"),
            |f| f["overtimeUntil"] = json!(1_793_700_000),
        ];
        for edit in edits {
            let mut file = spec_example();
            edit(&mut file);
            let loaded = read_edited(|f| {
                edit(f);
                f["water"]["on"] = json!(false);
            });
            let mut expect = Config::default();
            expect.nudges.water.on = false;
            assert_eq!(
                loaded,
                Loaded {
                    config: expect,
                    notice: true
                },
                "{file}"
            );
        }
    }

    #[test]
    fn an_end_not_after_the_start_resets_both_hours() {
        for (start, end) in [("09:00", "09:00"), ("17:00", "08:00")] {
            assert_falls_back(
                |f| {
                    f["workHours"] = json!({ "start": start, "end": end });
                    f["workDays"] = json!(["mon"]);
                },
                |c| {
                    c.work_days = WorkDays([false; 7]);
                    c.work_days.set(Weekday::Mon, true);
                },
            );
        }
    }

    #[test]
    fn repeated_work_days_are_harmless() {
        let loaded = read_edited(|f| f["workDays"] = json!(["sat", "sat"]));
        assert!(!loaded.notice);
        assert_eq!(
            loaded.config.work_days.iter().collect::<Vec<_>>(),
            [Weekday::Sat]
        );
    }

    #[test]
    fn both_end_moments_set_are_both_ignored() {
        assert_falls_back(
            |f| {
                f["offUntil"] = json!("2026-11-04T08:00:00+03:00");
                f["overtimeUntil"] = json!("2026-11-04T00:00:00+03:00");
            },
            |_| {},
        );
    }

    #[test]
    fn a_past_end_moment_is_ignored_silently() {
        for key in ["offUntil", "overtimeUntil"] {
            for past in ["2026-11-03T10:00:00+03:00", "2026-11-02T08:00:00+03:00"] {
                assert_eq!(
                    read_edited(|f| f[key] = json!(past)),
                    Loaded {
                        config: Config::default(),
                        notice: false
                    },
                    "{key} = {past}"
                );
            }
        }
    }

    #[test]
    fn a_future_end_moment_applies_whatever_its_offset() {
        // 07:30 UTC is 10:30 at +03:00: half an hour from now.
        let loaded = read_edited(|f| f["overtimeUntil"] = json!("2026-11-03T07:30:00Z"));
        assert!(!loaded.notice);
        assert_eq!(
            loaded.config.overtime_until,
            Some(at("2026-11-03T07:30:00Z"))
        );
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let loaded = read_edited(|f| {
            f["theme"] = json!("dark");
            f["stand"]["sound"] = json!(true);
        });
        assert_eq!(
            loaded,
            Loaded {
                config: Config::default(),
                notice: false
            }
        );
    }

    #[test]
    fn each_profile_derives_from_its_own_numbers() {
        for profile in Profile::ALL {
            assert_eq!(profile.nudges().profile(), Some(profile));
        }
        let strict = Profile::Strict.nudges();
        assert_eq!(
            (strict.stand.every_minutes, strict.pause.every_minutes),
            (30, 30)
        );
        assert_eq!(
            (strict.stand.enforcement, strict.pause.enforcement),
            (Enforcement::Firm, Enforcement::Firm)
        );
        let gentle = Profile::Gentle.nudges();
        assert_eq!(
            (gentle.stand.every_minutes, gentle.pause.every_minutes),
            (45, 90)
        );
        assert_eq!(gentle.pause.enforcement, Enforcement::Soft);
    }

    #[test]
    fn on_and_off_do_not_change_the_profile() {
        let mut nudges = Profile::Strict.nudges();
        nudges.stand.on = false;
        nudges.water.on = false;
        assert_eq!(nudges.profile(), Some(Profile::Strict));
    }

    #[test]
    fn balanced_with_stand_move_firm_is_custom() {
        let mut nudges = Profile::Balanced.nudges();
        nudges.stand.enforcement = Enforcement::Firm;
        assert_eq!(nudges.profile(), None);
    }

    #[test]
    fn any_number_off_preset_is_custom() {
        let edits: [fn(&mut Nudges); 6] = [
            |n| n.stand.every_minutes = 31,
            |n| n.stand.length_minutes = 3,
            |n| n.pause.every_minutes = 90,
            |n| n.pause.length_minutes = 10,
            |n| n.pause.enforcement = Enforcement::Soft,
            |n| n.water.every_minutes = 45,
        ];
        for edit in edits {
            let mut nudges = Profile::Balanced.nudges();
            edit(&mut nudges);
            assert_eq!(nudges.profile(), None, "{nudges:?}");
        }
    }

    #[test]
    fn applying_a_profile_leaves_on_and_off_alone() {
        let mut nudges = Nudges::default();
        nudges.pause.on = false;
        nudges.stand.length_minutes = 4;
        nudges.apply(Profile::Gentle);
        let mut expect = Profile::Gentle.nudges();
        expect.pause.on = false;
        assert_eq!(nudges, expect);
    }

    /// A fresh folder under the system temp folder, removed on drop.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("breakpoint-test-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn save_creates_the_folder_and_load_reads_it_back() {
        let tmp = TempDir::new("save");
        let path = tmp.0.join("breakpoint").join("config.json");
        assert_eq!(load(&path, now()), None, "no file is first run");

        let mut config = Config::default();
        config.nudges.apply(Profile::Strict);
        save(&path, &config).unwrap();
        assert_eq!(
            load(&path, now()),
            Some(Loaded {
                config: config.clone(),
                notice: false
            })
        );

        config.nudges.water.on = false;
        save(&path, &config).unwrap();
        assert_eq!(load(&path, now()).unwrap().config, config);
        assert_eq!(fs::read_to_string(&path).unwrap(), config.to_json());
        assert_eq!(
            fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1,
            "no temporary file left behind"
        );
    }

    #[test]
    fn loading_an_unparseable_file_leaves_it_untouched() {
        let tmp = TempDir::new("corrupt");
        fs::create_dir_all(&tmp.0).unwrap();
        let path = tmp.0.join("config.json");
        let bytes = b"{\"workDays\": [\"sun\" \xff\xfe garbage";
        fs::write(&path, bytes).unwrap();

        assert_eq!(
            load(&path, now()),
            Some(Loaded {
                config: Config::default(),
                notice: true
            })
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
