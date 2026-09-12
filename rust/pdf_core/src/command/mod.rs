//! Every mutation of a document, as a value.
//!
//! One rule governs the write path: a document is only ever changed by running a
//! [`Command`]. No mutate-the-page call from the JNI bridge, ever. That is what
//! makes undo, batch processing and scripting fall out later instead of having to
//! be retrofitted into every operation.
//!
//! ## Intent and undo record are different things
//!
//! A [`Command`] is *intent* — an enum of parameters, serialisable, replayable
//! against any document. An [`UndoRecord`] is what one particular execution needs
//! in order to be reversed, and it is neither.
//!
//! They have to be separate because a command cannot always invert itself.
//! `DeletePage { index }` knows which page it removed and nothing about what was
//! on it; once PDFium has deleted it the content is gone, and no serialisable
//! value could have carried it. So `execute` hands back the record, and the
//! history keeps the pair.
//!
//! This also keeps an Action Wizard script honest: a saved script is a
//! `Vec<Command>` alone. Carrying one document's undo payloads into a replay
//! against a *different* document would be a correctness bug, not merely bloat.
//!
//! ## Why an enum rather than a trait object
//!
//! `Serialize` cannot be derived on a trait, and the usual workaround —
//! `typetag` — registers implementations through link sections. This crate ships
//! as a `cdylib` built with `lto = true`, `codegen-units = 1` and
//! `strip = "symbols"`, which is exactly where that registration is stripped or
//! never runs: a green build and an empty deserialisation at runtime. An enum
//! with `match` dispatch has no link-time magic and works on every target.
//!
//! The cost is losing open extensibility for third-party commands. The `plugins`
//! module can carry its own escape hatch if that is ever wanted.

pub mod history;

pub use history::CommandHistory;

use serde::{Deserialize, Serialize};

use crate::document::{Annotation, Color, DocumentMut, PageSize, RemovedPage};
use crate::error::{PdfError, Result};

