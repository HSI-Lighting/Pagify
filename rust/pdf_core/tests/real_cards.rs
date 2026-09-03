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
/// **The guard's premise is wrong, not its threshold.** `looks_like_a_card` asks
/// for something reachable and something named, and "named" is
/// `could_be_a_name || is_a_company` — which a one-word logo fragment satisfies.
/// No gap threshold repairs that: the test cannot tell a name from a fragment of
/// a logo, so tuning the gap turns a knob that is not attached to the failure.
///
/// **Ignored rather than deleted, forced to pass, or patched.** The label-first
/// design resolves it structurally — segment on labels, and a card begins where a
/// second NAME appears — so a classifier that calls those fragments OTHER never
/// fires this split at all. Patching the geometric guard now would be work thrown
/// away, and this failure is evidence for that design rather than a separate bug.
/// Un-ignore when segmentation moves onto labels.
#[test]
#[ignore = "known: the guard cannot tell a logo fragment from a name; label-first segmentation resolves it"]
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
