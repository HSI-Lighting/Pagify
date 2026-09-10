//! The signatures somebody has drawn, kept so they are drawn once.
//!
//! # What this is, and what it is not
//!
//! A signature here is **ink** — the shape of a name, drawn with a mouse or a
//! trackpad and stamped onto a page. It is the mark a person writes on a form,
//! and it proves nothing about who wrote it. The thing that does is a
//! certificate, and it lives in [`pdf_core::pdf::sign`] under a different verb
//! with a different name. Keeping the two apart is the whole reason this module
//! says so in its first paragraph: a drawn signature that a reader believed was
//! a cryptographic one would be worse than no signature at all.
//!
//! # Why the strokes are normalised
//!
//! What is stored is the *shape*, in a unit box — 0 to 1 across and down — with
//! the drawn width-over-height beside it. Not the pixels of the canvas it
//! happened to be drawn on.
//!
//! That makes placing one a scale and a translate, and it means a signature
//! drawn on a laptop's small panel is the same signature on a large screen, in
//! a document of any size. The alternative — storing canvas coordinates —
//! bakes in the window the person happened to have open.
//!
//! # Where it lives
//!
//! In the platform's own settings directory, in the same shape as [`Recent`]
//! and [`OutlinedFonts`] — the file never leaves this machine. A signature is
//! about as personal as a file on a computer gets, and nothing here sends one
//! anywhere.
//!
//! [`Recent`]: crate::recent::Recent
//! [`OutlinedFonts`]: crate::outlined_fonts::OutlinedFonts

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A point in the unit box: 0 to 1 across and down, origin top-left.
pub type Unit = (f32, f32);

/// A drawn signature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    /// What to call it in a list. Not a claim about who anyone is.
    pub name: String,
    /// The shape, in the unit box.
    pub strokes: Vec<Vec<Unit>>,
    /// Width over height as drawn, so placing it cannot stretch it.
    pub aspect: f32,
    /// Seconds since the epoch, so a list can be ordered.
    pub made: u64,
}

/// The narrowest or flattest a drawing may be before it is padded rather than
/// divided by.
///
/// A signature that is one horizontal scrawl has no height at all, and
/// normalising it would divide by zero. Padding to a fiftieth of the other
/// dimension keeps it flat — which is what was drawn — without the arithmetic
/// falling over.
const FLATTEST: f32 = 0.02;

impl Signature {
    /// Take what was drawn on a canvas and keep the shape of it.
    ///
    /// Points are in whatever space the canvas used; only their proportions
    /// survive. `None` when there is nothing placeable: a stroke of one point
    /// is not a stroke PDFium will draw — measured, it produces an empty ink
    /// list rather than a dot — and a drawing with no extent at all is a
    /// smudge, not a signature.
    pub fn from_drawing(name: impl Into<String>, strokes: &[Vec<(f32, f32)>]) -> Option<Signature> {
        let drawn: Vec<&Vec<(f32, f32)>> = strokes.iter().filter(|s| s.len() >= 2).collect();
        if drawn.is_empty() {
            return None;
        }

        let (mut left, mut top) = (f32::MAX, f32::MAX);
        let (mut right, mut bottom) = (f32::MIN, f32::MIN);
        for point in drawn.iter().flat_map(|s| s.iter()) {
            if !point.0.is_finite() || !point.1.is_finite() {
                // One stray value would take the whole box with it.
                return None;
            }
            left = left.min(point.0);
            right = right.max(point.0);
            top = top.min(point.1);
            bottom = bottom.max(point.1);
        }

        let (width, height) = (right - left, bottom - top);
        if width <= 0.0 && height <= 0.0 {
            return None;
        }
        // Whichever dimension is missing borrows from the other, so a flat
        // scrawl stays flat instead of becoming a division by zero.
        let span = width.max(height);
        let (width, height) = (width.max(span * FLATTEST), height.max(span * FLATTEST));

        let strokes = drawn
            .iter()
            .map(|stroke| {
                stroke
                    .iter()
                    .map(|(x, y)| ((x - left) / width, (y - top) / height))
                    .collect()
            })
            .collect();

        Some(Signature { name: name.into(), strokes, aspect: width / height, made: crate::recent::now() })
    }

