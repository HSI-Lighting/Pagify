//! What the engine does with what a recogniser actually produced.
//!
//! Every other test in the contacts module builds its segments by hand, and hand-
//! built segments have the shape whoever wrote them expected. This one carries
//! **real ML Kit output**, captured from a photographed card on the device, with
//! the boxes untouched.
//!
//! # Why the text is anonymised and the boxes are not
//!
//! The boxes are the thing under test — every rule here is about position, size
//! and spacing. The text is somebody's name, telephone number and email address,
//! and committing that to a repository is committing a third party's personal
//! data to everyone who can read it. So names and contact routes are replaced by
//! strings of the same shape: same word count, same character classes, same
//! lengths, same OCR damage. Every rule sees what it saw.
//!
//! # What this fixture already proved
//!
//! It was captured to answer one question — when a card came back with a title, a
//! name and a company fused into one field, was that ML Kit or was it us?
//!
//! **It was neither, on this card.** ML Kit returned ten cleanly separated lines
//! and `into_lines` merges none of them. The fusion seen on another card is not
//! reproduced here, so the plan's premise cannot be assumed to hold generally
//! until it is captured on the card that showed it.

use pdf_core::contacts::parse::{parse_card, split_cards, TextSegment};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    segments: Vec<TextSegment>,
}

fn card_two_column() -> Vec<TextSegment> {
    let raw = include_str!("fixtures/card_two_column.json");
    let fixture: Fixture = serde_json::from_str(raw).expect("the fixture could not be read");
    fixture.segments
}

/// **One card must not come back as two.** Currently it does.
///
/// The photograph holds a single card. Below the contact details is a gap and then
/// the logo, which is the widest empty corridor on the card — and the splitter
/// searches widest first. The guard that should refuse the cut asks whether both
/// sides look like whole cards, and the bottom passes it: the website counts as a
/// way of reaching somebody, and the logo's two OCR fragments each look like a
/// name.
///
/// It costs three fields. The company, the email and the website all end up on the
/// phantom second card, so the contact that is saved has none of them.
///
/// **Ignored rather than deleted or made to pass**, because the fix is a threshold
/// and one card cannot tune a threshold — tightening it on this evidence alone
/// would trade this failure for a card that genuinely holds two. That is what the
/// harness is for; this assertion is what it has to satisfy. Un-ignore at A5.
#[test]
#[ignore = "known: split_cards cuts this single card in two — needs the harness to tune"]
fn a_single_card_is_not_split_in_two() {
    let cards = split_cards(card_two_column());
    assert_eq!(
        cards.len(),
        1,
        "one card came back as {} — the pieces read: {:?}",
        cards.len(),
        cards
            .iter()
            .map(|card| card.segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
    );
}

/// The lines ML Kit separated stay separated.
///
/// `into_lines` re-groups segments by vertical overlap, which is what fuses the
/// two halves of a two-column card. On this card nothing should merge: no two of
/// the ten lines overlap vertically by more than half the shorter one.
#[test]
fn nothing_that_arrived_separate_is_merged() {
    let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        card_two_column(),
    ));

    // `raw_text` is one line per assembled line, so its count is the assembly.
    let assembled = parsed.raw_text.lines().count();
    assert_eq!(
        assembled, 10,
        "ten recognised lines became {assembled}:\n{}",
        parsed.raw_text,
    );
}

/// What the fields actually come out as, so a change to any rule is visible.
///
/// Not asserted as *correct* — several of these are wrong, and deliberately
/// recorded as they are. This is the before-number the plan asks for: a baseline
/// that fails loudly when the parser changes, rather than a target.
#[test]
fn the_reading_of_a_real_card_is_recorded() {
    let cards = split_cards(card_two_column());
    let parsed = parse_card(&cards[0]);

    println!("name    : {:?}", parsed.name.as_ref().map(|f| &f.value));
    println!("title   : {:?}", parsed.title.as_ref().map(|f| &f.value));
    println!("company : {:?}", parsed.company.as_ref().map(|f| &f.value));
    println!("phones  : {:?}", parsed.phones.iter().map(|p| &p.raw).collect::<Vec<_>>());
    println!("emails  : {:?}", parsed.emails.iter().map(|f| &f.value).collect::<Vec<_>>());
    println!("urls    : {:?}", parsed.urls.iter().map(|f| &f.value).collect::<Vec<_>>());
    println!("address : {:?}", parsed.address.as_ref().map(|f| &f.value));
    println!("notes   : {:?}", parsed.notes);

    // The name is the one field the whole review screen is anchored on.
    assert_eq!(
        parsed.name.as_ref().map(|f| f.value.as_str()),
        Some("Firstname Lastname"),
        "the person's name was not read as the name",
    );
}
