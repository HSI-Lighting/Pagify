//! The signatures somebody has drawn or uploaded, kept so they are made once.
//!
//! # What this is, and what it is not
//!
//! A signature here is **ink** — the shape of a name, drawn with a mouse or a
//! trackpad, or a picture of one uploaded from a file — stamped onto a page
//! either way. It is the mark a person writes on a form, and it proves
//! nothing about who wrote it, however it was made. The thing that does is a
//! certificate, and it lives in [`pdf_core::pdf::sign`] under a different verb
//! with a different name. Keeping the two apart is the whole reason this module
//! says so in its first paragraph: a signature that a reader believed was a
//! cryptographic one would be worse than no signature at all — a drawn one
//! and an uploaded one are exactly the same risk, and get exactly the same
//! warning wherever either is offered.
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

/// A drawn or uploaded signature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    /// What to call it in a list. Not a claim about who anyone is.
    pub name: String,
    /// The shape, in the unit box. Empty for an uploaded picture — see
    /// `image` — never both: `from_drawing` and `from_image` each set
    /// exactly one.
    pub strokes: Vec<Vec<Unit>>,
    /// Width over height, so placing it cannot stretch it. Measured from the
    /// strokes' own bounds for ink, from the picture's pixel dimensions for
    /// an upload — either way, what `placed`/`placed_rect` scale from.
    pub aspect: f32,
    /// Seconds since the epoch, so a list can be ordered.
    pub made: u64,
    /// The picture, when this signature is one rather than ink.
    #[serde(default)]
    pub image: Option<StoredImage>,
}

/// One uploaded signature's pixels, kept exactly as given.
///
/// **Alpha is real, but only reaches the page as transparency once applied.**
/// A picture from [`crate::signature_extract`] carries a genuine alpha
/// channel — background pixels transparent, ink opaque or fading toward it —
/// and the mechanism `pdf_core`'s `Annotation::Image` is merely *placed*
/// through cannot carry it, even before anything is saved. Two things
/// happen with it, for two different moments: [`composite_onto`] flattens
/// it onto a background colour for the placed annotation itself (what shows
/// while it sits there, unapplied — `Session::place_image_signature` does
/// this once the page underneath is known, not here, and not at upload,
/// which only keeps what it was handed); separately, the original is kept
/// aside for `Session::apply_signatures`, which burns a real soft mask into
/// the page from it — see `pdf_core`'s `DocumentMut::remember_image_alpha`.
/// A picture with no meaningful alpha (every byte 255, as any upload made
/// before this existed still is) flattens to itself unchanged either way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredImage {
    /// RGBA, row-major, top row first — one row of `width * 4` bytes,
    /// `height` of them.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// One pixel, blended toward `background` by its own alpha — alpha 255
/// gives back the pixel unchanged, alpha 0 gives back `background`, and
/// everything between blends the two. Always fully opaque out: this is what
/// every compositor in this module bottoms out to.
fn blend_pixel(pixel: &[u8], background: [u8; 3]) -> [u8; 4] {
    let a = pixel[3] as f32 / 255.0;
    let blend = |channel: u8, bg: u8| (channel as f32 * a + bg as f32 * (1.0 - a)).round() as u8;
    [blend(pixel[0], background[0]), blend(pixel[1], background[1]), blend(pixel[2], background[2]), 255]
}

/// Flatten `rgba` (row-major RGBA) onto a solid `background` colour,
/// producing an opaque buffer of the same dimensions.
///
/// This is where a signature's alpha — real, from
/// [`crate::signature_extract`], or uniformly opaque, from anything made
/// before that existed — actually gets used: the page a signature is placed
/// on is not known until placement, so this cannot run any earlier than
/// that. Prefer [`composite_onto_image`] where an actual picture of the
/// destination is available — a single flat colour is a coarse stand-in for
/// it, only as good as the destination is close to one colour itself.
pub fn composite_onto(rgba: &[u8], background: [u8; 3]) -> Vec<u8> {
    rgba.chunks_exact(4).flat_map(|pixel| blend_pixel(pixel, background)).collect()
}

