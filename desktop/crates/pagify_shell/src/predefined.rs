//! The words somebody types into forms often enough to keep.
//!
//! A name, an address, an email, a reference number — the things filling a form
//! in means typing again and again. Kept so they are typed once.
//!
//! # Nothing is remembered without being asked for
//!
//! The obvious design is to record everything the typewriter tool writes, the
//! way a browser records a form. It is not what this does. What goes into a
//! form field is somebody's name, their address, an account number — and a
//! program that quietly copies all of that into a file on disk, because it
//! might be convenient later, has made a decision that was not its to make.
//!
//! So a snippet is kept only when somebody hands it to this tool. Nothing
//! written with `addtext` reaches here.
//!
//! # Why the order is the record
//!
//! Most recently used first, and that ordering *is* the list — there is no
//! separate "current" or "favourite" flag. A flag can name a snippet that has
//! since been deleted; an order cannot. The same reasoning as
//! [`crate::signatures::Signatures::current`].
//!
//! Like every other list this program keeps, it lives in the platform's own
//! settings directory and does not leave the machine.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The snippets somebody has kept, most recently used first.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Predefined {
    entries: Vec<String>,
}

impl Predefined {
    /// Most recently used first.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The one a bare press of the tool would use.
    pub fn current(&self) -> Option<&str> {
        self.entries.first().map(String::as_str)
    }

    /// Keep a snippet, or move it to the front if it is already kept.
    ///
    /// Blank text is not a snippet. Text that differs only in the spaces around
    /// it is the same snippet — otherwise a list fills up with what looks like
    /// one entry repeated.
    ///
    /// Returns whether anything is now kept that was not before.
    pub fn remember(&mut self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        let already = self.entries.iter().position(|e| e == text);
        match already {
            Some(at) => {
                let moved = self.entries.remove(at);
                self.entries.insert(0, moved);
                false
            }
            None => {
                self.entries.insert(0, text.to_string());
                true
            }
        }
    }

    pub fn forget(&mut self, text: &str) -> bool {
        let text = text.trim();
        let before = self.entries.len();
        self.entries.retain(|e| e != text);
        self.entries.len() != before
    }

    // -- persistence, same shape as `Recent` ---------------------------------

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
        base.map(|b| b.join("Pagify").join("predefined.json"))
    }

    /// A missing or unreadable file is an empty list, never an error.
    pub fn load_from(path: &std::path::Path) -> Predefined {
        let Ok(text) = std::fs::read_to_string(path) else { return Predefined::default() };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn load() -> Predefined {
        match Predefined::path() {
            Some(path) => Predefined::load_from(&path),
            None => Predefined::default(),
        }
    }

    /// Write the list, and say if it could not be written — the same reasoning
    /// as [`crate::signatures::Signatures::save_to`].
    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_used_last_is_offered_first() {
        let mut kept = Predefined::default();
        assert!(kept.current().is_none());

        assert!(kept.remember("Jane Smith"));
        assert!(kept.remember("jane@example.com"));
        assert_eq!(kept.current(), Some("jane@example.com"));

        // Using an older one brings it back to the front.
        assert!(!kept.remember("Jane Smith"), "it was already kept");
        assert_eq!(kept.current(), Some("Jane Smith"));
        assert_eq!(kept.entries().len(), 2, "using one made a second copy");
    }

    #[test]
    fn the_same_words_with_different_spaces_are_the_same_snippet() {
        let mut kept = Predefined::default();
        kept.remember("Jane Smith");
        kept.remember("  Jane Smith  ");
        assert_eq!(kept.entries(), ["Jane Smith"]);
    }

    #[test]
    fn blank_is_not_a_snippet() {
        let mut kept = Predefined::default();
        assert!(!kept.remember(""));
        assert!(!kept.remember("   \n "));
        assert!(kept.is_empty());
    }

    #[test]
    fn forgetting_one_leaves_the_rest() {
        let mut kept = Predefined::default();
        kept.remember("first");
        kept.remember("second");
        assert!(kept.forget("first"));
        assert!(!kept.forget("first"));
        assert_eq!(kept.entries(), ["second"]);
        assert_eq!(kept.current(), Some("second"));
    }

    /// Several lines is one snippet — an address is the case this exists for.
    #[test]
    fn a_snippet_can_be_more_than_one_line() {
        let mut kept = Predefined::default();
        kept.remember("Unit 4\nBoundary Road\nLE1 2AB");
        assert_eq!(kept.entries().len(), 1);
        assert!(kept.current().is_some_and(|t| t.lines().count() == 3));
    }

    #[test]
    fn the_list_survives_the_round_trip_to_disk() {
        let dir = std::env::temp_dir().join(format!("pagify-predefined-{}", std::process::id()));
        let path = dir.join("predefined.json");
        let mut kept = Predefined::default();
        kept.remember("Jane Smith");
        kept.save_to(&path).expect("write");

        assert_eq!(Predefined::load_from(&path), kept);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupted_file_is_an_empty_list_rather_than_a_failure_to_start() {
        let dir =
            std::env::temp_dir().join(format!("pagify-predefined-bad-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("predefined.json");
        std::fs::write(&path, b"not json at all").expect("write");
        assert!(Predefined::load_from(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