/// What the user asked for. Parameters only, and serialisable.
///
/// `rename_all` renames the *variants*; `rename_all_fields` renames the fields
/// inside them, and both are needed. Without the second, `SetPageRotation`
/// serialises its tag as `setPageRotation` but its field as `quarter_turns` — so a
/// caller that reasonably sends camelCase throughout gets
/// `missing field 'quarter_turns'` at runtime, and nothing at all at compile time.
///
/// That cost a run on a device to find, because the mistake is invisible in half
/// the enum: `DeletePage { index }` and `ReorderPages { order }` have single-word
/// fields and worked perfectly. Only the two variants with multi-word fields were
/// broken. `decoding_the_json_the_app_sends` below pins all four against literal
/// strings, which is the only thing that can hold a wire format shared with code
/// in another language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Command {
    /// `order[i]` is the index the page currently at `i` moves to.
    ReorderPages {
        order: Vec<usize>,
    },
    DeletePage {
        index: usize,
    },
    InsertBlankPage {
        at: usize,
        width_pt: f32,
        height_pt: f32,
        /// What the sheet is made of.
        ///
        /// A PDF page has no background colour of its own — white is simply what
        /// you see through an empty page. So a coloured sheet is a filled
        /// rectangle covering it, written as page content, and it prints.
        /// Defaulted so an older caller still gets the plain white page it asked
        /// for.
        #[serde(default)]
        fill: Option<Color>,
        /// What is printed on the sheet before anything is written on it: 0 for
        /// plain, then lined, squared, dotted. Defaulted for the same reason
        /// `fill` is — an older caller asked for plain paper.
        #[serde(default)]
        ruling: i32,
    },
    SetPageRotation {
        index: usize,
        quarter_turns: u8,
    },
    /// Trim what a page shows, without discarding what is outside it.
    ///
    /// The crop box is a window onto the sheet, not a knife: the content beyond
    /// it is still in the file and a wider crop brings it back. That is why this
    /// undoes by restoring the previous rectangle rather than by putting
    /// anything back.
    SetPageCrop {
        index: usize,
        crop: crate::document::Rect,
    },
    /// Change the sheet size, scaling the content to match.
    ///
    /// Content **and** sheet. Setting the boundary alone would only reveal or
    /// hide margin, which is not what anybody means by resizing a page.
    ///
    /// The content is scaled to fit — the same factor on both axes — and
    /// centred. A different factor per axis would fit the sheet exactly and
    /// distort every letter on it.
    SetPageSize {
        index: usize,
        width_pt: f32,
        height_pt: f32,
    },
    /// Replace the words in one run of text.
    SetTextRun {
        page_index: usize,
        object: usize,
        text: String,
        /// What else to change. `Default` means the words only.
        style: crate::document::TextStyle,
    },
    /// Put pages from somewhere else into this document at `at`.
    ///
    /// Carries the pages **as their own small PDF** rather than as a reference to
    /// the document they came from, and that is the whole design of it. Redo
    /// re-executes a command against the document as it now stands; a command
    /// holding a handle to the source file could not be redone once that file was
    /// closed, which is a minute after the import in every real use. Self
    /// contained, it redoes like anything else.
    ///
    /// The cost is that the bytes sit in the undo stack until they age out of it.
    /// That is bounded by the undo depth and by what somebody chose to import,
    /// and it buys an import that behaves like every other edit.
    ImportPages {
        at: usize,
        /// A PDF holding exactly the pages to insert, in order.
        pdf: Vec<u8>,
    },

    // ------------------------------------------------------------ annotation --
    /// Put a mark on a page.
    ///
    /// Annotations reach the document through the same path as everything else,
    /// which is the payoff of routing every mutation through a command: undo,
    /// redo, cache invalidation and the JNI surface already existed, so these two
    /// variants needed none of them written again.
    AddAnnotation {
        page_index: usize,
        annotation: Annotation,
    },
    /// Take a mark off a page. `index` is **PDFium's** index for it, not a
    /// position in any list this engine produced — see [`IndexedAnnotation`].
    RemoveAnnotation {
        page_index: usize,
        index: usize,
    },
    /// Take words off a page.
    ///
    /// By the app's own id rather than by position, because text is page content
    /// and page content has no annotation index. The id is on every object the
    /// write put there, so this finds all of them however the page has been
    /// edited since.
    RemoveText {
        page_index: usize,
        id: i32,
    },
    /// Write recognised words onto a page as invisible, selectable text.
    ///
    /// Arriving as a command rather than as a special path is the whole reason
    /// "make searchable" costs so little: undo, redo, cache invalidation and
    /// both bridges need nothing written for it.
    AddTextLayer {
        page_index: usize,
        words: Vec<crate::document::RecognisedWord>,
    },

    // --------------------------------------------------------------- redact --
    /// Destroy everything inside a rectangle.
    ///
    /// The one command that removes content rather than describing a change to
    /// it, which is why it undoes by restoring a copy of the whole page. See
    /// [`crate::document::DocumentMut::snapshot_page`].
    ///
    /// **After this the document can only be saved as a full copy.** An
    /// incremental save keeps the original bytes and appends a delta, so the
    /// removed words would still be in the file. The engine refuses it.
    Redact {
        page_index: usize,
        /// Page points, top-left origin.
        area: crate::document::Rect,
        /// The mark painted over the cleared area. Absent means a black one —
        /// what a reader expects a redaction to look like. Explicit `null`
        /// leaves the space blank.
        #[serde(default = "black_mark")]
        fill: Option<Color>,
        /// Go ahead even where the rectangle cannot be fully cleared.
        ///
        /// **Phrased as the permission rather than the requirement, so that the
        /// default is the safe one.** `#[serde(default)]` on a `require_complete`
        /// flag would give `false` — and a caller on another platform that had
        /// simply never heard of the field would silently get incomplete
        /// redactions. Absent means no.
        #[serde(default)]
        allow_incomplete: bool,
        /// Faces to match type-converted-to-curves against, if the caller has
        /// any. Every candidate's glyphs are merged into one catalogue before
        /// matching — a body face and its bold both count, since the shape
        /// distance decides which candidate a given letter matches, not which
        /// one the caller happened to try first.
        ///
        /// **Empty is not a lesser request — it is the correct one** for the
        /// overwhelming majority of redactions, which run against ordinary
        /// text and need no font at all. Carried here, rather than looked up
        /// again on redo, for the same reason `ImportPages` carries whole PDF
        /// bytes: redo re-executes this exact command against the document as
        /// it now stands, and it must match with the *same* faces each time or
        /// an outlined letter that redacted once could silently stop
        /// redacting the second time.
        #[serde(default, with = "font_bytes_list")]
        outlined_fonts: Vec<Vec<u8>>,
    },

    /// Put a page back from a copy of it.
    ///
    /// **How unlocking reaches the document.** Decryption happens outside the
    /// command stack, deliberately: a command carrying a passcode would leave it
    /// sitting in the undo history, and the one thing a passcode must not do is
    /// outlive the moment it was typed. What lands here is the page itself,
    /// already verified, so the re-insertion undoes and redoes like every other
    /// edit.
    ReplacePage {
        index: usize,
        /// The page as a one-page PDF.
        #[serde(with = "page_bytes")]
        pdf: Vec<u8>,
    },
}

