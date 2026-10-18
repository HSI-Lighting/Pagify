//! The Form tab's Tool Settings — the one preference this build stores.
//!
//! **Why one switch and not a page of them.** A "Tool Settings" panel that
//! stores nothing is a button pretending to be a feature. Highlighting the boxes
//! a click can fill is the setting this build has behind it: the rectangles are
//! the same ones filling hit-tests against, the wash is drawn over the page and
//! never written to it, and the choice is remembered in
//! `state_dir()/settings.json` — Pagify's own directory, never beside the
//! document and never beside the executable.
//!
//! An absent or unreadable file is the default, never an error — the same rule
//! `recent.json`, `predefined.json` and the signature list follow: a corrupted
//! settings file must not stop the program starting.

use std::path::{Path, PathBuf};

/// What the Tool Settings panel holds. A second setting joins it here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ToolSettings {
    /// Draw a light wash over every box a click can fill — text fields, tick
    /// boxes, radios and choice lists — so filling a form shows where its
    /// boxes are. Drawn over the page, never written into it.
    pub highlight_fields: bool,
}

impl ToolSettings {
    /// Where the settings live. Under the platform's own per-user config
    /// directory, not next to a document or the executable.
    pub fn path() -> Option<PathBuf> {
        crate::state::state_dir().map(|dir| dir.join("settings.json"))
    }

    /// A missing or unreadable file is the default, never an error.
    pub fn load_from(path: &Path) -> ToolSettings {
        let Ok(text) = std::fs::read_to_string(path) else {
            return ToolSettings::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn load() -> ToolSettings {
        match ToolSettings::path() {
            Some(path) => ToolSettings::load_from(&path),
            None => ToolSettings::default(),
        }
    }

    /// Write the settings, and say if they could not be written — the same
    /// contract as every other file Pagify keeps.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        crate::state::write_own(path, text.as_bytes())
    }

    /// The switch as the word the panel and the command's answer use.
    pub fn highlight_words(&self) -> &'static str {
        if self.highlight_fields {
            "on"
        } else {
            "off"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pagify-tool-settings-{name}-{}.json",
            std::process::id()
        ))
    }

    #[test]
    fn highlighting_is_off_until_it_is_asked_for() {
        let settings = ToolSettings::default();
        assert!(!settings.highlight_fields);
        assert_eq!(settings.highlight_words(), "off");
    }

    /// What the panel sets is what the next launch reads: the file on disk is
    /// the record.
    #[test]
    fn the_choice_survives_being_written_and_read_back() {
        let path = scratch("round-trip");
        let settings = ToolSettings { highlight_fields: true };
        settings.save_to(&path).expect("save");
        assert_eq!(ToolSettings::load_from(&path), settings);
        assert_eq!(ToolSettings::load_from(&path).highlight_words(), "on");
        let written = std::fs::read_to_string(&path).expect("read the file");
        assert!(written.contains("highlight_fields"), "the field's name changed: {written}");
        std::fs::remove_file(&path).ok();
    }

    /// A missing file and a corrupted one are both the default — never an
    /// error, and never a reason the program does not start.
    #[test]
    fn a_missing_or_corrupt_file_is_the_default() {
        let missing = ToolSettings::load_from(std::path::Path::new(
            "/nonexistent/pagify/settings-that-cannot-exist.json",
        ));
        assert_eq!(missing, ToolSettings::default());

        let path = scratch("corrupt");
        std::fs::write(&path, b"{ not json at all").expect("write the corrupt file");
        assert_eq!(ToolSettings::load_from(&path), ToolSettings::default());
        std::fs::remove_file(&path).ok();
    }
}
