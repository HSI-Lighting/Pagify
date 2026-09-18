//! A running, on-disk transcript of what the command box has said.
//!
//! **Not** [`crate::automate`]. That module's whole premise is a script of
//! *recordable* lines — text that can be typed back in and replayed, and
//! deliberately nothing else, because a line that could not be typed could
//! not be replayed either. A bug report needs the other half: what the app
//! actually *said happened* — a resize that landed as a move, a font that
//! could not be spelled, a refusal and its reason — almost none of which is
//! typeable, so none of it belongs in a script. Mixing the two would also
//! make Automate's own scripts unreplayable the moment an outcome line got
//! mistaken for a command.
//!
//! Written as it happens, one JSON object per line, so a crash mid-bug loses
//! nothing already flushed — the whole reason this is a file and not just the
//! in-memory scrollback `CommandBox::history` already keeps, which is gone
//! the moment the window closes.

use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// How many past sessions' logs to keep. Bounded for the same reason
/// `CommandBox`'s own in-memory history is capped: a machine that has had
/// Pagify open for months should not carry every session it ever ran.
const KEEP: usize = 20;

#[derive(Serialize)]
struct LogLine<'a> {
    /// Milliseconds since this session's log started — not wall-clock, so a
    /// reader is never left converting time zones to see what happened first.
    /// The file name carries the session's own start time for whoever needs
    /// to line logs up against something else.
    at_ms: u128,
    /// "command" for a line the command box dispatched (typed or run from a
    /// ribbon button — see the two call sites in `pagify_app`, which is where
    /// every command actually starts); "info" or "error" for what the app
    /// said about it, in `CommandBox::Kind`'s own words.
    kind: &'a str,
    text: &'a str,
}

/// Where these logs live: under the platform's own config directory,
/// alongside `recent.json` and the rest — see [`crate::state::state_dir`].
fn dir() -> Option<PathBuf> {
    crate::state::state_dir().map(|d| d.join("session-logs"))
}

fn unix_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// This run's transcript. Degrades to doing nothing rather than failing —
/// a read-only profile or a full disk must not stop the program starting,
/// the same choice [`crate::recent::Recent::load`] makes for a file it
/// cannot read.
pub struct SessionLog {
    file: Option<File>,
    path: Option<PathBuf>,
    started: Instant,
}

impl Default for SessionLog {
    fn default() -> Self {
        SessionLog { file: None, path: None, started: Instant::now() }
    }
}

impl SessionLog {
    /// Start a fresh log for this run, pruning old ones first.
    pub fn start() -> SessionLog {
        match dir() {
            Some(dir) => Self::start_in(dir),
            None => SessionLog::default(),
        }
    }

    /// The same, in a directory named explicitly rather than
    /// [`state_dir`][crate::state::state_dir] — the seam a caller outside
    /// this module uses to prove its own wiring calls [`Self::record`] at
    /// the right moments, against a directory a test controls, without
    /// touching the real one `start` writes under.
    pub fn start_in(dir: PathBuf) -> SessionLog {
        let _ = std::fs::create_dir_all(&dir);
        Self::prune(&dir);

        let path = dir.join(format!("session-{}-{}.jsonl", unix_ms(), std::process::id()));
        let file = OpenOptions::new().create(true).append(true).open(&path).ok();
        let path = file.is_some().then_some(path);
        SessionLog { file, path, started: Instant::now() }
    }

    /// Delete the oldest logs once there are more than [`KEEP`] — by name,
    /// not by reading each file's own timestamp: the file name already
    /// carries it, sorts the same way numerically as it does in time (both
    /// are the same fixed-width-free millisecond count), and needs no
    /// metadata call per file.
    fn prune(dir: &std::path::Path) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut logs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
            .collect();
        if logs.len() < KEEP {
            return;
        }
        logs.sort();
        for old in &logs[..logs.len() - (KEEP - 1)] {
            let _ = std::fs::remove_file(old);
        }
    }

    /// Where this run is writing to, so the app can tell someone where to
    /// find it. `None` means logging could not start — a read-only profile,
    /// most likely — and every [`Self::record`] call this run is a no-op.
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    /// Append one line. Flushed immediately: a log a crash can still lose the
    /// last few lines of is a log that stops being trustworthy right where a
    /// crash bug would need it most.
    pub fn record(&mut self, kind: &str, text: &str) {
        let Some(file) = &mut self.file else { return };
        let line = LogLine { at_ms: self.started.elapsed().as_millis(), kind, text };
        let Ok(json) = serde_json::to_string(&line) else { return };
        let _ = writeln!(file, "{json}");
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not `SessionLog::start()`: that writes under the real config
    /// directory, which a unit test must not touch. This drives the same
    /// `record`/file shape directly against a temp file instead.
    fn log_at(path: &std::path::Path) -> SessionLog {
        let file = OpenOptions::new().create(true).append(true).open(path).ok();
        SessionLog { file, path: Some(path.to_path_buf()), started: Instant::now() }
    }

    #[test]
    fn a_recorded_line_is_one_json_object_readable_back() {
        let path = std::env::temp_dir()
            .join(format!("pagify-test-session-log-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut log = log_at(&path);

        log.record("command", "editobject");
        log.record("info", "edit object: click a picture or shape to select it.");

        let text = std::fs::read_to_string(&path).expect("the file exists");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "one JSON object per record: {text}");

        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("valid json");
        assert_eq!(first["kind"], "command");
        assert_eq!(first["text"], "editobject");
        assert!(first["at_ms"].is_number());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_log_with_nowhere_to_write_does_not_panic() {
        let mut log = SessionLog::default();
        assert!(log.path().is_none());
        log.record("info", "nothing to see");
    }

    #[test]
    fn pruning_keeps_only_the_newest_sessions() {
        let dir = std::env::temp_dir().join(format!("pagify-test-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        for i in 0..KEEP + 5 {
            std::fs::write(dir.join(format!("session-{i:04}-1.jsonl")), b"{}").unwrap();
        }
        SessionLog::prune(&dir);

        // One short of `KEEP`: `prune` runs just before a new file is
        // written, leaving room for it to bring the count back to `KEEP`.
        let left = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(left, KEEP - 1, "pruning should leave room for one more, found {left}");

        // And it kept the newest ones, not an arbitrary subset.
        assert!(dir.join(format!("session-{:04}-1.jsonl", KEEP + 4)).exists());
        assert!(!dir.join("session-0000-1.jsonl").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
