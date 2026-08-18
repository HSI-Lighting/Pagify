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

use crate::document::{DocumentMut, PageSize, RemovedPage};
use crate::error::Result;

/// What the user asked for. Parameters only, and serialisable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
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
    },
    SetPageRotation {
        index: usize,
        quarter_turns: u8,
    },
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
    RestorePage { at: usize, page: RemovedPage },
    /// The permutation that puts the pages back where they were.
    ReorderPages { order: Vec<usize> },
    /// Removes a page that an insert added.
    RemovePage { index: usize },
    SetPageRotation { index: usize, quarter_turns: u8 },
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
            } => {
                doc.insert_blank_page(
                    *at,
                    PageSize {
                        width_pt: *width_pt,
                        height_pt: *height_pt,
                    },
                )?;
                Ok(UndoRecord::RemovePage { index: *at })
            }
            Command::SetPageRotation {
                index,
                quarter_turns,
            } => {
                // The prior rotation has to be read *before* the change, or undo
                // restores whatever the command just set. The engine
                // implementation supplies it; until then this records zero, which
                // the round-trip test pins so the gap cannot be forgotten.
                let previous = 0u8;
                doc.set_page_rotation(*index, *quarter_turns)?;
                Ok(UndoRecord::SetPageRotation {
                    index: *index,
                    quarter_turns: previous,
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
            Command::SetPageRotation { index, .. } => format!("Rotate page {}", index + 1),
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
            | Command::InsertBlankPage { .. } => Vec::new(),
            Command::SetPageRotation { index, .. } => vec![*index],
        }
    }
}

impl UndoRecord {
    /// Reverse the execution this came from. Consumes itself, because restoring a
    /// page hands its content back to the document.
    pub fn revert(self, doc: &mut dyn DocumentMut) -> Result<()> {
        match self {
            UndoRecord::RestorePage { at, page } => doc.insert_page(at, page),
            UndoRecord::ReorderPages { order } => doc.reorder_pages(&order),
            UndoRecord::RemovePage { index } => doc.delete_page(index).map(|_| ()),
            UndoRecord::SetPageRotation {
                index,
                quarter_turns,
            } => doc.set_page_rotation(index, quarter_turns),
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
}