    /// Where the strokes go when this is placed on a page.
    ///
    /// Anchored at the **left of the baseline**, because that is where a person
    /// clicks: on the line they are signing. The signature sits on that line
    /// rather than hanging below it or straddling it.
    ///
    /// Points come back in page space — points from the top-left, y downwards —
    /// which is the space the reader and the engine already share.
    pub fn placed(&self, left: f32, baseline: f32, width: f32) -> Vec<Vec<(f32, f32)>> {
        let height = width / self.aspect.max(f32::EPSILON);
        self.strokes
            .iter()
            .map(|stroke| {
                stroke
                    .iter()
                    .map(|(u, v)| (left + u * width, baseline - height + v * height))
                    .collect()
            })
            .collect()
    }

    /// How tall it will be when placed at this width.
    pub fn height_at(&self, width: f32) -> f32 {
        width / self.aspect.max(f32::EPSILON)
    }
}

/// Why a rename did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rename {
    NoSuchSignature,
    NoName,
    /// Another signature already has that name, and taking it would destroy
    /// that drawing.
    NameTaken,
}

impl Rename {
    pub fn describe(&self) -> &'static str {
        match self {
            Rename::NoSuchSignature => "there is no signature by that name",
            Rename::NoName => "a signature needs a name",
            Rename::NameTaken => "another signature is already called that",
        }
    }
}

/// Every signature this person has drawn.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signatures {
    entries: Vec<Signature>,
}

impl Signatures {
    pub fn entries(&self) -> &[Signature] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The one a click uses: the most recently drawn **or chosen**.
    ///
    /// Rather than a "default" flag held beside the list. A flag can name a
    /// signature that has since been deleted, and then the tool has a current
    /// signature that does not exist; an order cannot. Choosing one moves it to
    /// the end, the way a recently-used list works — see [`Signatures::choose`].
    pub fn current(&self) -> Option<&Signature> {
        self.entries.last()
    }

    /// Find one by name, ignoring case.
    ///
    /// Forgiving on purpose: these names are typed at a command box, and
    /// `use signature 2` meaning something other than `use Signature 2` would
    /// be a puzzle rather than a rule.
    pub fn find(&self, name: &str) -> Option<&Signature> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Make one current, by moving it to the end.
    ///
    /// `false` when there is no such signature — the caller says which names
    /// there are rather than choosing one on somebody's behalf.
    pub fn choose(&mut self, name: &str) -> bool {
        let Some(at) = self
            .entries
            .iter()
            .position(|e| e.name.eq_ignore_ascii_case(name.trim()))
        else {
            return false;
        };
        let chosen = self.entries.remove(at);
        self.entries.push(chosen);
        true
    }

