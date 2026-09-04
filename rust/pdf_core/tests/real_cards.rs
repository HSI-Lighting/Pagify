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

fn card_two_column_logo() -> Vec<TextSegment> {
    let raw = include_str!("fixtures/card_two_column_logo.json");
    let fixture: Fixture = serde_json::from_str(raw).expect("the fixture could not be read");
    fixture.segments
}

/// **Two columns must not weld into one line.**
///
/// This is the card that produced `Marketing Manager Abdul Ajees HSI LIGHTING`
/// as a single field. The name sits in the left column and the logo in the right,
/// and they share a row — so they overlap vertically, which is the only thing
/// `shares_a_line` was asking about.
///
/// Measured, not supposed: the name spans y 2382–2462 and `LIGHTING` spans
/// 2394–2462, an overlap of 68 against a shorter height of 68. The logo's `HSI`
/// then chains in, and the title chains onto that.
///
/// Not skew. The lines on this card sit at 1.0, −2.8, −0.9 and 0.8 degrees.
#[test]
fn two_columns_are_not_welded_into_one_line() {
    let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        card_two_column_logo(),
    ));

    let lines: Vec<&str> = parsed.raw_text.lines().collect();
    assert!(
        lines.iter().any(|line| *line == "Firstname Lastname"),
        "the name was fused with something else. Lines were:\n{}",
        parsed.raw_text,
    );
    assert!(
        lines.iter().any(|line| *line == "Marketing Manager"),
        "the title was fused with something else. Lines were:\n{}",
        parsed.raw_text,
    );
}

/// The same card, and what separating the columns recovers.
#[test]
fn a_two_column_card_reads_its_own_columns() {
    let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        card_two_column_logo(),
    ));

    assert_eq!(
        parsed.title.as_ref().map(|f| f.value.as_str()),
        Some("Marketing Manager"),
        "the title should be its own line now that the columns are apart",
    );
    // The address is in the left column and the GPS line in the right; they share
    // a row too, and welding them buried a Plus Code inside the address.
    assert!(
        parsed.address.as_ref().is_none_or(|f| !f.value.contains("GPS")),
        "the right column's GPS line was welded into the address: {:?}",
        parsed.address,
    );
}

/// **The logo is bigger than the name, and geometry cannot tell.**
///
/// Separating the columns does not finish this card — it changes which failure
/// shows. The lines are now right and the *labelling* is wrong: the name rule
/// takes the largest text near the top, and the logo lockup measures 117px
/// against the name's 80px, so `HSI LIGHTING` is read as the person.
///
/// This is the canonical case the whole classifier plan is built on, arrived at
/// from a real card rather than argued: **the largest text on a card is often the
/// company.** The rule that already exists to prevent it — a line carrying a
/// legal suffix is not a person — does not fire, because "Lighting" is an
/// industry word and not a suffix.
///
/// **Deliberately not patched.** Adding trade words to `COMPANY_SUFFIXES` is the
/// brittle keyword list the plan warns against: it would have to know every
/// industry in every language, and it would still miss a company called after its
/// founder. Lexical evidence is the axis that separates these, and that is what
/// the classifier is for.
///
/// What matters is that this is now a *labelling* failure. Before the columns
/// were separated the name, title and company arrived as one string and no
/// classifier downstream could have recovered them.
#[test]
#[ignore = "known: the logo lockup is taller than the name; needs lexical evidence, not geometry"]
fn the_person_is_read_as_the_person_not_the_logo() {
    let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        card_two_column_logo(),
    ));

    assert_eq!(
        parsed.name.as_ref().map(|f| f.value.as_str()),
        Some("Firstname Lastname"),
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

// ---------------------------------------------------------------------------
// The 2026-09-03 batch: five more cards, chosen for layout rather than variety.
//
// Twenty distinct companies were photographed and read. These five carry the
// §6.4 classes the two fixtures above do not: three columns, a caption beside
// the number it labels, the name last and alone, an eight-fold spread of line
// heights on one card, and two whole cards in one photograph.
// ---------------------------------------------------------------------------

fn fixture(raw: &str) -> Vec<TextSegment> {
    let fixture: Fixture = serde_json::from_str(raw).expect("the fixture could not be read");
    fixture.segments
}

fn card_three_column() -> Vec<TextSegment> {
    fixture(include_str!("fixtures/card_three_column.json"))
}

fn card_label_beside_number() -> Vec<TextSegment> {
    fixture(include_str!("fixtures/card_label_beside_number.json"))
}

fn card_two_cards_one_photograph() -> Vec<TextSegment> {
    fixture(include_str!("fixtures/card_two_cards_one_photograph.json"))
}

/// **A caption merges with the number it labels.** Half of the falsification.
///
/// The `FAX` caption sits to the left of its own number, 1.5625 line heights
/// away, and the two are one field. They merge, which is right.
///
/// Read together with `a_three_column_cards_columns_do_not_weld` below, which
/// must *split* at 1.545 — a tighter gap than this one that must merge. The two
/// measurements come from different real cards and cannot both be satisfied by
/// `MAX_GAP`, whatever it is set to.
#[test]
fn a_caption_merges_with_the_number_it_labels() {
    let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        card_label_beside_number(),
    ));

    assert!(
        parsed.raw_text.lines().any(|line| line.starts_with("FAX ")),
        "the caption came away from its own number. Lines were:\n{}",
        parsed.raw_text,
    );
}