/// An optional font's bytes, the same way [`page_bytes`] carries a page's —
/// present as an ordinary byte array when given, entirely absent otherwise
/// rather than `null`, so an old caller's recorded command still decodes.
mod font_bytes_list {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(fonts: &[Vec<u8>], s: S) -> Result<S::Ok, S::Error> {
        fonts.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Vec<u8>>, D::Error> {
        Vec::<Vec<u8>>::deserialize(d)
    }
}

/// A page's bytes as base64 in the wire format.
mod page_bytes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        bytes.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        Vec::<u8>::deserialize(d)
    }
}

/// What a redaction mark is when the caller does not say.
fn black_mark() -> Option<Color> {
    Some(Color { r: 0, g: 0, b: 0, a: 255 })
}

/// What one execution needs in order to be undone.
///
/// Deliberately **not** `Serialize`: it can own a removed page, it is meaningful
/// only against the document it came from, and writing one into a script file
/// would invite replaying it against another.
#[derive(Debug)]
pub enum UndoRecord {
    /// Restores a deleted page. Owns the content, which is why this type cannot
    /// be cloned or serialised.
    RestorePage {
        at: usize,
        page: RemovedPage,
    },
    /// Puts back a page a redaction destroyed part of.
    ///
    /// Distinct from [`UndoRecord::RestorePage`] because the page was never
    /// removed: this one replaces, and inserting would leave two.
    RestoreRedactedPage {
        index: usize,
        page: RemovedPage,
    },
    /// The permutation that puts the pages back where they were.
    ReorderPages {
        order: Vec<usize>,
    },
    /// Removes a page that an insert added.
    RemovePage {
        index: usize,
    },
    /// Removes a run of pages that an import added.
    ///
    /// A count rather than a list of indices: the pages went in contiguously at
    /// `at`, so that is what has to come back out, and a list would let a caller
    /// ask for something that never happened.
    RemovePages {
        at: usize,
        count: usize,
    },
    SetPageRotation {
        index: usize,
        quarter_turns: u8,
    },
    SetPageCrop {
        index: usize,
        crop: crate::document::Rect,
    },
    SetTextRun {
        page_index: usize,
        object: usize,
        text: String,
        style: crate::document::TextStyle,
    },
    /// Put a resized page back exactly as it was.
    ///
    /// The **inverse matrix**, not the previous size. Re-deriving a scale from
    /// the old dimensions would centre the content on the way back, so a page
    /// whose content was not centred to begin with would not return to where it
    /// started. An inverse is exact.
    RestorePageSize {
        index: usize,
        matrix: [f32; 6],
        width_pt: f32,
        height_pt: f32,
    },

    /// Removes a mark that an add put there.
    RemoveAnnotation {
        page_index: usize,
        index: usize,
    },
    /// Puts back a mark that a remove took away.
    ///
    /// Re-adding appends, so a restored annotation lands at the end of the page's
    /// array rather than back at the index it held. That is a z-order change and
    /// nothing more, visible only where two marks overlap. Restoring the exact
    /// position would mean removing and rewriting every annotation after it —
    /// which would destroy any form widget or link this engine cannot model, a
    /// far worse trade than a highlight changing which of two overlapping marks
    /// draws on top.
    /// Take a text mark, and everything written under its id, back off a page.
    RemoveText {
        page_index: usize,
        id: i32,
    },
    RestoreAnnotation {
        page_index: usize,
        annotation: Annotation,
    },
}

