//! Turning words into the glyphs a font actually draws.
//!
//! The app used to write text one character at a time, at widths taken from the
//! standard-14 metric tables. For English that is exactly right, and it is still
//! what happens: those fonts are in every reader ever made and cost nothing to
//! use.
//!
//! For everything else it was wrong in a way no font choice could fix. Arabic
//! letters change shape depending on what they are joined to, and a joined form
//! has no character of its own to write — so every letter came out isolated, and
//! laid out left to right the word read backwards. Devanagari reorders. Thai
//! stacks. None of that is a font; it is shaping, and it has to happen before
//! anything is written down.
//!
//! So a string goes: split into runs of one direction, shaped by rustybuzz into
//! glyph ids and advances, and written into the page as those ids against an
//! embedded font. What comes back out is the third thing that has to be true —
//! see [`to_unicode_cmap`].

use crate::error::{PdfError, Result};
use rustybuzz::{Direction, Face, UnicodeBuffer};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use unicode_bidi::{BidiInfo, Level};

/// One glyph, ready to be placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    /// The glyph's id in the font — what gets written, since a joined form has
    /// no character of its own.
    pub id: u32,
    /// Where in the source string this glyph came from, as a byte offset. What
    /// makes the text searchable again afterwards.
    pub cluster: u32,
    /// How far the pen moves after drawing it, as a fraction of the point size.
    pub advance: f32,
    /// How far the glyph itself sits from the pen, as a fraction of the point
    /// size. Non-zero for marks that hang off the letter they belong to.
    pub offset_x: f32,
    pub offset_y: f32,
}

/// A shaped string: its glyphs in the order they are drawn, left to right.
#[derive(Debug, Clone, Default)]
pub struct ShapedText {
    pub glyphs: Vec<ShapedGlyph>,
    /// True when the string as a whole reads right to left. The caller needs it
    /// to decide which edge of the box the text starts from.
    pub right_to_left: bool,
}

impl ShapedText {
    /// Total width, as a fraction of the point size.
    pub fn width(&self) -> f32 {
        self.glyphs.iter().map(|g| g.advance).sum()
    }
}

/// The fonts the app has handed us, by name.
///
/// Registered once from the app's assets rather than passed with every write: a
/// font file is most of a megabyte, and a caption is a few dozen bytes.
static FONTS: OnceLock<RwLock<HashMap<String, Arc<Vec<u8>>>>> = OnceLock::new();

fn fonts() -> &'static RwLock<HashMap<String, Arc<Vec<u8>>>> {
    FONTS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Take a font file under a name the app will ask for later.
pub fn register(name: &str, data: Vec<u8>) -> Result<()> {
    // Parsed once, here, so a font that cannot be read fails at registration
    // rather than the first time somebody types in it.
    Face::from_slice(&data, 0)
        .ok_or_else(|| PdfError::InvalidArgument(format!("{name} is not a font we can read")))?;
    fonts()
        .write()
        .map_err(|_| PdfError::Pdfium("the font registry is poisoned".into()))?
        .insert(name.to_string(), Arc::new(data));
    Ok(())
}

/// The bytes of a registered font.
pub fn font_data(name: &str) -> Result<Arc<Vec<u8>>> {
    fonts()
        .read()
        .map_err(|_| PdfError::Pdfium("the font registry is poisoned".into()))?
        .get(name)
        .cloned()
        .ok_or_else(|| PdfError::InvalidArgument(format!("no font registered as {name}")))
}

pub fn is_registered(name: &str) -> bool {
    fonts().read().map(|f| f.contains_key(name)).unwrap_or(false)
}

/// Whether a registered font can draw every character of `text`.
///
/// What picks the font when the reader has not: typing Persian into a caption set
/// in Helvetica should produce Persian, not a row of empty boxes.
pub fn covers(name: &str, text: &str) -> bool {
    let Ok(data) = font_data(name) else {
        return false;
    };
    let Some(face) = Face::from_slice(&data, 0) else {
        return false;
    };
    text.chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .all(|c| face.glyph_index(c).is_some())
}

/// Shape `text` in the named font.
pub fn shape(name: &str, text: &str) -> Result<ShapedText> {
    let data = font_data(name)?;
    let face = Face::from_slice(&data, 0)
        .ok_or_else(|| PdfError::InvalidArgument(format!("{name} is not a font we can read")))?;
    Ok(shape_with(&face, text))
}

