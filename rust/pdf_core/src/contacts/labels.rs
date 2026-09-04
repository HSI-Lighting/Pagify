//! What a line can be called, and the shape a labelled dataset takes.
//!
//! # Four classes, not eight
//!
//! Part 9 describes eight labels — NAME, TITLE, COMPANY, PHONE, EMAIL, URL,
//! ADDRESS, OTHER. Four of those are already settled deterministically: regex
//! owns the telephone, the email and the website, and beats any model there.
//! The address has its own rules and does not compete for the field that is
//! actually failing.
//!
//! Measured over the whole card corpus, **every wrong field is a wrong name**,
//! and fifteen of sixteen of those failures are lexical. Half are one question:
//! is this line a person or an organisation? So the model this module labels
//! for is the smallest one that answers what is broken, and the eight-label
//! layout-aware version stays on the shelf until something asks for it.
//!
//! # Why this makes the card supply stop mattering
//!
//! A person-versus-organisation model does not learn from business cards. It
//! learns from names: company registers, personal-name corpora, existing
//! named-entity sets — none of which involves a photographed card. The twenty
//! cards then become the **validation** set rather than the training set, which
//! is what twenty cards are actually good for.
//!
//! That is the whole reason the 300–500 card requirement in Part 12 does not
//! bind here. It was sized for the eight-label model.

use serde::{Deserialize, Serialize};

/// What a line is, for the narrow classifier.
///
/// Deliberately not an exhaustive taxonomy of card content. Anything that is
/// not one of the three competing identities is [`LineLabel::Other`], including
/// addresses and contact routes, because those are decided elsewhere and a
/// label the model never has to act on is a label it should not spend capacity
/// learning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineLabel {
    /// A person: `Michael Peng`, `PRANAV MENON`.
    Person,
    /// An organisation, whether or not it carries a legal suffix. `VILTROX` and
    /// `HSI LIGHTING` are organisations exactly as much as `Northwind Ltd` is,
    /// and the ones without a suffix are the whole difficulty.
    Organisation,
    /// What somebody does: `Marketing Manager`, `VP and Co-Founder`.
    Role,
    /// Everything else on a card, and everything the model must not claim.
    Other,
}

impl LineLabel {
    /// Every label, in the order a model's output columns are written.
    ///
    /// Pinned, because a model's columns mean whatever this order says they
    /// mean. Reordering it silently relabels every prediction.
    pub const ALL: [LineLabel; 4] = [
        LineLabel::Person,
        LineLabel::Organisation,
        LineLabel::Role,
        LineLabel::Other,
    ];

    /// The column this label occupies in a model's output.
    pub fn column(self) -> usize {
        Self::ALL.iter().position(|l| *l == self).expect("ALL is exhaustive")
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LineLabel::Person => "person",
            LineLabel::Organisation => "organisation",
            LineLabel::Role => "role",
            LineLabel::Other => "other",
        }
    }
}

/// One labelled string, which is all a training row ever is.
///
/// No geometry, because the sources have none — see
/// [`crate::contacts::features::LEXICAL_WIDTH`]. A row from a company register
/// is a company name and the knowledge that it is one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelledLine {
    pub text: String,
    pub label: LineLabel,
    /// Where the row came from, so a bad source can be found and removed
    /// wholesale rather than row by row.
    #[serde(default)]
    pub source: String,
}

/// A dataset, as it is read from and written to disk.
///
/// **Deliberately a file the repository does not hold.** The validation half of
/// this is the real text of twenty third parties' cards — their names, their
/// employers — and Part 7 is explicit that committing text rather than
/// photographs does not sidestep privacy, because the text *is* the personal
/// data.
///
/// Anonymising it is not available either, and this is the one place where that
/// rule bites. Every other fixture in this crate replaces names with
/// same-shaped placeholders and loses nothing, because those fixtures test
/// geometry. This one tests whether the *language* of a name separates it from
/// the language of a company, and a placeholder has no language. `Firstname
/// Lastname` would validate nothing.
///
/// So the dataset lives outside the tree and the harness reads it from a path,
/// skipping when it is absent — the same pattern `RecogniserCaptureTest` uses
/// on the Android side, and for the same reason.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dataset {
    #[serde(default)]
    pub note: String,
    pub lines: Vec<LabelledLine>,
}

impl Dataset {
    /// How many rows carry each label, so an unbalanced set is visible before
    /// it becomes an unbalanced model.
    pub fn counts(&self) -> [usize; 4] {
        let mut counts = [0usize; 4];
        for line in &self.lines {
            counts[line.label.column()] += 1;
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The output columns are pinned. Reordering them relabels every prediction
    /// a trained model has ever made, without changing a single number.
    #[test]
    fn the_label_columns_are_fixed() {
        assert_eq!(LineLabel::Person.column(), 0);
        assert_eq!(LineLabel::Organisation.column(), 1);
        assert_eq!(LineLabel::Role.column(), 2);
        assert_eq!(LineLabel::Other.column(), 3);
        assert_eq!(LineLabel::ALL.len(), 4);
    }

    /// The wire form is the one the training script reads, so it is worth
    /// asserting rather than assuming serde's defaults stay put.
    #[test]
    fn a_dataset_round_trips_through_its_file_form() {
        let dataset = Dataset {
            note: "example".into(),
            lines: vec![
                LabelledLine {
                    text: "Priya Raman".into(),
                    label: LineLabel::Person,
                    source: "names".into(),
                },
                LabelledLine {
                    text: "Northwind Traders".into(),
                    label: LineLabel::Organisation,
                    source: "register".into(),
                },
            ],
        };

        let json = serde_json::to_string(&dataset).unwrap();
        assert!(json.contains("\"person\""), "labels are written as plain words: {json}");
        let back: Dataset = serde_json::from_str(&json).unwrap();
        assert_eq!(back.lines.len(), 2);
        assert_eq!(back.lines[0].label, LineLabel::Person);
        assert_eq!(back.counts(), [1, 1, 0, 0]);
    }

    /// A row need not say where it came from, so a hand-written set is not
    /// rejected for want of provenance it does not have.
    #[test]
    fn a_row_without_a_source_still_reads() {
        let one: LabelledLine =
            serde_json::from_str(r#"{"text":"VILTROX","label":"organisation"}"#).unwrap();
        assert_eq!(one.label, LineLabel::Organisation);
        assert!(one.source.is_empty());
    }
}