impl Command {
    /// Apply, returning what is needed to reverse it.
    pub fn execute(&self, doc: &mut dyn DocumentMut) -> Result<UndoRecord> {
        match self {
            Command::ReorderPages { order } => {
                doc.reorder_pages(order)?;
                Ok(UndoRecord::ReorderPages {
                    order: invert_permutation(order),
                })
            }
            Command::DeletePage { index } => {
                let page = doc.delete_page(*index)?;
                Ok(UndoRecord::RestorePage { at: *index, page })
            }
            Command::InsertBlankPage {
                at,
                width_pt,
                height_pt,
                fill,
                ruling,
            } => {
                doc.insert_blank_page(
                    *at,
                    PageSize {
                        width_pt: *width_pt,
                        height_pt: *height_pt,
                    },
                    *fill,
                    crate::document::Ruling::from_code(*ruling),
                )?;
                Ok(UndoRecord::RemovePage { index: *at })
            }
            Command::ImportPages { at, pdf } => {
                let source =
                    crate::document::pdfium_doc::PdfiumDocument::open_bytes(pdf.clone(), None)?;
                // An empty index list is PDFium own spelling of "every page",
                // which is exactly what this command carries.
                let count = doc.import_pages(&source, &[], *at)?;
                Ok(UndoRecord::RemovePages { at: *at, count })
            }
            Command::SetPageRotation {
                index,
                quarter_turns,
            } => {
                // Read *before* the change, or undo restores whatever the command
                // just set rather than what was there.
                let previous = doc.page_rotation(*index)?;
                doc.set_page_rotation(*index, *quarter_turns)?;
                Ok(UndoRecord::SetPageRotation {
                    index: *index,
                    quarter_turns: previous,
                })
            }
            Command::SetPageCrop { index, crop } => {
                // Read before the change, or undo restores what the command
                // just set rather than what was there.
                let previous = doc.page_crop(*index)?;
                doc.set_page_crop(*index, *crop)?;
                Ok(UndoRecord::SetPageCrop { index: *index, crop: previous })
            }
            Command::SetPageSize { index, width_pt, height_pt } => {
                let before = doc.page_crop(*index)?;
                let (was_w, was_h) = (
                    (before.right - before.left).abs().max(1.0),
                    (before.bottom - before.top).abs().max(1.0),
                );

                // Fit, then centre. `min` of the two ratios rather than each
                // axis on its own: filling the sheet exactly means stretching
                // one axis, and a page of stretched type is worse than a page
                // with a margin.
                let scale = (*width_pt / was_w).min(*height_pt / was_h);
                let (tx, ty) = (
                    (*width_pt - was_w * scale) / 2.0,
                    (*height_pt - was_h * scale) / 2.0,
                );

                doc.transform_page(*index, [scale, 0.0, 0.0, scale, tx, ty])?;
                doc.set_page_media(*index, *width_pt, *height_pt)?;

                // The exact inverse: undo the translate, then the scale.
                let back = 1.0 / scale;
                Ok(UndoRecord::RestorePageSize {
                    index: *index,
                    matrix: [back, 0.0, 0.0, back, -tx * back, -ty * back],
                    width_pt: was_w,
                    height_pt: was_h,
                })
            }
            Command::SetTextRun { page_index, object, text, style } => {
                // The write reports both the words and the appearance it
                // replaced, so undo restores the whole thing — and neither can
                // have come from a different state than the change.
                let (previous, appearance) =
                    doc.set_text_run_styled(*page_index, *object, text, style)?;
                Ok(UndoRecord::SetTextRun {
                    page_index: *page_index,
                    object: *object,
                    text: previous,
                    style: appearance,
                })
            }
            Command::AddAnnotation {
                page_index,
                annotation,
            } => {
                // Text is not an annotation. It is written as page content —
                // real text objects, which is the whole reason for writing it
                // rather than drawing it — and it is found again by the id
                // tagged onto every object, not by a position in the page's
                // annotation list.
                //
                // Recording `RemoveAnnotation` for it removed whatever
                // annotation happened to sit at that index, or nothing at all,
                // and left the words on the page. Added text simply did not
                // undo.
                if let Annotation::Text { id, .. } = annotation {
                    doc.add_annotation(*page_index, annotation)?;
                    return Ok(UndoRecord::RemoveText { page_index: *page_index, id: *id });
                }

                // The index PDFium actually gave it, so undo removes this mark and
                // not whichever one happens to be last by then.
                let index = doc.add_annotation(*page_index, annotation)?;
                Ok(UndoRecord::RemoveAnnotation {
                    page_index: *page_index,
                    index,
                })
            }
            Command::RemoveAnnotation { page_index, index } => {
                let annotation = doc.take_annotation(*page_index, *index)?;
                Ok(UndoRecord::RestoreAnnotation {
                    page_index: *page_index,
                    annotation,
                })
            }
            Command::AddTextLayer { page_index, words } => {
                doc.add_text_layer(*page_index, words)?;
                // The whole layer shares one id, so undoing it is one call.
                Ok(UndoRecord::RemoveText {
                    page_index: *page_index,
                    id: crate::document::TEXT_LAYER_ID,
                })
            }
            Command::Redact { page_index, area, fill, allow_incomplete, outlined_fonts } => {
                // Copied **before** the removal, because afterwards there is
                // nothing left to copy — the same reason `delete_page` takes its
                // copy first.
                let page = doc.snapshot_page(*page_index)?;
                // Every candidate's glyphs merged into one catalogue: the
                // shape distance decides which face a given letter matches,
                // not the order candidates were given in.
                let catalogue = (!outlined_fonts.is_empty()).then(|| {
                    let mut catalogue = crate::document::glyphs::Catalogue::default();
                    for font in outlined_fonts {
                        catalogue.extend_from_font_common(font);
                    }
                    catalogue
                });
                doc.redact(
                    &crate::document::Redaction {
                        page_index: *page_index,
                        area: *area,
                        fill: *fill,
                        require_complete: !*allow_incomplete,
                        // A recorded redaction is a rectangle by construction.
                        parts: Vec::new(),
                    },
                    catalogue.as_ref(),
                )?;
                Ok(UndoRecord::RestoreRedactedPage { index: *page_index, page })
            }
            Command::ReplacePage { index, pdf } => {
                // Copied first, so undo has the page this is about to displace.
                let page = doc.snapshot_page(*index)?;
                doc.replace_page(*index, pdf)?;
                Ok(UndoRecord::RestoreRedactedPage { index: *index, page })
            }
            Command::RemoveText { page_index, id } => {
                // Read what is there before taking it out, so undo can put the
                // words back. The blob is the app's own description of the mark,
                // stored alongside it when it was written.
                let restore = doc.text_mark_restore(*page_index, *id)?;
                let annotation: Annotation = serde_json::from_str(&restore).map_err(|error| {
                    PdfError::InvalidArgument(format!("unreadable text mark {id}: {error}"))
                })?;
                doc.remove_text(*page_index, *id)?;
                Ok(UndoRecord::RestoreAnnotation {
                    page_index: *page_index,
                    annotation,
                })
            }
        }
    }