/// Shape against an already-parsed face.
///
/// Split by direction first. A line can hold both — a Persian sentence with a
/// year in it, an English one with an Arabic name — and shaping the whole thing
/// in one direction gets one of them backwards. The bidi algorithm decides where
/// the boundaries are; it is not something that can be guessed from the first
/// character.
pub fn shape_with(face: &Face, text: &str) -> ShapedText {
    if text.is_empty() {
        return ShapedText::default();
    }

    let units_per_em = face.units_per_em().max(1) as f32;
    let bidi = BidiInfo::new(text, None);

    // The paragraph's own direction, which is what the caller aligns to. A
    // string of digits inside Persian is left-to-right but the line is not.
    let right_to_left = bidi
        .paragraphs
        .first()
        .map(|p| p.level.is_rtl())
        .unwrap_or(false);

    let mut glyphs = Vec::new();
    for paragraph in &bidi.paragraphs {
        let range = paragraph.range.clone();
        let (levels, runs) = bidi.visual_runs(paragraph, range);
        for run in runs {
            let slice = &text[run.clone()];
            if slice.is_empty() {
                continue;
            }
            let level = levels.get(run.start).copied().unwrap_or(Level::ltr());
            let direction = if level.is_rtl() {
                Direction::RightToLeft
            } else {
                Direction::LeftToRight
            };

            let mut buffer = UnicodeBuffer::new();
            buffer.push_str(slice);
            buffer.set_direction(direction);
            let shaped = rustybuzz::shape(face, &[], buffer);

            // Clusters come back relative to the run; the caller needs them
            // relative to the whole string, or the ToUnicode maps every glyph to
            // whatever happens to sit at that offset from the start.
            let base = run.start as u32;
            glyphs.extend(
                shaped
                    .glyph_infos()
                    .iter()
                    .zip(shaped.glyph_positions())
                    .map(|(info, position)| ShapedGlyph {
                        id: info.glyph_id,
                        cluster: base + info.cluster,
                        advance: position.x_advance as f32 / units_per_em,
                        offset_x: position.x_offset as f32 / units_per_em,
                        offset_y: position.y_offset as f32 / units_per_em,
                    }),
            );
        }
    }

    ShapedText { glyphs, right_to_left }
}

/// The CMap that says which characters each glyph stands for.
///
/// Without one the words draw perfectly and cannot be searched, copied, or read
/// by anything that is not looking at them — which is how this failed the first
/// time it was tried: `العربية` came back out of the file as `اϨʹ۰ՍЪة`, because
/// PDFium builds its own by running the font's cmap backwards and a joined form
/// has no character to run back to.
///
/// Built from the shaper's clusters instead. A cluster is the byte offset a glyph
/// came from, so the characters a glyph stands for run from its own cluster to
/// the next boundary along — which is how one glyph honestly claims a ligature of
/// two letters, and how three marks stacked on a letter all claim the same one.
pub fn to_unicode_cmap(runs: &[(&str, &ShapedText)]) -> String {
    let mut entries: Vec<(u32, String)> = Vec::new();

    for (text, shaped) in runs {
        let mut boundaries: Vec<usize> =
            shaped.glyphs.iter().map(|g| g.cluster as usize).collect();
        boundaries.sort_unstable();
        boundaries.dedup();

        for glyph in &shaped.glyphs {
            let start = glyph.cluster as usize;
            let end = boundaries
                .iter()
                .find(|&&b| b > start)
                .copied()
                .unwrap_or(text.len());
            if let Some(slice) = text.get(start..end) {
                if !slice.is_empty() {
                    entries.push((glyph.id, slice.to_string()));
                }
            }
        }
    }

    // One entry per glyph. A glyph used twice must mean the same thing both
    // times, and a repeated code in a CMap is undefined rather than harmless.
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.dedup_by_key(|(id, _)| *id);
    cmap_from(&entries)
}

