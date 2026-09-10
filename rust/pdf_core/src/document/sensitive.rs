//! Finding the things on a page somebody would not want to send out.
//!
//! # Report, never act
//!
//! Nothing here removes anything. It says *"there is what looks like a card
//! number on page 12"* and stops, because the alternative — a tool that
//! silently blacks out whatever it guesses is sensitive — fails in the two
//! worst directions at once. It hides a price somebody meant to send, and it
//! reassures them about a name it never recognised.
//!
//! So the promise is deliberately small: **these are candidates, and a person
//! decides.**
//!
//! # Why there are no names in here
//!
//! Names, addresses and dates are what people mean by "sensitive", and they are
//! exactly what cannot be found this way. A name is a name because of what it
//! refers to, not because of how it is spelt, and no pattern separates "Mr
//! Wells" from "Wells Street". Offering it anyway would be the reassurance
//! above, in its purest form.
//!
//! What is here instead is the set that can be **checked** rather than guessed:
//! a card number that fails Luhn is not a card number, an IBAN that fails its
//! mod-97 is not an IBAN. Where a check exists, it is applied, and the false
//! positives go away.

use std::ops::Range;

/// What kind of thing was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Email,
    /// A run of digits that passes the Luhn check, which is what a payment card
    /// number is.
    PaymentCard,
    /// An account number that passes its own mod-97 check.
    Iban,
    /// A telephone number, which has no check digit and so is the weakest of
    /// these.
    Telephone,
}

impl Kind {
    pub fn describe(&self) -> &'static str {
        match self {
            Kind::Email => "email address",
            Kind::PaymentCard => "payment card number",
            Kind::Iban => "bank account number",
            Kind::Telephone => "telephone number",
        }
    }
}

/// Something found in a page's text, and where in that text it sits.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub kind: Kind,
    /// Character positions, not bytes — the boxes a page reports are per
    /// character, and an offset in the wrong units draws the mark in the wrong
    /// place.
    pub at: Range<usize>,
    pub text: String,
}

/// Everything worth a second look in some text.
///
/// Overlapping finds are resolved in favour of the one with a check behind it:
/// a card number that also looks like a telephone number is a card number.
pub fn scan(text: &str) -> Vec<Found> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();

    found.extend(emails(&chars));
    found.extend(numbers(&chars));

    // Strongest first, then drop anything that overlaps something already kept.
    found.sort_by_key(|f| (rank(f.kind), f.at.start));
    let mut kept: Vec<Found> = Vec::new();
    for candidate in found {
        let clashes = kept
            .iter()
            .any(|k| candidate.at.start < k.at.end && candidate.at.end > k.at.start);
        if !clashes {
            kept.push(candidate);
        }
    }
    kept.sort_by_key(|f| f.at.start);
    kept
}

/// How much a find is worth believing. Lower is stronger.
fn rank(kind: Kind) -> u8 {
    match kind {
        Kind::PaymentCard | Kind::Iban => 0,
        Kind::Email => 1,
        Kind::Telephone => 2,
    }
}

/// Addresses, found by their `@` and then grown outwards.
///
/// Grown rather than matched, because the interesting part is in the middle:
/// finding the `@` first and then asking how far the address reaches either
/// side needs no pattern language at all.
fn emails(chars: &[char]) -> Vec<Found> {
    let mut out = Vec::new();
    for (at, c) in chars.iter().enumerate() {
        if *c != '@' {
            continue;
        }
        let local = chars[..at]
            .iter()
            .rposition(|c| !is_local(*c))
            .map_or(0, |n| n + 1);
        let after = at + 1;
        let domain = chars[after..]
            .iter()
            .position(|c| !is_domain(*c))
            .map_or(chars.len(), |n| after + n);

        // A domain has to have a dot in it with something after, and a local
        // part cannot be empty. That is the whole of the rule, and it is enough
        // to keep `@` in prose from counting.
        let spelt: String = chars[local..domain].iter().collect();
        let Some((before, host)) = spelt.split_once('@') else { continue };
        if before.is_empty() || !host.contains('.') || host.ends_with('.') {
            continue;
        }
        if host.rsplit('.').next().is_none_or(|tld| tld.len() < 2) {
            continue;
        }
        out.push(Found { kind: Kind::Email, at: local..domain, text: spelt });
    }
    out
}

