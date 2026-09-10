//! How a document says how far it may travel.
//!
//! # A marking, not a control
//!
//! A sensitivity label is a **statement of intent**, and it is worth being
//! exact about that. It stops nobody: it does not encrypt, it does not lock,
//! and anybody who can open the document can read the label and ignore it. What
//! it does is tell a person holding the file what somebody meant them to do
//! with it — which is what most leaks lack, and what most mail systems and
//! document stores act on.
//!
//! So it is **stamped on every page**, where a reader cannot miss it and where
//! nothing else can quietly remove it. There is no second copy in the document
//! information on purpose: `hiddendata clean` strips that, and a marking that
//! disappeared when somebody sanitised a file would be worse than none.
//!
//! Whoever offers this must not let it be mistaken for [`super::redact`] or the
//! lock. Marking a page "Confidential" and sending it is not protecting it.
//!
//! # Why the stamp is removable
//!
//! It is written under one marked-content id, which is how it comes off again.
//! A label that could only be added would make an accident permanent, and a
//! document reclassified downwards — a draft becoming public — would carry the
//! old word forever.

use std::fmt;

/// How far a document may travel.
///
/// The four are the ones most schemes agree on. Deliberately not more: a scale
/// with eight steps is one nobody can place a document on, and the value of a
/// marking is that everybody reads it the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    Public,
    Internal,
    Confidential,
    Secret,
}

/// The marked-content id every stamp is written under, so it can be found and
/// taken off again.
pub const STAMP_ID: i32 = 0x5053_454E; // "PSEN"

// There is deliberately no second copy in the document's information: sanitising
// a file removes that, and a marking that vanished when somebody tidied the
// metadata would be worse than none at all. The stamp on the page is the record.

impl Sensitivity {
    /// What is stamped on the page.
    pub fn stamp(&self) -> &'static str {
        match self {
            Sensitivity::Public => "PUBLIC",
            Sensitivity::Internal => "INTERNAL",
            Sensitivity::Confidential => "CONFIDENTIAL",
            Sensitivity::Secret => "SECRET",
        }
    }

    /// What it means, in words, so a person choosing one is choosing knowingly.
    pub fn describe(&self) -> &'static str {
        match self {
            Sensitivity::Public => "may be shared with anyone",
            Sensitivity::Internal => "for people inside the organisation",
            Sensitivity::Confidential => "for named recipients only",
            Sensitivity::Secret => "for named recipients, and not to be copied",
        }
    }

    /// The colour it is stamped in — grey where it permits, red where it does
    /// not.
    pub fn colour(&self) -> super::Color {
        match self {
            Sensitivity::Public => super::Color { r: 0x6B, g: 0x72, b: 0x80, a: 255 },
            Sensitivity::Internal => super::Color { r: 0x1D, g: 0x4E, b: 0xD8, a: 255 },
            Sensitivity::Confidential => super::Color { r: 0xB4, g: 0x53, b: 0x09, a: 255 },
            Sensitivity::Secret => super::Color { r: 0xB9, g: 0x1C, b: 0x1C, a: 255 },
        }
    }

    /// Read one from what somebody typed.
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "public" => Some(Sensitivity::Public),
            "internal" => Some(Sensitivity::Internal),
            "confidential" => Some(Sensitivity::Confidential),
            "secret" => Some(Sensitivity::Secret),
            _ => None,
        }
    }

    /// Every one, for offering the choice.
    pub fn all() -> [Sensitivity; 4] {
        [
            Sensitivity::Public,
            Sensitivity::Internal,
            Sensitivity::Confidential,
            Sensitivity::Secret,
        ]
    }
}

impl fmt::Display for Sensitivity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.stamp())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_can_be_typed_and_read_back() {
        for level in Sensitivity::all() {
            assert_eq!(Sensitivity::parse(level.stamp()), Some(level));
            // However it is typed.
            assert_eq!(Sensitivity::parse(&level.stamp().to_lowercase()), Some(level));
            assert_eq!(Sensitivity::parse(&format!("  {}  ", level.stamp())), Some(level));
        }
    }

    #[test]
    fn a_word_that_is_not_a_level_is_refused() {
        for word in ["", "very secret", "top-secret", "classified"] {
            assert_eq!(Sensitivity::parse(word), None, "{word:?} was accepted");
        }
    }

    /// Each level says what it means. A marking somebody cannot interpret is a
    /// marking they will ignore.
    #[test]
    fn every_level_explains_itself() {
        for level in Sensitivity::all() {
            assert!(!level.describe().is_empty());
            assert_ne!(level.describe(), level.stamp());
        }
    }

    /// The two that restrict are marked in a colour that says so; the two that
    /// do not, are not.
    #[test]
    fn the_restrictive_levels_are_not_grey() {
        assert_ne!(Sensitivity::Secret.colour(), Sensitivity::Public.colour());
        assert_ne!(Sensitivity::Confidential.colour(), Sensitivity::Internal.colour());
    }
}