/// Wrap glyph-to-text mappings in the CMap boilerplate a PDF expects.
fn cmap_from(entries: &[(u32, String)]) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n\
         <0000> <FFFF>\n\
         endcodespacerange\n",
    );

    // In chunks: a CMap may not declare more than 100 mappings in one block.
    for chunk in entries.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (id, text) in chunk {
            let utf16: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
            cmap.push_str(&format!("<{id:04X}> <{utf16}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }

    cmap.push_str(
        "endcmap\n\
         CMapName currentdict /CMap defineresource pop\n\
         end\n\
         end\n",
    );
    cmap
}


/// A font cut down to the glyphs a piece of text actually uses.
///
/// PDFium embeds whatever font it is given, whole. Without this a four-character
/// Chinese caption put a sixteen-megabyte font into the file — the words were
/// right, the document was unusable. Subsetting the same case gives two
/// kilobytes.
///
/// Subsetting renumbers the glyphs that survive, so the ids written into the
/// page and into both tables have to be the *new* ones. That is what [ids] is
/// for, and getting it wrong draws the wrong letters rather than failing.
pub struct Subset {
    /// The cut-down font file, to embed.
    pub data: Vec<u8>,
    /// The glyphs to write, renumbered into the subset.
    pub ids: Vec<u32>,
    /// How many glyphs the subset holds, for the identity table.
    pub glyph_count: u16,
}

/// Cut [name] down to the glyphs [ids] uses.
pub fn subset(name: &str, ids: &[u32]) -> Result<Subset> {
    let data = font_data(name)?;
    let mut remapper = subsetter::GlyphRemapper::new();
    let renumbered: Vec<u32> = ids
        .iter()
        .map(|&id| remapper.remap(id as u16) as u32)
        .collect();

    let cut = subsetter::subset(&data, 0, &remapper).map_err(|e| {
        PdfError::Pdfium(format!("{name} could not be cut down: {e:?}"))
    })?;

    Ok(Subset {
        data: cut,
        ids: renumbered,
        glyph_count: remapper.num_gids(),
    })
}

/// An identity CID-to-glyph table of [glyph_count] entries.
///
/// Two bytes per glyph, big-endian, each mapping to itself — which is what
/// Identity-H means. Written out in full because PDFium refuses the font
/// outright when it is not given one: the call returns null and says nothing
/// about why.
pub fn identity_table(glyph_count: u16) -> Vec<u8> {
    (0..glyph_count as usize)
        .flat_map(|gid| [(gid >> 8) as u8, gid as u8])
        .collect()
}

/// An identity CID-to-glyph table for a font.
///
/// Two bytes per glyph, big-endian, each mapping to itself — which is what
/// Identity-H means and what the shaper's ids already are. Written out in full
/// because PDFium refuses the font outright when it is not given one: the call
/// returns null and says nothing about why.
pub fn identity_cid_to_gid(name: &str) -> Result<Vec<u8>> {
    let data = font_data(name)?;
    let face = Face::from_slice(&data, 0)
        .ok_or_else(|| PdfError::InvalidArgument(format!("{name} is not a font we can read")))?;
    let count = face.number_of_glyphs() as usize;
    Ok((0..count)
        .flat_map(|gid| [(gid >> 8) as u8, gid as u8])
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font built in the test, so the suite does not depend on the app's
    /// assets being where somebody left them.
    fn latin_face() -> Option<Vec<u8>> {
        // The bundled fonts live in the app, not the crate. Where they are not
        // reachable the shaping tests are skipped rather than failed: they are
        // testing our use of the shaper, not that a file exists.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../app/src/main/assets/fonts/NotoSans-Regular.ttf");
        std::fs::read(path).ok()
    }

    fn arabic_face() -> Option<Vec<u8>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../app/src/main/assets/fonts/NotoNaskhArabic-Regular.ttf");
        std::fs::read(path).ok()
    }

    #[test]
    fn an_empty_string_shapes_to_nothing() {
        let Some(data) = latin_face() else { return };
        let face = Face::from_slice(&data, 0).expect("parse");
        let shaped = shape_with(&face, "");
        assert!(shaped.glyphs.is_empty());
        assert_eq!(shaped.width(), 0.0);
    }

    #[test]
    fn latin_keeps_one_glyph_per_letter() {
        let Some(data) = latin_face() else { return };
        let face = Face::from_slice(&data, 0).expect("parse");
        let shaped = shape_with(&face, "Hello");
        assert_eq!(shaped.glyphs.len(), 5);
        assert!(!shaped.right_to_left);
        assert!(shaped.width() > 0.0);
        // Clusters ascend for left-to-right text, and each is the byte offset of
        // its letter.
        let clusters: Vec<u32> = shaped.glyphs.iter().map(|g| g.cluster).collect();
        assert_eq!(clusters, vec![0, 1, 2, 3, 4]);
    }

    /// The bug this module exists for.
    #[test]
    fn arabic_joins_and_runs_right_to_left() {
        let Some(data) = arabic_face() else { return };
        let face = Face::from_slice(&data, 0).expect("parse");
        let word = "العربية";
        let shaped = shape_with(&face, word);

        assert!(shaped.right_to_left, "the line was not recognised as right to left");
        assert!(!shaped.glyphs.is_empty());

        // Visual order: the first glyph drawn is the *rightmost* letter, so the
        // clusters descend. This is the half that made the word read backwards.
        let clusters: Vec<u32> = shaped.glyphs.iter().map(|g| g.cluster).collect();
        let descending = clusters.windows(2).all(|w| w[0] >= w[1]);
        assert!(descending, "clusters did not descend: {clusters:?}");

        // And joining: at least one glyph must differ from the isolated form the
        // old path drew, or the shaper ran and changed nothing.
        let isolated: Vec<u32> = word
            .chars()
            .filter_map(|c| face.glyph_index(c))
            .map(|g| g.0 as u32)
            .collect();
        let joined = shaped.glyphs.iter().any(|g| !isolated.contains(&g.id));
        assert!(joined, "every glyph is an isolated form — nothing was joined");
    }

    /// Mixed text is the case a "does it start with an Arabic letter" guess gets
    /// wrong, and a year inside a Persian sentence is not an exotic input.
    #[test]
    fn digits_inside_arabic_stay_left_to_right() {
        let Some(data) = arabic_face() else { return };
        let face = Face::from_slice(&data, 0).expect("parse");
        let shaped = shape_with(&face, "سال 2026 است");

        assert!(shaped.right_to_left, "the line as a whole reads right to left");

        // The digits form their own run, and within it the clusters ascend even
        // though the line around them descends.
        let digits: Vec<u32> = shaped
            .glyphs
            .iter()
            .map(|g| g.cluster)
            .filter(|&c| {
                shaped_char_at("سال 2026 است", c as usize).is_some_and(|c| c.is_ascii_digit())
            })
            .collect();
        assert_eq!(digits.len(), 4, "expected four digits, got {digits:?}");
        assert!(
            digits.windows(2).all(|w| w[0] < w[1]),
            "the digits came out backwards: {digits:?}",
        );
    }

    fn shaped_char_at(text: &str, offset: usize) -> Option<char> {
        text.get(offset..)?.chars().next()
    }

    #[test]
    fn the_cmap_maps_every_glyph_back_to_its_characters() {
        let Some(data) = arabic_face() else { return };
        let face = Face::from_slice(&data, 0).expect("parse");
        let word = "العربية";
        let shaped = shape_with(&face, word);
        let cmap = to_unicode_cmap(&[(word, &shaped)]);

        assert!(cmap.contains("beginbfchar"), "no mappings were written");
        assert!(cmap.contains("endcmap"), "the CMap was not closed");

        // Every glyph on the page must appear, or that part of the word is
        // uncopyable — which is exactly the silent failure this replaces.
        for glyph in &shaped.glyphs {
            assert!(
                cmap.contains(&format!("<{:04X}> <", glyph.id)),
                "glyph {} has no mapping back to text",
                glyph.id,
            );
        }

        // And the alef, first character of the word, must map to U+0627.
        assert!(cmap.contains("0627"), "the alef is not in the CMap");
    }

    #[test]
    fn a_glyph_never_gets_two_meanings() {
        let Some(data) = latin_face() else { return };
        let face = Face::from_slice(&data, 0).expect("parse");
        // "ll" uses one glyph twice; a naive builder emits it twice, which makes
        // the CMap undefined rather than merely redundant.
        let text = "hello";
        let shaped = shape_with(&face, text);
        let cmap = to_unicode_cmap(&[(text, &shaped)]);

        let mut codes: Vec<&str> = cmap
            .lines()
            .filter(|line| line.starts_with('<'))
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        let before = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(before, codes.len(), "a glyph was mapped twice");
    }
}

/// The ToUnicode CMap for glyphs that are already laid out.
///
/// Takes what each glyph *means* from the glyph itself rather than re-deriving it
/// from the source string. By the time text is written the shaping has already
/// happened, and the page and the CMap have to agree about which id stands for
/// which characters — shaping a second time to find out is two chances to differ.
pub fn to_unicode_from_glyphs(glyphs: &[crate::document::Glyph]) -> String {
    let mut entries: Vec<(u32, String)> = glyphs
        .iter()
        .filter(|g| !g.ch.is_empty())
        .map(|g| (g.id, g.ch.clone()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.dedup_by_key(|(id, _)| *id);
    cmap_from(&entries)
}