/// Flatten `rgba` (row-major, `width`×`height`) onto `background` (row-major
/// RGB or RGBA, `bg_width`×`bg_height`) — sampled once per **source** pixel,
/// nearest-neighbour, rather than averaged into one flat colour first.
///
/// **Why this, and not [`composite_onto`].** A signature placed over
/// anything that is not one uniform colour — a photo, a patterned card, a
/// gradient — has no single colour that matches it; a flat average lands
/// somewhere between everything nearby and next to nothing, which reads as
/// a visible box rather than a matched background. Copying each
/// destination pixel's own colour in behind the signature's own alpha,
/// pixel for pixel, is indistinguishable from real transparency for as long
/// as the page underneath does not change — which, for a rectangle just
/// placed there this same moment, it has not.
///
/// `background`'s pixel stride is inferred from its length against
/// `bg_width`×`bg_height`: 3 bytes per pixel or 4 are both accepted, so a
/// caller holding a plain RGB crop need not pad it to RGBA first.
pub fn composite_onto_image(
    rgba: &[u8],
    width: u32,
    height: u32,
    background: &[u8],
    bg_width: u32,
    bg_height: u32,
) -> Vec<u8> {
    let stride = if bg_width == 0 || bg_height == 0 {
        return composite_onto(rgba, [255, 255, 255]);
    } else {
        background.len() / (bg_width as usize * bg_height as usize)
    };
    if stride < 3 {
        return composite_onto(rgba, [255, 255, 255]);
    }
    let mut out = Vec::with_capacity(rgba.len());
    for y in 0..height {
        // Nearest-neighbour from source pixel space into background pixel
        // space — the two are almost never the same resolution (a photo
        // crop placed at a few dozen points across), and nothing here needs
        // to be smoother than that: alpha's own edge already anti-aliases.
        let by = ((y as u64 * bg_height as u64) / height.max(1) as u64).min(bg_height as u64 - 1) as u32;
        for x in 0..width {
            let bx = ((x as u64 * bg_width as u64) / width.max(1) as u64).min(bg_width as u64 - 1) as u32;
            let bi = (by as usize * bg_width as usize + bx as usize) * stride;
            let bg = [background[bi], background[bi + 1], background[bi + 2]];
            let i = ((y * width + x) * 4) as usize;
            out.extend_from_slice(&blend_pixel(&rgba[i..i + 4], bg));
        }
    }
    out
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

        Some(Signature {
            name: name.into(),
            strokes,
            aspect: width / height,
            made: crate::recent::now(),
            image: None,
        })
    }

    /// Take an uploaded picture and keep it — the image counterpart of
    /// [`Signature::from_drawing`]. `None` for one with no size, or whose
    /// pixels do not match its claimed width and height — the same refusal
    /// [`pdf_core::document::Annotation::Image`] makes, checked here too so
    /// a bad upload is refused where it was chosen rather than where it is
    /// placed.
    pub fn from_image(name: impl Into<String>, rgba: Vec<u8>, width: u32, height: u32) -> Option<Signature> {
        if width == 0 || height == 0 {
            return None;
        }
        let expected = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        if rgba.len() != expected {
            return None;
        }
        Some(Signature {
            name: name.into(),
            strokes: Vec::new(),
            aspect: width as f32 / height as f32,
            made: crate::recent::now(),
            image: Some(StoredImage { rgba, width, height }),
        })
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

    /// Where a picture signature's rect goes when placed — the same anchor
    /// as [`Signature::placed`], so a picture sits exactly where ink would
    /// for the same click: left of the baseline, sized by `width`, height
    /// following the stored aspect ratio.
    pub fn placed_rect(&self, left: f32, baseline: f32, width: f32) -> pdf_core::document::Rect {
        let height = width / self.aspect.max(f32::EPSILON);
        pdf_core::document::Rect {
            left,
            top: baseline - height,
            right: left + width,
            bottom: baseline,
        }
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
        crate::state::state_dir().map(|dir| dir.join("signatures.json"))
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
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        crate::state::write_own(path, text.as_bytes())
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

    /// **A fully opaque picture flattens to itself** — every upload made
    /// before real alpha existed is exactly this shape, and must not change
    /// when it is placed.
    #[test]
    fn a_fully_opaque_picture_is_unchanged_by_compositing() {
        let rgba = vec![10, 20, 30, 255, 200, 150, 100, 255];
        assert_eq!(composite_onto(&rgba, [9, 9, 9]), rgba);
    }

    /// **A fully transparent pixel becomes exactly the background** —
    /// nothing of its own colour should show through at alpha 0.
    #[test]
    fn a_fully_transparent_pixel_becomes_the_background() {
        let rgba = vec![0, 0, 0, 0];
        assert_eq!(composite_onto(&rgba, [200, 100, 50]), vec![200, 100, 50, 255]);
    }

    /// **Half-alpha lands halfway between the pixel's own colour and the
    /// background** — a smoke test for the blend arithmetic itself, not
    /// just its two endpoints.
    #[test]
    fn partial_alpha_blends_between_pixel_and_background() {
        let rgba = vec![100, 100, 100, 128];
        let out = composite_onto(&rgba, [200, 200, 200]);
        assert!((145..=155).contains(&(out[0] as i32)), "expected roughly halfway, got {out:?}");
        assert_eq!(out[3], 255, "a composited picture is always opaque");
    }

    /// **A transparent pixel picks up its *own* corresponding background
    /// pixel, not one averaged across the whole destination** — the entire
    /// reason this function exists instead of always using
    /// [`composite_onto`]: two transparent pixels over two differently
    /// coloured parts of the background must come back two different
    /// colours.
    #[test]
    fn each_transparent_pixel_takes_the_background_pixel_behind_it() {
        // A 2x1 picture, both pixels fully transparent.
        let rgba = vec![0, 0, 0, 0, 0, 0, 0, 0];
        // A 2x1 background: left red, right blue.
        let background = vec![255, 0, 0, 255, 0, 0, 255, 255];
        let out = composite_onto_image(&rgba, 2, 1, &background, 2, 1);
        assert_eq!(&out[0..4], &[255, 0, 0, 255], "the left pixel should be red, not blended");
        assert_eq!(&out[4..8], &[0, 0, 255, 255], "the right pixel should be blue, not blended");
    }

    /// A background at a different resolution than the picture is sampled
    /// nearest-neighbour, not stretched incorrectly or panicking on the
    /// size mismatch — the ordinary case, since a photographed signature
    /// crop is almost never the same pixel size as the page region it is
    /// placed into.
    #[test]
    fn a_differently_sized_background_is_sampled_not_mismatched() {
        let rgba = vec![0u8; 10 * 10 * 4]; // 10x10, fully transparent
        let background = vec![80, 90, 100, 255].repeat(4); // 2x2, one flat colour
        let out = composite_onto_image(&rgba, 10, 10, &background, 2, 2);
        assert_eq!(out.len(), 10 * 10 * 4);
        assert!(out.chunks_exact(4).all(|p| p == [80, 90, 100, 255]));
    }

    /// An opaque pixel ignores the background entirely, regardless of which
    /// background pixel it would have mapped to.
    #[test]
    fn an_opaque_pixel_in_composite_onto_image_ignores_the_background() {
        let rgba = vec![9, 8, 7, 255];
        let background = vec![1, 2, 3, 255];
        assert_eq!(composite_onto_image(&rgba, 1, 1, &background, 1, 1), vec![9, 8, 7, 255]);
    }

    /// An empty or dimensionless background falls back to white rather than
    /// dividing by zero or panicking — the same safe default the caller had
    /// before backgrounds were sampled at all.
    #[test]
    fn an_empty_background_falls_back_to_white() {
        let rgba = vec![10, 20, 30, 0];
        assert_eq!(composite_onto_image(&rgba, 1, 1, &[], 0, 0), vec![255, 255, 255, 255]);
    }
}