    /// Shown in the UI ("Undo delete page 5"), so it reads as a user action.
    pub fn description(&self) -> String {
        match self {
            Command::ReorderPages { .. } => "Reorder pages".into(),
            Command::DeletePage { index } => format!("Delete page {}", index + 1),
            Command::InsertBlankPage { at, .. } => format!("Insert page {}", at + 1),
            Command::ImportPages { at, .. } => format!("Import pages at {}", at + 1),
            Command::SetPageRotation { index, .. } => format!("Rotate page {}", index + 1),
            Command::SetPageCrop { index, .. } => format!("Crop page {}", index + 1),
            Command::SetTextRun { page_index, .. } => {
                format!("Edit text on page {}", page_index + 1)
            }
            Command::SetPageSize { index, .. } => format!("Resize page {}", index + 1),
            // Named by what the user drew, not by "annotation" — the label goes
            // straight onto an undo button, and "Undo add annotation" tells nobody
            // which of their marks is about to vanish.
            Command::AddAnnotation {
                page_index,
                annotation,
            } => format!("{} on page {}", annotation.describe(), page_index + 1),
            Command::RemoveAnnotation { page_index, .. } => {
                format!("Erase on page {}", page_index + 1)
            }
            Command::RemoveText { page_index, .. } => {
                format!("Erase text on page {}", page_index + 1)
            }
            Command::AddTextLayer { page_index, words } => {
                format!("Make page {} searchable ({} words)", page_index + 1, words.len())
            }
            Command::Redact { page_index, .. } => format!("Redact on page {}", page_index + 1),
            Command::ReplacePage { index, .. } => format!("Restore page {}", index + 1),
        }
    }

    /// Pages whose cached rasters this invalidates.
    ///
    /// An empty vector means *everything*. Reordering, deleting or inserting
    /// shifts every index after the change, so a cache keyed by page index has no
    /// subset it could safely keep.
    pub fn affected_pages(&self) -> Vec<usize> {
        match self {
            Command::ReorderPages { .. }
            | Command::DeletePage { .. }
            | Command::InsertBlankPage { .. }
            // An import renumbers every page after it, exactly as an insert does.
            | Command::ImportPages { .. } => Vec::new(),
            Command::SetPageRotation { index, .. } => vec![*index],
            Command::SetPageCrop { index, .. } => vec![*index],
            Command::SetTextRun { page_index, .. } => vec![*page_index],
            Command::SetPageSize { index, .. } => vec![*index],
            // A mark changes one page and renumbers nothing, so the rest of the
            // cache survives — which matters, because marks are made far more
            // often than pages are moved.
            Command::AddAnnotation { page_index, .. }
            | Command::RemoveAnnotation { page_index, .. }
            | Command::RemoveText { page_index, .. }
            // Invisible text changes no pixels, but the cache is not only for
            // pixels: a raster kept from before the layer existed would hand
            // back a page whose text and image disagree.
            | Command::AddTextLayer { page_index, .. }
            // Restoring the page replaces it, so the raster from before is
            // wrong either way round.
            | Command::Redact { page_index, .. } => vec![*page_index],
            Command::ReplacePage { index, .. } => vec![*index],
        }
    }
}

