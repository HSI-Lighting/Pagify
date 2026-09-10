//! What a passcode has to be before it will lock anything.
//!
//! # Why there is a rule at all
//!
//! A locked document carries its own original inside itself. The passcode is
//! the only thing between somebody with the file and everything that was taken
//! off its pages — there is no server to rate-limit them, no account to lock
//! out, and no way to change the passcode on a copy already sent. Whatever
//! guessing anyone cares to do, they can do offline, for as long as they like.
//!
//! Argon2id makes each guess expensive, which is what carries most of the
//! weight here. Eight characters across four classes is a **modest** bar — it
//! is worth being plain about that rather than implying otherwise. What it
//! buys is the whole space of ordinary words, names and dates, which is where
//! guessing starts; what it does not buy is much against somebody willing to
//! spend real money on the problem. Length is the only lever that does, and
//! the number here is a deliberate choice about how much to ask of the person
//! typing.
//!
//! # It applies to setting, never to opening
//!
//! A document locked under an older rule — or by another program, or before
//! this existed — must still open. So this is asked at the moment a passcode is
//! **chosen**, and never at the moment one is used. Nothing here can lock
//! somebody out of their own document.

/// Something a passcode is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmet {
    TooShort { need: usize, have: usize },
    NoUppercase,
    NoLowercase,
    NoDigit,
    NoSymbol,
}

impl Unmet {
    /// What to show against the requirement, in the words a person would use.
    pub fn describe(&self) -> String {
        match self {
            Unmet::TooShort { need, have } => {
                format!("at least {need} characters — {have} so far")
            }
            Unmet::NoUppercase => "an upper-case letter".into(),
            Unmet::NoLowercase => "a lower-case letter".into(),
            Unmet::NoDigit => "a number".into(),
            Unmet::NoSymbol => "a symbol, such as ! ? - or £".into(),
        }
    }
}

/// The shortest a new passcode may be.
pub const LEAST: usize = 8;

/// Everything a passcode is missing, in the order it should be shown.
///
/// An empty list means it will do. The list is returned whole rather than the
/// first problem, so somebody can be shown all of it at once and fix it in one
/// go instead of being told one thing at a time.
pub fn unmet(passcode: &str) -> Vec<Unmet> {
    let mut missing = Vec::new();

    // Counted in characters rather than bytes: "£" is two bytes and one
    // character, and telling somebody their eight characters are nine would be
    // nonsense.
    let length = passcode.chars().count();
    if length < LEAST {
        missing.push(Unmet::TooShort { need: LEAST, have: length });
    }
    if !passcode.chars().any(char::is_uppercase) {
        missing.push(Unmet::NoUppercase);
    }
    if !passcode.chars().any(char::is_lowercase) {
        missing.push(Unmet::NoLowercase);
    }
    if !passcode.chars().any(|c| c.is_numeric()) {
        missing.push(Unmet::NoDigit);
    }
    // A symbol is anything that is not a letter, a number, or a space. Defined
    // by what it is not, so that punctuation from any language counts — a rule
    // that only accepted ASCII punctuation would refuse a perfectly good
    // passcode for being foreign.
    if !passcode.chars().any(|c| !c.is_alphanumeric() && !c.is_whitespace()) {
        missing.push(Unmet::NoSymbol);
    }
    missing
}

/// Whether a passcode may be used to lock something for the first time.
pub fn is_strong_enough(passcode: &str) -> bool {
    unmet(passcode).is_empty()
}

/// One line saying what is still wrong, or `None` when nothing is.
pub fn problem(passcode: &str) -> Option<String> {
    let missing = unmet(passcode);
    if missing.is_empty() {
        return None;
    }
    let described: Vec<String> = missing.iter().map(Unmet::describe).collect();
    Some(format!("still needs: {}", described.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passcode_with_everything_is_accepted() {
        assert!(is_strong_enough("Correct-Horse-99-Battery"));
        assert!(problem("Correct-Horse-99-Battery").is_none());
    }

    #[test]
    fn each_missing_class_is_reported_on_its_own() {
        assert!(unmet("correct-horse-99-battery").contains(&Unmet::NoUppercase));
        assert!(unmet("CORRECT-HORSE-99-BATTERY").contains(&Unmet::NoLowercase));
        assert!(unmet("Correct-Horse-Battery-Ok").contains(&Unmet::NoDigit));
        assert!(unmet("CorrectHorse99BatteryOk").contains(&Unmet::NoSymbol));
    }

    /// **Everything wrong at once, so it can be fixed in one go.** Being told
    /// one requirement at a time is how a person ends up on their fifth attempt.
    #[test]
    fn everything_missing_is_reported_together() {
        let missing = unmet("short");
        assert_eq!(missing.len(), 4, "{missing:?}");
        let said = problem("short").expect("a problem");
        for expected in ["8 characters", "upper-case", "number", "symbol"] {
            assert!(said.contains(expected), "{expected:?} was not said: {said}");
        }
    }

    #[test]
    fn the_length_is_counted_in_characters_not_bytes() {
        // Eighteen characters, twenty-two bytes.
        let passcode = "Pâté-Sundæ-99-Ωmega";
        assert_eq!(passcode.chars().count(), 19);
        assert!(passcode.len() > 19, "the fixture is not multi-byte after all");
        assert!(is_strong_enough(passcode), "{:?}", unmet(passcode));
    }

    /// A rule that only accepted ASCII punctuation would refuse a perfectly
    /// good passcode for being foreign.
    #[test]
    fn punctuation_from_any_language_counts_as_a_symbol() {
        for passcode in [
            "Correct£Horse99Battery",
            "Correct、Horse99Battery",
            "Correct—Horse99Battery",
        ] {
            assert!(
                !unmet(passcode).contains(&Unmet::NoSymbol),
                "{passcode:?} was not credited with a symbol"
            );
        }
    }

    /// A space is not a symbol. Somebody typing a sentence has met the length
    /// and nothing else, and telling them otherwise would be a lie.
    #[test]
    fn a_space_alone_does_not_count_as_a_symbol() {
        assert!(unmet("Correct horse 99 battery").contains(&Unmet::NoSymbol));
    }

    #[test]
    fn the_length_is_reported_with_how_far_along_they_are() {
        let missing = unmet("Ab1!");
        assert!(missing.contains(&Unmet::TooShort { need: LEAST, have: 4 }));
        assert!(problem("Ab1!").expect("a problem").contains("4 so far"));
    }

    /// The shortest thing that passes, so the boundary is somebody's decision
    /// rather than an accident of the code.
    #[test]
    fn eight_characters_across_the_four_classes_is_enough() {
        assert!(is_strong_enough("Ab1!efgh"), "{:?}", unmet("Ab1!efgh"));
        assert!(!is_strong_enough("Ab1!efg"), "seven characters was accepted");
    }
}