/// **Three columns weld into one line, and it costs the name.**
///
/// The other half of the falsification, and the worse half. The name sits in
/// the left column and the title in the middle, 1.545 line heights apart on the
/// same row — tighter than the caption above that must merge — so `MAX_GAP`
/// pulls them together. The association role beneath the title then chains on,
/// and one field comes back reading
/// `Nicolas Wong VP and Co-Founder Vice Chairman of Lumen China`.
///
/// With the name buried inside that string, the name rule falls through to the
/// largest thing left near the top and returns `fO in` — the recogniser's
/// reading of two social-media glyphs.
///
/// **Ignored, and deliberately not patched.** There is no value to patch to:
/// any `MAX_GAP` above 1.5625 welds this, and any value at or below 1.545
/// breaks the caption. That is not a band left untested, it is a rule that has
/// been falsified, and the answer is the gutter detection in §2.1.4 rather than
/// a different number. Un-ignore when that lands.
#[test]
#[ignore = "known: MAX_GAP is falsified — see a_caption_merges_with_the_number_it_labels"]
fn a_three_column_cards_columns_do_not_weld() {
    let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        card_three_column(),
    ));

    let lines: Vec<&str> = parsed.raw_text.lines().collect();
    assert!(
        lines.iter().any(|line| *line == "Nicolas Wong"),
        "the name welded to the column beside it. Lines were:\n{}",
        parsed.raw_text,
    );
}

/// **Two whole cards in one photograph should be two contacts.** They are three.
///
/// The cards are stacked with 176 pixels of clear space between them, against
/// line heights of 50 to 78 — the widest horizontal corridor in the frame, and
/// the one a correct split cuts on. `split_cards` finds it and then keeps
/// going, cutting one of the halves again.
///
/// **The same cause as `a_single_card_is_not_split_in_two`**, on a card where
/// the right answer is not one but two, which is why it is worth having both.
/// `looks_like_a_card` asks for something reachable and something named, and a
/// fragment of a logo lockup satisfies `named`. Resolved by Phase B's
/// label-first segmentation, not by a threshold.
#[test]
#[ignore = "known: looks_like_a_card cannot tell a logo fragment from a name; Phase B resolves it"]
fn two_cards_in_one_photograph_are_two_contacts() {
    let cards = split_cards(card_two_cards_one_photograph());

    assert_eq!(
        cards.len(),
        2,
        "a photograph of two cards came back as {}: {:?}",
        cards.len(),
        cards
            .iter()
            .map(|card| card.segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
    );
}

/// **A certification mark is dialled as a telephone number.**
///
/// `ISO9001:2015` comes back in `phones` as `90012015`. It is printed on a
/// great many manufacturers' cards, so this is not a curiosity — it puts a
/// number that reaches nobody into the contact, and the review screen shows it
/// as a phone for someone to accept without thinking.
///
/// Not a threshold and not a Phase B question: a run of digits broken by a
/// colon, with no separators and no dialling prefix, is not a telephone number.
/// Ignored only because fixing it is a change to the phone rules that has not
/// been made yet, and it should be made deliberately with its own tests.
#[test]
fn a_certification_mark_is_not_a_telephone_number() {
    let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        card_label_beside_number(),
    ));

    let dialled: Vec<&str> = parsed.phones.iter().map(|p| p.raw.as_str()).collect();
    assert!(
        !dialled.iter().any(|raw| raw.contains("90012015")),
        "a certification mark was read as a telephone number: {dialled:?}",
    );
}

/// **An email with a space before the `@` is not read as an email at all.**
///
/// The recogniser returned `owen.hart @aureliacircuits.com`, and the card comes
/// back with no email on it. That is the field most likely to be the reason
/// somebody scanned the card, and losing it silently is worse than reading it
/// wrongly: there is nothing on the review screen to correct.
///
/// A stray space around punctuation is ordinary OCR damage and appears
/// elsewhere in this corpus. Ignored for the same reason as the mark above —
/// worth fixing deliberately rather than as a side effect of this batch.
#[test]
fn a_space_before_the_at_still_reads_as_an_email() {
    let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        card_label_beside_number(),
    ));

    assert!(
        !parsed.emails.is_empty(),
        "the only email on the card was lost. Lines were:\n{}",
        parsed.raw_text,
    );
}
