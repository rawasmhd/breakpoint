//! The event log's files: `%APPDATA%\breakpoint\log\YYYY-MM.jsonl` (SPEC §10.2).
//!
//! Best effort throughout. Each line opens and closes the file, so deleting a file or the whole
//! folder while breakpoint runs is harmless: the next line recreates it. A failed write is
//! dropped silently; nudges never depend on the log. The lines themselves come from
//! [`breakpoint::core::log`].

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use breakpoint::core::log::{self, Today};
use breakpoint::core::scheduler::Event;
use breakpoint::core::time::LocalTime;

pub struct Log {
    /// The `log` folder; `None` if `%APPDATA%` is unknown, and then nothing is written or read.
    dir: Option<PathBuf>,
}

impl Log {
    /// The log under `%APPDATA%\breakpoint\`.
    pub fn new() -> Self {
        let dir = std::env::var_os("APPDATA")
            .map(|app_data| PathBuf::from(app_data).join("breakpoint").join("log"));
        Self { dir }
    }

    /// Reads what start-up needs: the highest `occ` in the newest file (0 if there is none), for
    /// the scheduler to continue from, and the Today count for `now`, rebuilt from the current
    /// month's file.
    pub fn start(&self, now: LocalTime) -> (u64, Today) {
        let Some(dir) = &self.dir else {
            return (0, Today::rebuild("", now));
        };
        let names: Vec<String> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .collect();
        let last = log::newest_file(names.iter().map(String::as_str))
            .and_then(|name| log::last_occ(&read(&dir.join(name))))
            .unwrap_or(0);
        let today = Today::rebuild(&read(&dir.join(log::file_name(now))), now);
        (last, today)
    }

    /// Appends the line for `event` at `t` to its month's file, creating the folder and the file
    /// if needed.
    pub fn append(&self, t: LocalTime, event: &Event) {
        let Some(dir) = &self.dir else { return };
        let write = || -> std::io::Result<()> {
            fs::create_dir_all(dir)?;
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(log::file_name(t)))?
                .write_all(log::line(t, event).as_bytes())
        };
        let _ = write();
    }
}

/// A log file's text, or nothing if it cannot be read. Bytes that are not UTF-8 spoil only their
/// own line, which the readers then skip.
fn read(path: &Path) -> String {
    fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use breakpoint::core::scheduler::{Nudge, SatisfyCause};

    use super::*;

    fn at(s: &str) -> LocalTime {
        LocalTime::parse_rfc3339(s).unwrap()
    }

    /// A fresh scratch `log` folder under the system temp folder.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("breakpoint-test-{}-{name}", std::process::id()))
            .join("log");
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn appends_go_to_the_months_file_and_survive_the_folder_being_deleted() {
        let dir = scratch("append");
        let log = Log {
            dir: Some(dir.clone()),
        };
        let october = at("2026-10-31T23:59:00+03:00");
        let november = at("2026-11-01T00:01:00+03:00");
        let fired = Event::Fired {
            nudge: Nudge::Stand,
            occ: 7,
            into: None,
        };
        log.append(october, &fired);
        fs::remove_dir_all(&dir).unwrap();
        log.append(october, &fired);
        log.append(
            november,
            &Event::Satisfied {
                nudge: Nudge::Stand,
                occ: 7,
                by: SatisfyCause::Away,
            },
        );
        let oct = fs::read_to_string(dir.join("2026-10.jsonl")).unwrap();
        assert_eq!(oct, log::line(october, &fired));
        let nov = fs::read_to_string(dir.join("2026-11.jsonl")).unwrap();
        assert!(nov.ends_with("\"by\":\"away\"}\n"), "{nov}");

        // Start-up in November: occ goes on from the newest file; Today counts nothing because
        // Stand/Move 7 fired in October.
        let (last, today) = log.start(november);
        assert_eq!(last, 7);
        assert_eq!(today.text(november), "Today: 0 taken, 0 skipped");
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn with_no_folder_start_up_begins_at_occ_1() {
        let dir = scratch("empty");
        let log = Log { dir: Some(dir) };
        let now = at("2026-11-02T09:00:00+03:00");
        let (last, today) = log.start(now);
        assert_eq!(last, 0);
        assert_eq!(today.text(now), "Today: 0 taken, 0 skipped");
    }
}
