//! Recent documents.
//!
//! The mockup's largest panel, and the thing a PDF editor is judged on before
//! anything else: opening the file you had last time should take one click.
//!
//! Kept in the shell rather than the app because all of it is logic — dedupe by
//! path, newest first, cap the list, and drop entries whose file has since been
//! moved or deleted. A list that offers a file that is not there any more is
//! worse than a short list.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many to remember. Long enough to cover a working week, short enough that
/// the panel never needs its own search.
const LIMIT: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    /// Seconds since the epoch, as reported when the file was last opened.
    /// Stored rather than read from the filesystem so the list can be ordered
    /// without touching a disk that may be a slow network share.
    pub opened_at: u64,
    #[serde(default)]
    pub pages: usize,
}

impl Entry {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }

    /// The containing directory, with the home directory shortened to `~`.
    pub fn location(&self) -> String {
        let parent = self.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let shown = parent.to_string_lossy().into_owned();

        match std::env::var("HOME") {
            Ok(home) if !home.is_empty() && shown.starts_with(&home) => {
                format!("~{}", &shown[home.len()..])
            }
            _ => shown,
        }
    }

    pub fn exists(&self) -> bool {
        self.path.is_file()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Recent {
    #[serde(default)]
    pub entries: Vec<Entry>,
}

impl Recent {
    /// Record an open. Newest first, no duplicates.
    pub fn record(&mut self, path: &Path, pages: usize, at: u64) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        // The same file opened twice is one entry, moved to the top — not two.
        self.entries.retain(|e| e.path != path);
        self.entries.insert(0, Entry { path, opened_at: at, pages });
        self.entries.truncate(LIMIT);
    }

    pub fn forget(&mut self, path: &Path) {
        self.entries.retain(|e| e.path != path);
    }

    /// Only the entries whose file is still where it was.
    pub fn present(&self) -> Vec<&Entry> {
        self.entries.iter().filter(|e| e.exists()).collect()
    }

    /// Drop everything that has gone missing, and say how many.
    pub fn prune(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(Entry::exists);
        before - self.entries.len()
    }

    // -- persistence --------------------------------------------------------

    /// Where the list lives. Under the platform's own config directory, not
    /// next to the executable — a bundle should be read-only, and on Windows it
    /// is in Program Files where it certainly is.
    pub fn path() -> Option<PathBuf> {
        let base = if cfg!(target_os = "macos") {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
        } else if cfg!(target_os = "windows") {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        };
        base.map(|b| b.join("Pagify").join("recent.json"))
    }

    /// Read the list. A missing or unreadable file is an empty list, never an
    /// error — a corrupted recents file must not stop the program starting.
    pub fn load() -> Recent {
        let Some(path) = Recent::path() else { return Recent::default() };
        let Ok(text) = std::fs::read_to_string(path) else { return Recent::default() };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Recent::path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

/// Seconds since the epoch, for stamping an entry.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `2026-08-31 14:23`, from a Unix timestamp, in UTC.
///
/// Hand-rolled rather than pulling in a date crate for one format string. UTC
/// rather than local because a wrong local time needs a timezone database and a
/// wrong one here is a misleading timestamp on someone's document list.
pub fn format_time(seconds: u64) -> String {
    let days = seconds / 86_400;
    let rest = seconds % 86_400;
    let (hour, minute) = (rest / 3600, (rest % 3600) / 60);

    // Civil-from-days, Howard Hinnant's algorithm.
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02} {hour:02}:{minute:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_file_opened_twice_is_one_entry_at_the_top() {
        let mut recent = Recent::default();
        recent.record(Path::new("/tmp/a.pdf"), 3, 100);
        recent.record(Path::new("/tmp/b.pdf"), 1, 200);
        recent.record(Path::new("/tmp/a.pdf"), 3, 300);

        assert_eq!(recent.entries.len(), 2);
        assert_eq!(recent.entries[0].path, Path::new("/tmp/a.pdf"));
        assert_eq!(recent.entries[0].opened_at, 300);
    }

    #[test]
    fn the_list_is_capped() {
        let mut recent = Recent::default();
        for i in 0..(LIMIT + 20) {
            recent.record(Path::new(&format!("/tmp/{i}.pdf")), 1, i as u64);
        }
        assert_eq!(recent.entries.len(), LIMIT);
        // The newest survived, the oldest did not.
        assert!(recent.entries[0].path.ends_with(&format!("{}.pdf", LIMIT + 19)));
    }

    #[test]
    fn files_that_have_gone_are_not_offered() {
        // A list that offers a file that is not there is worse than a short one.
        let mut recent = Recent::default();
        recent.record(Path::new("/tmp/definitely-not-here-9f3a.pdf"), 1, 1);
        assert!(recent.present().is_empty());
        assert_eq!(recent.prune(), 1);
        assert!(recent.entries.is_empty());
    }

    #[test]
    fn a_real_file_is_offered() {
        let dir = std::env::temp_dir().join("pagify-recent-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("here.pdf");
        std::fs::write(&file, b"%PDF-1.4\n").unwrap();

        let mut recent = Recent::default();
        recent.record(&file, 2, 5);
        assert_eq!(recent.present().len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_entry_shows_a_name_and_a_shortened_location() {
        let entry = Entry {
            path: PathBuf::from("/Users/someone/Downloads/HSI CATALOG 2026.pdf"),
            opened_at: 0,
            pages: 12,
        };
        assert_eq!(entry.name(), "HSI CATALOG 2026.pdf");
        assert!(entry.location().ends_with("/Downloads"));
    }

    #[test]
    fn a_corrupt_recents_file_is_an_empty_list_not_a_crash() {
        assert!(serde_json::from_str::<Recent>("{ this is not json").is_err());
        // …which `load` turns into a default rather than propagating.
        let fallback: Recent = serde_json::from_str("{ nope").unwrap_or_default();
        assert!(fallback.entries.is_empty());
    }

    #[test]
    fn timestamps_format_the_way_the_panel_shows_them() {
        // 2026-08-31 14:23 UTC.
        assert_eq!(format_time(1_788_186_180), "2026-08-31 14:23");
        assert_eq!(format_time(0), "1970-01-01 00:00");
    }
}