    /// Give one a different name.
    ///
    /// Refuses an empty name and refuses one already in use: [`Signatures::add`]
    /// replaces on a name collision, which is right for redrawing a signature
    /// and wrong here — renaming onto an existing name would silently destroy a
    /// drawing that cannot be got back.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), Rename> {
        let to = to.trim();
        if to.is_empty() {
            return Err(Rename::NoName);
        }
        let Some(at) = self
            .entries
            .iter()
            .position(|e| e.name.eq_ignore_ascii_case(from.trim()))
        else {
            return Err(Rename::NoSuchSignature);
        };
        if self
            .entries
            .iter()
            .enumerate()
            .any(|(i, e)| i != at && e.name.eq_ignore_ascii_case(to))
        {
            return Err(Rename::NameTaken);
        }
        self.entries[at].name = to.to_string();
        Ok(())
    }

    /// The names, in the order they are held — the current one last.
    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// Keep one. A name already used is replaced rather than duplicated.
    pub fn add(&mut self, signature: Signature) {
        self.entries.retain(|e| e.name != signature.name);
        self.entries.push(signature);
    }

    /// Forget one. **Not undoable** — the drawing is gone with it, and no
    /// document holds a copy to draw it back from.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| !e.name.eq_ignore_ascii_case(name.trim()));
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
        base.map(|b| b.join("Pagify").join("signatures.json"))
    }

    /// A missing or unreadable file is an empty list, never an error — a
    /// corrupted settings file must not stop the program starting.
    pub fn load_from(path: &std::path::Path) -> Signatures {
        let Ok(text) = std::fs::read_to_string(path) else { return Signatures::default() };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn load() -> Signatures {
        match Signatures::path() {
            Some(path) => Signatures::load_from(&path),
            None => Signatures::default(),
        }
    }

    /// Write the list, and say if it could not be written.
    ///
    /// Unlike [`Recent`], where a settings file that fails to save costs
    /// somebody a line in a menu. A signature that is reported as kept and is
    /// not kept is a different matter: the drawing is gone, and the person is
    /// told it is safe. So this reports, and the caller says so.
    ///
    /// [`Recent`]: crate::recent::Recent
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

    /// A rough two-stroke scrawl, in some canvas's pixels.
    fn scrawl() -> Vec<Vec<(f32, f32)>> {
        vec![
            vec![(100.0, 200.0), (140.0, 160.0), (180.0, 210.0), (220.0, 150.0)],
            vec![(120.0, 190.0), (240.0, 190.0)],
        ]
    }

    #[test]
    fn a_drawing_is_kept_as_its_shape_not_its_pixels() {
        let signature = Signature::from_drawing("mine", &scrawl()).expect("a signature");
        for point in signature.strokes.iter().flatten() {
            assert!((0.0..=1.0).contains(&point.0), "x out of the box: {point:?}");
            assert!((0.0..=1.0).contains(&point.1), "y out of the box: {point:?}");
        }
        // Drawn 140 across by 60 down.
        assert!((signature.aspect - 140.0 / 60.0).abs() < 0.001, "{}", signature.aspect);
    }

    /// **The same signature, drawn twice at different sizes, is the same
    /// signature.** The reason the strokes are normalised at all.
    #[test]
    fn the_size_of_the_canvas_does_not_change_what_is_stored() {
        let small = Signature::from_drawing("a", &scrawl()).expect("a");
        let large: Vec<Vec<(f32, f32)>> = scrawl()
            .iter()
            .map(|s| s.iter().map(|(x, y)| (x * 3.0 + 500.0, y * 3.0 - 40.0)).collect())
            .collect();
        let large = Signature::from_drawing("a", &large).expect("b");

        assert_eq!(small.strokes.len(), large.strokes.len());
        for (a, b) in small.strokes.iter().flatten().zip(large.strokes.iter().flatten()) {
            assert!((a.0 - b.0).abs() < 0.001 && (a.1 - b.1).abs() < 0.001, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn a_smudge_is_not_a_signature() {
        // One point is not a stroke; nothing at all is not a drawing.
        assert!(Signature::from_drawing("x", &[]).is_none());
        assert!(Signature::from_drawing("x", &[vec![(5.0, 5.0)]]).is_none());
        assert!(Signature::from_drawing("x", &[vec![(5.0, 5.0), (5.0, 5.0)]]).is_none());
    }

    /// A flat scrawl is flat, not a crash.
    #[test]
    fn a_signature_with_no_height_is_padded_rather_than_divided_by() {
        let flat = Signature::from_drawing("flat", &[vec![(0.0, 50.0), (100.0, 50.0)]])
            .expect("a flat one is still a signature");
        assert!(flat.aspect.is_finite(), "{}", flat.aspect);
        assert!(flat.aspect > 10.0, "a flat scrawl should be wide: {}", flat.aspect);
    }

    #[test]
    fn a_stray_infinity_is_refused_rather_than_taking_the_box_with_it() {
        assert!(
            Signature::from_drawing("x", &[vec![(0.0, 0.0), (f32::INFINITY, 4.0)]]).is_none()
        );
    }

    /// **Placed on the line that was clicked, at the width asked for.**
    #[test]
    fn placing_sits_the_signature_on_the_baseline_it_was_given() {
        let signature = Signature::from_drawing("mine", &scrawl()).expect("a signature");
        let placed = signature.placed(72.0, 400.0, 140.0);

        let xs: Vec<f32> = placed.iter().flatten().map(|p| p.0).collect();
        let ys: Vec<f32> = placed.iter().flatten().map(|p| p.1).collect();
        let (left, right) = (xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max));
        let (top, bottom) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));

        assert!((left - 72.0).abs() < 0.01, "left {left}");
        assert!((right - 212.0).abs() < 0.01, "right {right}");
        // Sitting *on* the line, not through it.
        assert!((bottom - 400.0).abs() < 0.01, "bottom {bottom}");
        assert!(top < 400.0, "it hangs below the line: {top}");
        assert!((signature.height_at(140.0) - (bottom - top)).abs() < 0.01);
    }

    #[test]
    fn choosing_one_makes_it_the_one_a_click_uses() {
        let mut store = Signatures::default();
        store.add(Signature::from_drawing("work", &scrawl()).expect("a"));
        store.add(Signature::from_drawing("personal", &scrawl()).expect("b"));
        assert_eq!(store.current().map(|s| s.name.as_str()), Some("personal"));

        // Case is not a puzzle to be solved.
        assert!(store.choose("WORK"));
        assert_eq!(store.current().map(|s| s.name.as_str()), Some("work"));
        assert_eq!(store.entries().len(), 2, "choosing lost one");
        assert!(!store.choose("nobody"));
    }

    /// **A choice cannot outlive what it chose.** The reason the current one is
    /// a position rather than a remembered name.
    #[test]
    fn deleting_the_chosen_one_leaves_a_real_signature_current() {
        let mut store = Signatures::default();
        store.add(Signature::from_drawing("work", &scrawl()).expect("a"));
        store.add(Signature::from_drawing("personal", &scrawl()).expect("b"));
        store.choose("work");

        assert!(store.remove("work"));
        assert_eq!(store.current().map(|s| s.name.as_str()), Some("personal"));
        assert!(store.find("work").is_none());
    }

    #[test]
    fn renaming_refuses_to_take_a_name_that_would_destroy_another_drawing() {
        let mut store = Signatures::default();
        store.add(Signature::from_drawing("work", &scrawl()).expect("a"));
        store.add(Signature::from_drawing("personal", &scrawl()).expect("b"));

        assert_eq!(store.rename("work", "personal"), Err(Rename::NameTaken));
        assert_eq!(store.rename("work", "  "), Err(Rename::NoName));
        assert_eq!(store.rename("nobody", "x"), Err(Rename::NoSuchSignature));
        assert_eq!(store.entries().len(), 2);

        assert_eq!(store.rename("work", "Work"), Ok(()), "its own name, differently cased");
        assert_eq!(store.rename("Work", "signing"), Ok(()));
        assert!(store.find("SIGNING").is_some());
        assert!(store.find("work").is_none());
    }

    #[test]
    fn the_newest_is_the_one_a_click_uses() {
        let mut store = Signatures::default();
        assert!(store.current().is_none());
        store.add(Signature::from_drawing("first", &scrawl()).expect("a"));
        store.add(Signature::from_drawing("second", &scrawl()).expect("b"));
        assert_eq!(store.current().map(|s| s.name.as_str()), Some("second"));
        assert_eq!(store.entries().len(), 2);
    }

    #[test]
    fn a_name_used_twice_replaces_rather_than_duplicates() {
        let mut store = Signatures::default();
        store.add(Signature::from_drawing("mine", &scrawl()).expect("a"));
        store.add(Signature::from_drawing("mine", &scrawl()).expect("b"));
        assert_eq!(store.entries().len(), 1);
        assert!(store.remove("mine"));
        assert!(!store.remove("mine"));
    }

    #[test]
    fn a_signature_survives_the_round_trip_to_disk() {
        let dir = std::env::temp_dir().join(format!("pagify-signatures-{}", std::process::id()));
        let path = dir.join("signatures.json");
        let mut store = Signatures::default();
        store.add(Signature::from_drawing("mine", &scrawl()).expect("a"));
        store.save_to(&path).expect("write");

        assert_eq!(Signatures::load_from(&path), store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupted_file_is_an_empty_list_rather_than_a_failure_to_start() {
        let dir = std::env::temp_dir().join(format!("pagify-signatures-bad-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("signatures.json");
        std::fs::write(&path, b"{ this is not json").expect("write");
        assert!(Signatures::load_from(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