impl Annotation {
    /// How the user would name this mark, for an undo label.
    pub fn describe(&self) -> &'static str {
        match self {
            Annotation::Highlight { .. } => "Highlight",
            Annotation::Underline { .. } => "Underline",
            Annotation::StrikeOut { .. } => "Strikeout",
            Annotation::Squiggly { .. } => "Squiggly",
            Annotation::Ink { .. } => "Drawing",
            Annotation::Note { .. } => "Note",
            Annotation::Text { .. } => "Text",
        }
    }
}

impl UndoRecord {
    /// Reverse the execution this came from. Consumes itself, because restoring a
    /// page hands its content back to the document.
    pub fn revert(self, doc: &mut dyn DocumentMut) -> Result<()> {
        match self {
            UndoRecord::RestorePage { at, page } => doc.insert_page(at, page),
            UndoRecord::RestoreRedactedPage { index, page } => {
                // Replace, not insert: the redacted page is still there. Delete
                // first and the copy goes back at the same index, so nothing
                // after it moves.
                doc.delete_page(index)?;
                doc.insert_page(index, page)
            }
            UndoRecord::ReorderPages { order } => doc.reorder_pages(&order),
            UndoRecord::RemovePage { index } => doc.delete_page(index).map(|_| ()),
            // Backwards: removing a page shifts every index after it, so taking
            // the run out front-first would delete the wrong pages from the
            // second one on.
            UndoRecord::RemovePages { at, count } => {
                for index in (at..at + count).rev() {
                    doc.delete_page(index)?;
                }
                Ok(())
            }
            UndoRecord::SetPageRotation {
                index,
                quarter_turns,
            } => doc.set_page_rotation(index, quarter_turns),
            UndoRecord::SetPageCrop { index, crop } => doc.set_page_crop(index, crop),
            UndoRecord::SetTextRun { page_index, object, text, style } => {
                doc.set_text_run_styled(page_index, object, &text, &style).map(|_| ())
            }
            UndoRecord::RestorePageSize { index, matrix, width_pt, height_pt } => {
                // The sheet first, then the content: transforming into a page
                // that is still the new size would clip against the wrong
                // boundary on the way back.
                doc.set_page_media(index, width_pt, height_pt)?;
                doc.transform_page(index, matrix)
            }
            UndoRecord::RemoveAnnotation { page_index, index } => {
                doc.remove_annotation(page_index, index)
            }
            UndoRecord::RemoveText { page_index, id } => doc.remove_text(page_index, id),
            UndoRecord::RestoreAnnotation {
                page_index,
                annotation,
            } => doc.add_annotation(page_index, &annotation).map(|_| ()),
        }
    }
}

