//! Light or dark — the one UI preference Pagify remembers across launches.
//!
//! Lives here rather than in `pagify_app` even though it carries no document
//! meaning at all: this crate's own charter is "if logic can be tested
//! without a window, it must live here" (see `lib.rs`), and a preference file
//! is exactly that. `recent.rs` and `signatures.rs` already own this same
//! shape — `path()`/`load_from()`/`save_to()` over [`crate::state::state_dir`]
//! — so this reuses it rather than adding a second, `pagify_app`-local
//! version of the identical mechanism for the sake of a purity distinction
//! that wouldn't pay for itself.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Dark,
    Light,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    mode: Mode,
}

pub fn path() -> Option<PathBuf> {
    crate::state::state_dir().map(|dir| dir.join("appearance.json"))
}

/// A missing or unreadable file is Dark, never an error — the same
/// "a corrupted preferences file must not stop the program starting" rule
/// [`crate::recent::Recent::load`]/[`crate::signatures::Signatures::load_from`]
/// already follow.
pub fn load_from(path: &Path) -> Mode {
    let Ok(text) = std::fs::read_to_string(path) else { return Mode::Dark };
    serde_json::from_str::<Stored>(&text).map(|s| s.mode).unwrap_or(Mode::Dark)
}

pub fn load() -> Mode {
    match path() {
        Some(path) => load_from(&path),
        None => Mode::Dark,
    }
}

pub fn save_to(path: &Path, mode: Mode) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(&Stored { mode })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    crate::state::write_own(path, text.as_bytes())
}

pub fn save(mode: Mode) {
    if let Some(path) = path() {
        let _ = save_to(&path, mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_loads_as_dark() {
        let dir = std::env::temp_dir().join(format!("pagify-appearance-missing-{}", std::process::id()));
        assert_eq!(load_from(&dir.join("appearance.json")), Mode::Dark);
    }

    #[test]
    fn a_saved_mode_loads_back_the_same() {
        let dir = std::env::temp_dir().join(format!("pagify-appearance-{}", std::process::id()));
        let path = dir.join("appearance.json");
        save_to(&path, Mode::Light).expect("write");
        assert_eq!(load_from(&path), Mode::Light);
        save_to(&path, Mode::Dark).expect("write");
        assert_eq!(load_from(&path), Mode::Dark);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_loads_as_dark_rather_than_failing() {
        let dir = std::env::temp_dir().join(format!("pagify-appearance-bad-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("appearance.json");
        std::fs::write(&path, b"not json").expect("write");
        assert_eq!(load_from(&path), Mode::Dark);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
