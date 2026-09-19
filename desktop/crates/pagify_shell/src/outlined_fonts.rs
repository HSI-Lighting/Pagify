//! Extra outlined-text fonts a reader has added, beyond the ones this build
//! bundles.
//!
//! The bundled faces — see `pagify_app`'s `BUNDLED_OUTLINED_FONTS` — cover the
//! typeface they were measured against. A reader's own document may use
//! something else entirely: an in-house catalogue face, a client's brand
//! font. Vector matching only resolves a page drawn in a font the catalogue
//! actually has glyphs from, so the fix is letting a reader supply that file
//! rather than widening the bundle to guess at every typeface in the world.
//!
//! Kept in the shell for the same reason `recent` is: it is a fact about the
//! program, not about a window, and belongs where it can be tested without
//! one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutlinedFonts {
    #[serde(default)]
    pub paths: Vec<PathBuf>,
}

impl OutlinedFonts {
    /// Add a font. Rejected only for the reasons a reader would want to know
    /// about immediately: the file cannot be read, or it is already on the
    /// list. Whether it actually resolves anything is not checked here — a
    /// font that shares no glyph shapes with a given page simply never wins a
    /// match there, which is a silent no-op rather than a reason to refuse
    /// adding it.
    ///
    /// Stored exactly as given, deliberately not canonicalised: `remove`
    /// takes a path from the same UI that displayed this one, and it must
    /// still match after the file itself has gone missing, when
    /// canonicalising the incoming path is no longer possible at all. The
    /// duplicate check below still canonicalises for the comparison, since
    /// that is a same-call, file-still-there question.
    pub fn add(&mut self, path: PathBuf) -> Result<(), String> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        let duplicate = self.paths.iter().any(|existing| {
            existing == &path || existing.canonicalize().unwrap_or_else(|_| existing.clone()) == canonical
        });
        if duplicate {
            return Err(format!("{} is already on the list.", path.display()));
        }
        std::fs::read(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        self.paths.push(path);
        Ok(())
    }

    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
    }

    pub fn clear(&mut self) {
        self.paths.clear();
    }

    /// Only the ones still where they were added from. A font that has since
    /// been moved or deleted is silently left out — offering it as a choice
    /// for `remove`, or feeding it to a catalogue build that can only fail on
    /// it, would not be a service.
    pub fn present(&self) -> Vec<&PathBuf> {
        self.paths.iter().filter(|p| p.is_file()).collect()
    }

    /// Every present font's bytes, ready to merge into a catalogue alongside
    /// the bundled ones.
    ///
    /// Read fresh each call rather than cached: these are a handful of files
    /// a few hundred KB each, added rarely, feeding into a catalogue build
    /// already measured in seconds — the read is noise next to that. A cache
    /// would also mean a font edited or replaced on disk keeps matching
    /// against its old bytes until the process restarts.
    pub fn bytes(&self) -> Vec<Vec<u8>> {
        self.present().into_iter().filter_map(|p| std::fs::read(p).ok()).collect()
    }

    // -- persistence, same shape as `Recent` ---------------------------------

    /// Where the list lives: beside the other state files, under the one
    /// folder [`crate::state::state_dir`] names. This used to build the path
    /// again for itself, which is how `outlined_fonts.json` came to be missed
    /// by `clearhistory` and by the 0600 rule. Found by audit.
    pub fn path() -> Option<PathBuf> {
        crate::state::state_dir().map(|dir| dir.join("outlined_fonts.json"))
    }

    /// A missing or unreadable file is an empty list, never an error — a
    /// corrupted settings file must not stop the program starting.
    pub fn load() -> OutlinedFonts {
        let Some(path) = OutlinedFonts::path() else { return OutlinedFonts::default() };
        let Ok(text) = std::fs::read_to_string(path) else { return OutlinedFonts::default() };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Written the way every other state file is: through
    /// [`crate::state::write_own`], which creates it 0600 on Unix. A bare
    /// `std::fs::write` here left it at the umask's 0644. Found by audit.
    pub fn save(&self) {
        let Some(path) = OutlinedFonts::path() else { return };
        self.save_to(&path);
    }

    /// The write itself, so a test can aim it at a file it owns rather than
    /// at the reader's real settings directory.
    fn save_to(&self, path: &Path) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = crate::state::write_own(path, text.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font file under a fresh temp directory, named per-test so parallel
    /// test threads do not collide — the bytes only need to be *readable*,
    /// not a real font, since `add` never parses them; see its own doc
    /// comment for why.
    fn a_real_font(unique: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("pagify-outlined-fonts-test-{unique}"));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Some Font.ttf");
        std::fs::write(&file, b"not really a font, just bytes to read").unwrap();
        (dir, file)
    }

    #[test]
    fn a_real_file_can_be_added_and_is_then_present() {
        let (dir, file) = a_real_font("add");
        let mut fonts = OutlinedFonts::default();
        fonts.add(file).expect("a real file should add cleanly");
        assert_eq!(fonts.present().len(), 1);
        assert_eq!(fonts.bytes().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_refused_rather_than_added_silently() {
        let mut fonts = OutlinedFonts::default();
        let err = fonts
            .add(PathBuf::from("/definitely/not/here/9f3a.ttf"))
            .expect_err("a missing file must not add");
        assert!(err.contains("could not read"), "unhelpful message: {err}");
        assert!(fonts.paths.is_empty());
    }

    #[test]
    fn adding_the_same_font_twice_is_refused_not_duplicated() {
        let (dir, file) = a_real_font("dup");
        let mut fonts = OutlinedFonts::default();
        fonts.add(file.clone()).unwrap();
        let err = fonts.add(file).expect_err("a second add should be refused");
        assert!(err.contains("already"), "unhelpful message: {err}");
        assert_eq!(fonts.paths.len(), 1, "must not have been added twice");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_takes_it_back_off_the_list() {
        let (dir, file) = a_real_font("remove");
        let mut fonts = OutlinedFonts::default();
        fonts.add(file.clone()).unwrap();
        fonts.remove(&file);
        assert!(fonts.paths.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_empties_the_whole_list() {
        let (dir_a, a) = a_real_font("clear-a");
        let (dir_b, b) = a_real_font("clear-b");
        let mut fonts = OutlinedFonts::default();
        fonts.add(a).unwrap();
        fonts.add(b).unwrap();
        fonts.clear();
        assert!(fonts.paths.is_empty());
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    #[test]
    fn a_font_that_has_since_gone_missing_is_not_offered_or_read() {
        // A list that offers a file that is not there is worse than a short
        // one — the same reasoning `recent.rs` uses.
        let (dir, file) = a_real_font("gone");
        let mut fonts = OutlinedFonts::default();
        fonts.add(file.clone()).unwrap();

        std::fs::remove_dir_all(&dir).unwrap();

        assert!(fonts.present().is_empty());
        assert!(fonts.bytes().is_empty());
        // But `remove`/`clear` still work on an entry that is merely stored,
        // not merely present — this is bookkeeping, not a re-read.
        fonts.remove(&file);
        assert!(fonts.paths.is_empty());
    }

    #[test]
    fn a_corrupt_settings_file_is_an_empty_list_not_a_crash() {
        let fallback: OutlinedFonts = serde_json::from_str("{ nope").unwrap_or_default();
        assert!(fallback.paths.is_empty());
    }

    #[test]
    fn a_file_with_no_paths_field_at_all_still_loads() {
        // The shape a hand-edited or pre-this-feature settings directory
        // would have.
        let loaded: OutlinedFonts = serde_json::from_str("{}").expect("defaults should fill in");
        assert!(loaded.paths.is_empty());
    }

    /// **The list is a state file like the others.** It sits where they do,
    /// and is readable by its owner alone from the moment it exists. Found by
    /// audit: a bare `fs::write` into a path function of its own left it 0644
    /// and outside `clearhistory`.
    #[cfg(unix)]
    #[test]
    fn what_is_saved_is_private_and_sits_with_the_other_state_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("pagify-outlined-fonts-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("outlined_fonts.json");

        let mut fonts = OutlinedFonts::default();
        fonts.paths.push(PathBuf::from("/tmp/Some Font.ttf"));
        fonts.save_to(&path);

        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "outlined_fonts.json was written at {mode:o}");
        let text = std::fs::read_to_string(&path).expect("read");
        let back: OutlinedFonts = serde_json::from_str(&text).expect("parse");
        assert_eq!(back.paths, fonts.paths);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_list_is_kept_beside_the_other_state_files() {
        if let (Some(here), Some(dir)) = (OutlinedFonts::path(), crate::state::state_dir()) {
            assert_eq!(here, dir.join("outlined_fonts.json"));
        }
    }
}