/// The permutation that undoes `order`.
///
/// `order[i] = j` means "the page at i moves to j", so the inverse sends j back
/// to i. Spelled out rather than reversed in place because getting it backwards
/// produces a reorder that looks plausible and is wrong on any permutation that
/// is not its own inverse — the identity and a simple swap both survive the
/// mistake, which is why the tests below use a rotation.
fn invert_permutation(order: &[usize]) -> Vec<usize> {
    let mut inverse = vec![0usize; order.len()];
    for (from, &to) in order.iter().enumerate() {
        if to < inverse.len() {
            inverse[to] = from;
        }
    }
    inverse
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Color, Rect};

    #[test]
    fn a_permutation_and_its_inverse_cancel() {
        let order = vec![2, 0, 3, 1];
        let inverse = invert_permutation(&order);
        let round_trip: Vec<usize> = (0..order.len()).map(|i| inverse[order[i]]).collect();
        assert_eq!(vec![0, 1, 2, 3], round_trip);
    }

    #[test]
    fn inverting_a_swap_is_the_same_swap() {
        assert_eq!(vec![1, 0, 2], invert_permutation(&[1, 0, 2]));
    }

    /// The case that catches a reversed inverse: a rotation is not its own.
    #[test]
    fn a_rotation_inverts_to_the_opposite_rotation() {
        assert_eq!(vec![2, 0, 1], invert_permutation(&[1, 2, 0]));
    }

    #[test]
    fn every_command_round_trips_through_json() {
        let commands = vec![
            Command::ReorderPages {
                order: vec![2, 0, 1],
            },
            Command::DeletePage { index: 4 },
            Command::InsertBlankPage {
                at: 1,
                width_pt: 595.0,
                height_pt: 842.0,
                            fill: None,
                            ruling: 0,
            },
            Command::SetPageRotation {
                index: 7,
                quarter_turns: 3,
            },
        ];

        for command in commands {
            let json = serde_json::to_string(&command).expect("serialise");
            let back: Command = serde_json::from_str(&json).expect("deserialise");
            assert_eq!(command, back, "round trip changed {json}");
        }
    }

    /// The tag is what lets a saved script be read by a later build that has
    /// added variants, so it is part of the format rather than an accident of it.
    #[test]
    fn the_serialised_form_is_tagged_by_operation() {
        let json = serde_json::to_string(&Command::DeletePage { index: 2 }).unwrap();
        assert!(json.contains("\"op\":\"deletePage\""), "got {json}");
    }

    #[test]
    /// **The default a caller gets by not knowing about the field.**
    ///
    /// This is why the flag is `allowIncomplete` and not `requireComplete`.
    /// Serde fills an absent field with `Default`, which for a bool is `false` —
    /// so the safe answer has to be the one `false` means. Named the other way
    /// round, a platform that had never heard of the field would silently ship
    /// incomplete redactions, and nothing would say so.
    #[test]
    fn a_caller_that_says_nothing_gets_the_strict_redaction() {
        let decoded: Command = serde_json::from_str(
            r#"{"op":"redact","pageIndex":0,"area":{"left":0.0,"top":0.0,"right":1.0,"bottom":1.0}}"#,
        )
        .expect("decode");
        let Command::Redact { allow_incomplete, fill, .. } = decoded else {
            panic!("wrong variant")
        };
        assert!(!allow_incomplete, "silence was read as permission");
        assert_eq!(
            fill,
            Some(Color { r: 0, g: 0, b: 0, a: 255 }),
            "a redaction with no mark asked for should still be marked"
        );
    }

    /// And a caller that means it can still say so, both ways.
    #[test]
    fn a_caller_can_ask_for_an_unmarked_or_partial_redaction() {
        let decoded: Command = serde_json::from_str(
            r#"{"op":"redact","pageIndex":0,"area":{"left":0.0,"top":0.0,"right":1.0,"bottom":1.0},"fill":null,"allowIncomplete":true}"#,
        )
        .expect("decode");
        let Command::Redact { allow_incomplete, fill, .. } = decoded else {
            panic!("wrong variant")
        };
        assert!(allow_incomplete);
        assert_eq!(fill, None);
    }

    /// A caller with a candidate face — or several — can send them. Plain
    /// byte arrays in the wire format, the same as `ReplacePage`'s `pdf`
    /// field: `#[serde(with = "font_bytes_list")]` passes a `Vec<Vec<u8>>`
    /// straight through rather than encoding it as text.
    #[test]
    fn a_caller_can_send_more_than_one_candidate_face() {
        let decoded: Command = serde_json::from_str(
            r#"{"op":"redact","pageIndex":0,"area":{"left":0.0,"top":0.0,"right":1.0,"bottom":1.0},"outlinedFonts":[[1,2,3],[4,5]]}"#,
        )
        .expect("decode");
        let Command::Redact { outlined_fonts, .. } = decoded else { panic!("wrong variant") };
        assert_eq!(outlined_fonts, vec![vec![1, 2, 3], vec![4, 5]]);
    }

    /// And the overwhelming common case — no font at all — is what absence
    /// decodes to, not an error.
    #[test]
    fn no_outlined_fonts_field_at_all_decodes_to_an_empty_list() {
        let decoded: Command = serde_json::from_str(
            r#"{"op":"redact","pageIndex":0,"area":{"left":0.0,"top":0.0,"right":1.0,"bottom":1.0}}"#,
        )
        .expect("decode");
        let Command::Redact { outlined_fonts, .. } = decoded else { panic!("wrong variant") };
        assert!(outlined_fonts.is_empty());
    }

    #[test]
    fn a_redaction_names_its_page_for_the_undo_button() {
        let command = Command::Redact {
            page_index: 4,
            area: Rect { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 },
            fill: None,
            allow_incomplete: false,
            outlined_fonts: Vec::new(),
        };
        assert_eq!("Redact on page 5", command.description());
        assert_eq!(vec![4], command.affected_pages());
    }

    fn descriptions_count_pages_from_one_because_readers_do() {
        assert_eq!(
            "Delete page 5",
            Command::DeletePage { index: 4 }.description()
        );
    }

    #[test]
    fn a_reorder_invalidates_every_cached_page() {
        assert!(Command::ReorderPages { order: vec![1, 0] }
            .affected_pages()
            .is_empty());
    }

    #[test]
    fn a_rotation_invalidates_only_the_page_it_turned() {
        assert_eq!(
            vec![3],
            Command::SetPageRotation {
                index: 3,
                quarter_turns: 1
            }
            .affected_pages()
        );
    }

    /// The exact strings `PdfCommand.toJson()` produces in Kotlin.
    ///
    /// Written as literals on purpose. A round-trip test — serialise a `Command`,
    /// deserialise it, compare — passes happily whatever the field names are, so it
    /// cannot see a mismatch with the other side of the boundary. These strings are
    /// copied from `app/src/main/java/com/hsilighting/pagify/core/PdfEdit.kt`, and
    /// they are the contract.
    ///
    /// This test exists because two of the four commands were undecodable in a
    /// build whose Rust and Kotlin suites were both green: `rename_all` renamed the
    /// variants and left `quarter_turns`, `width_pt` and `height_pt` in snake_case.
    /// It took running the app on a tablet to find, which is far too late.
    #[test]
    fn decoding_the_json_the_app_sends() {
        let cases = [
            (
                r#"{"op":"deletePage","index":3}"#,
                Command::DeletePage { index: 3 },
            ),
            (
                r#"{"op":"reorderPages","order":[2,0,1]}"#,
                Command::ReorderPages {
                    order: vec![2, 0, 1],
                },
            ),
            (
                r#"{"op":"insertBlankPage","at":1,"widthPt":595,"heightPt":842}"#,
                Command::InsertBlankPage {
                    at: 1,
                    width_pt: 595.0,
                    height_pt: 842.0,
                                    fill: None,
                                    ruling: 0,
                },
            ),
            (
                r#"{"op":"setPageRotation","index":0,"quarterTurns":1}"#,
                Command::SetPageRotation {
                    index: 0,
                    quarter_turns: 1,
                },
            ),
            // A mark is a *nested* object under "annotation". Merging its fields
            // into the command reads perfectly well and decodes as
            // `missing field annotation` — which is how the app first sent it, and
            // what a device run rather than either test suite had to catch.
            (
                r#"{"op":"addAnnotation","pageIndex":0,"annotation":{"kind":"highlight","rects":[{"left":1.0,"top":2.0,"right":3.0,"bottom":4.0}],"color":{"r":255,"g":224,"b":102,"a":128}}}"#,
                Command::AddAnnotation {
                    page_index: 0,
                    annotation: Annotation::Highlight {
                        rects: vec![Rect {
                            left: 1.0,
                            top: 2.0,
                            right: 3.0,
                            bottom: 4.0,
                        }],
                        color: Color {
                            r: 255,
                            g: 224,
                            b: 102,
                            a: 128,
                        },
                    },
                },
            ),
            (
                r#"{"op":"removeAnnotation","pageIndex":3,"index":7}"#,
                Command::RemoveAnnotation {
                    page_index: 3,
                    index: 7,
                },
            ),
            (
                r#"{"op":"redact","pageIndex":2,"area":{"left":10.0,"top":20.0,"right":90.0,"bottom":34.0}}"#,
                Command::Redact {
                    page_index: 2,
                    area: Rect { left: 10.0, top: 20.0, right: 90.0, bottom: 34.0 },
                    fill: Some(Color { r: 0, g: 0, b: 0, a: 255 }),
                    allow_incomplete: false,
                    outlined_fonts: Vec::new(),
                },
            ),
        ];

        for (json, expected) in cases {
            let decoded: Command = serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("the app sends {json}, which failed to decode: {e}"));
            assert_eq!(expected, decoded, "decoded {json} into the wrong command");
        }
    }

    /// Every command must also *encode* to the same shape, so a saved script can be
    /// read back by either side.
    #[test]
    fn encoding_matches_what_the_app_expects_to_read() {
        let encoded = serde_json::to_string(&Command::SetPageRotation {
            index: 2,
            quarter_turns: 3,
        })
        .expect("encode");

        assert!(
            encoded.contains(r#""quarterTurns":3"#),
            "fields must be camelCase like the tag, got {encoded}",
        );
        assert!(
            encoded.contains(r#""op":"setPageRotation""#),
            "got {encoded}"
        );
    }

    /// An unknown operation must be an error rather than a silent no-op.
    #[test]
    fn an_unrecognised_command_is_refused() {
        // Fully qualified: `Result` in this crate is an alias with one parameter.
        let result: std::result::Result<Command, serde_json::Error> =
            serde_json::from_str(r#"{"op":"encryptEverything"}"#);
        assert!(
            result.is_err(),
            "an operation this build does not implement must fail loudly, not be ignored",
        );
    }
}