fn is_local(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-' | '\'')
}

fn is_domain(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '-')
}

/// Runs of digits and separators, then asked what they are.
fn numbers(chars: &[char]) -> Vec<Found> {
    let mut out = Vec::new();
    let mut at = 0usize;

    while at < chars.len() {
        if !chars[at].is_ascii_digit() && !is_lead(chars, at) {
            at += 1;
            continue;
        }
        // A run may hold digits and the separators people write between them.
        let mut end = at;
        while end < chars.len()
            && (chars[end].is_ascii_digit()
                || matches!(chars[end], ' ' | '-' | '.' | '(' | ')' | '+')
                || (end == at && chars[end] == '+'))
        {
            end += 1;
        }
        // Trailing separators belong to the sentence, not to the number.
        while end > at && !chars[end - 1].is_ascii_digit() {
            end -= 1;
        }
        if end <= at {
            at += 1;
            continue;
        }

        let spelt: String = chars[at..end].iter().collect();
        let digits: String = spelt.chars().filter(char::is_ascii_digit).collect();

        if (13..=19).contains(&digits.len()) && luhn(&digits) {
            out.push(Found { kind: Kind::PaymentCard, at: at..end, text: spelt });
        } else if (9..=15).contains(&digits.len()) && looks_like_a_telephone(&spelt) {
            out.push(Found { kind: Kind::Telephone, at: at..end, text: spelt });
        }
        at = end.max(at + 1);
    }

    out.extend(ibans(chars));
    out
}

fn is_lead(chars: &[char], at: usize) -> bool {
    chars[at] == '+' && chars.get(at + 1).is_some_and(char::is_ascii_digit)
}

/// A telephone number is the weakest of these, so the shape has to carry it.
///
/// **The grouping is what does the work.** Separators alone are not enough:
/// measured against a real catalogue, a table of contents reading
/// `1 04 - 05 2 14 - 15` came back as a telephone number, because it has ten
/// digits and plenty of spaces and dashes. What it does not have is a group of
/// four digits together, and every real telephone number does — `020 7946
/// 0958`, `+44 20 7946 0958`, or an unbroken run.
fn looks_like_a_telephone(spelt: &str) -> bool {
    let mut longest = 0usize;
    let mut run = 0usize;
    for c in spelt.chars() {
        if c.is_ascii_digit() {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest >= 4
}

/// The check every payment card number satisfies.
fn luhn(digits: &str) -> bool {
    let mut sum = 0u32;
    for (index, c) in digits.chars().rev().enumerate() {
        let mut value = c.to_digit(10).unwrap_or(0);
        if index % 2 == 1 {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
    }
    sum % 10 == 0
}

/// Two letters, two digits, then up to thirty alphanumerics — checked by the
/// mod-97 the standard defines, so what comes back is an account number rather
/// than a word that happened to start with two capitals.
fn ibans(chars: &[char]) -> Vec<Found> {
    let mut out = Vec::new();
    for at in 0..chars.len() {
        if at > 0 && (chars[at - 1].is_alphanumeric()) {
            continue;
        }
        if !chars[at].is_ascii_uppercase()
            || !chars.get(at + 1).is_some_and(char::is_ascii_uppercase)
            || !chars.get(at + 2).is_some_and(char::is_ascii_digit)
            || !chars.get(at + 3).is_some_and(char::is_ascii_digit)
        {
            continue;
        }
        let mut end = at;
        while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == ' ') {
            end += 1;
        }
        while end > at && !chars[end - 1].is_ascii_alphanumeric() {
            end -= 1;
        }
        let spelt: String = chars[at..end].iter().collect();
        let packed: String = spelt.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        if !(15..=34).contains(&packed.len()) || !mod_97(&packed) {
            continue;
        }
        out.push(Found { kind: Kind::Iban, at: at..end, text: spelt });
    }
    out
}

/// The IBAN check: move the first four characters to the end, turn letters into
/// numbers, and the whole thing must leave a remainder of one modulo 97.
fn mod_97(packed: &str) -> bool {
    let rotated: String = packed[4..].chars().chain(packed[..4].chars()).collect();
    let mut remainder = 0u32;
    for c in rotated.chars() {
        let value = if c.is_ascii_digit() {
            c.to_digit(10).unwrap_or(0)
        } else if c.is_ascii_alphabetic() {
            u32::from(c.to_ascii_uppercase() as u8 - b'A') + 10
        } else {
            return false;
        };
        // Fed in a digit at a time so the number never has to be held whole —
        // an IBAN is far too long for any integer here.
        remainder = if value > 9 {
            (remainder * 100 + value) % 97
        } else {
            (remainder * 10 + value) % 97
        };
    }
    remainder == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<Kind> {
        scan(text).into_iter().map(|f| f.kind).collect()
    }

    #[test]
    fn an_address_is_found_and_its_edges_are_right() {
        let found = scan("write to sales@hsilighting.co.uk about it");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, Kind::Email);
        assert_eq!(found[0].text, "sales@hsilighting.co.uk");
    }

    /// An `@` in prose is not an address, and neither is one without a dot.
    #[test]
    fn an_at_sign_on_its_own_is_not_an_address() {
        assert!(kinds("priced @ £40 each").is_empty());
        assert!(kinds("someone@localhost").is_empty());
        assert!(kinds("@example.com").is_empty());
    }

    /// **The check is what makes this worth offering.** A sixteen-digit number
    /// that fails Luhn is not a card number, and saying it is would teach
    /// somebody to ignore the tool.
    #[test]
    fn a_card_number_has_to_pass_its_check() {
        // A well-known test number, which passes.
        assert_eq!(kinds("4111 1111 1111 1111"), vec![Kind::PaymentCard]);
        // The same with one digit changed, which does not.
        assert!(!kinds("4111 1111 1111 1112").contains(&Kind::PaymentCard));
    }

    #[test]
    fn an_account_number_has_to_pass_its_check() {
        assert_eq!(kinds("GB82 WEST 1234 5698 7654 32"), vec![Kind::Iban]);
        assert!(!kinds("GB82 WEST 1234 5698 7654 31").contains(&Kind::Iban));
        // Two capitals and two digits in ordinary text are not an account.
        assert!(!kinds("model AB12 in stock").contains(&Kind::Iban));
    }

    #[test]
    fn a_telephone_number_is_found_by_its_shape() {
        assert_eq!(kinds("call +44 20 7946 0958"), vec![Kind::Telephone]);
        assert_eq!(kinds("020 7946 0958"), vec![Kind::Telephone]);
    }

    /// A catalogue is full of numbers. Part numbers, wattages and years must
    /// not come back as telephone numbers, or the report is noise.
    ///
    /// The last of these is from a real document — a table of contents that the
    /// first version of this reported as a telephone number, on the strength of
    /// having ten digits and some dashes.
    #[test]
    fn ordinary_numbers_in_a_catalogue_are_left_alone() {
        for text in [
            "3000 K colour temperature",
            "IP65 rated to 240 V",
            "in 2026 the range was 12 W",
            "order 500 units",
            "1 04 - 05 2 14 - 15",
        ] {
            assert!(scan(text).is_empty(), "{text:?} was reported as {:?}", scan(text));
        }
    }

    /// **A card number is a card number**, even though it is also a long run of
    /// digits with spaces in it.
    #[test]
    fn the_stronger_reading_wins_where_two_overlap() {
        let found = scan("4111 1111 1111 1111");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, Kind::PaymentCard);
    }

    /// Positions are in characters, because that is what a page's boxes are in.
    #[test]
    fn positions_are_character_offsets_not_byte_offsets() {
        let text = "£40 — sales@hsilighting.co.uk";
        let found = scan(text);
        assert_eq!(found.len(), 1);
        let spelt: String = text
            .chars()
            .skip(found[0].at.start)
            .take(found[0].at.end - found[0].at.start)
            .collect();
        assert_eq!(spelt, "sales@hsilighting.co.uk");
    }

    #[test]
    fn several_things_in_one_page_all_come_back() {
        let found = scan("sales@hsi.co.uk, 020 7946 0958, GB82 WEST 1234 5698 7654 32");
        assert_eq!(found.len(), 3, "{found:?}");
        // In the order they appear, so a reader can follow the page.
        assert!(found[0].at.start < found[1].at.start);
        assert!(found[1].at.start < found[2].at.start);
    }
}
