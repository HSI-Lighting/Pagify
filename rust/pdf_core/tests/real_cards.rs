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

/// **One card must not come back as two.** It no longer does.
///
/// The photograph holds a single card. Below the contact details is a gap and then
/// the logo, which is the widest empty corridor on the card — and the splitter
/// searches widest first. The guard that should have refused the cut asked whether
/// both halves look like whole cards, and the bottom passed: the website counted
/// as a way of reaching somebody, and the logo's two OCR fragments each looked
/// like a name.
///
/// It cost three fields. The company, the email and the website all ended up on
/// the phantom second card, so the contact that was saved had none of them.
///
/// **Fixed — though not by the route this test spent two revisions predicting.**
/// The prediction was that only label-first segmentation could resolve it, since
/// no gap threshold tells a name from a logo fragment. The reasoning was right and
/// the conclusion too narrow: the guard never needed to tell them apart. It needed
/// to stop accepting the evidence a logo fragment offers.
///
/// Two policies on `looks_like_a_card` do that — a card begins with who it is
/// rather than how to reach them, and a card carries a telephone number where a
/// logo lockup carries only a website. Both are claims about what a card is rather
/// than tuned numbers, both are argued from a measured asymmetry, and both are the
/// interim mitigation the plan called for rather than the structural fix. Phase B
/// still supersedes them, and will retire the cost recorded in
/// `a_card_with_no_telephone_is_still_its_own_card`.
#[test]
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

/// **Two whole cards in one photograph are two contacts.** They were three.
///
/// The cards are stacked with 176 pixels of clear space between them, against
/// line heights of 50 to 78 — the widest horizontal corridor in the frame, and
/// the one a correct split cuts on. `split_cards` found it and then kept going,
/// cutting one of the halves again just below its own telephone line.
///
/// **Worth keeping beside `a_single_card_is_not_split_in_two`** precisely
/// because the right answer here is two rather than one. A guard made harder to
/// satisfy will always fix the card that should not be cut; the question is
/// whether it still cuts the photograph that should be. This is that question,
/// and it is the fixture that would catch a mitigation which had simply stopped
/// splitting anything.
#[test]
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

/// **The same card, photographed twice, must read the same.**
///
/// Nothing else in the corpus checks this. Every other fixture is one
/// photograph of one card, so a reading that depends on where the camera was
/// would pass all of them.
///
/// The recogniser genuinely returned this card differently the second time: the
/// logo as one run rather than two, the email line as *two* boxes rather than
/// one, and a space inserted after the `@`. All three are the kind of variation
/// that comes free with a different distance and angle, and none of them should
/// reach the fields.
///
/// The title, both addresses and the website survive it. **The name does not**,
/// and is deliberately left out of this assertion rather than quietly weakened
/// to fit: shot one reads `HSI LIGHTING` and shot two `HSHCHTING`. Both are the
/// logo and both are wrong — that is
/// `the_person_is_read_as_the_person_not_the_logo` above — but they are wrong
/// *differently*, so the value on the review screen depends on how the
/// photograph was taken. Fold this back in when the classifier lands.
#[test]
fn the_same_card_photographed_twice_reads_the_same() {
    let first = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        card_two_column_logo(),
    ));
    let second = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(fixture(
        include_str!("fixtures/card_two_column_logo_second_shot.json"),
    )));

    let addresses = |card: &pdf_core::contacts::BusinessCard| {
        card.emails.iter().map(|f| f.value.clone()).collect::<Vec<_>>()
    };
    let sites = |card: &pdf_core::contacts::BusinessCard| {
        card.urls.iter().map(|f| f.value.clone()).collect::<Vec<_>>()
    };

    assert_eq!(
        first.title.as_ref().map(|f| &f.value),
        second.title.as_ref().map(|f| &f.value),
        "the title changed with the camera angle",
    );
    assert_eq!(
        addresses(&first),
        addresses(&second),
        "the addresses changed with the camera angle",
    );
    assert_eq!(sites(&first), sites(&second), "the website changed with the camera angle");
}

/// **Both addresses, from the one line that carries them.**
///
/// This card prints its two email addresses side by side on a single line, and
/// only the first was ever read — the second was dropped in silence, on the
/// card belonging to the person building this. The second shot splits the same
/// line into two boxes, so the fix has to hold whichever way it arrives.
#[test]
fn two_addresses_on_one_line_are_both_read() {
    for (shot, raw) in [
        ("one box", include_str!("fixtures/card_two_column_logo.json")),
        ("two boxes", include_str!("fixtures/card_two_column_logo_second_shot.json")),
    ] {
        let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
            fixture(raw),
        ));
        let addresses: Vec<&str> = parsed.emails.iter().map(|f| f.value.as_str()).collect();
        assert_eq!(
            addresses,
            vec!["Lights@example.co.uk", "Lm@example.co.uk"],
            "the second address was lost when the line arrived as {shot}",
        );
    }
}

/// **On a bilingual card the unreadable script takes the name.**
///
/// The app ships the Latin recogniser only, so Arabic comes back as strings of
/// Latin and extended-Latin characters that mean nothing. The plan accepted
/// that for v1 on the grounds that "most UAE trade cards carry the phone, email
/// and company in Latin even when the name is also Arabic — so the fields that
/// matter may already be reachable".
///
/// **This card shows that is worse than not reading the Arabic at all.** The
/// Arabic sits across the top and is the largest text there, so the name rule
/// takes it: the name reads as `älgpqr Kbcud ülömn` and the person's real name
/// is pushed down into `notes`. The company goes the same way. The telephone
/// and the email do survive, so half the premise holds — but a contact filed
/// under mangled script is not one anybody finds again.
///
/// **Ignored, not patched, and deliberately so.** Every cheap way to spot the
/// soup is a bad rule: the characters are ordinary Latin ones, so a
/// character-class test cannot see it, and preferring Title Case would drop
/// `PRANAV MENON` — a real name in all capitals on another card in this same
/// corpus. Lexical evidence separates `älgpqr Kbcud ülömn` from `Rosalyn Vance`
/// immediately, and that is exactly the n-gram feature the classifier is for.
///
/// **What this changes:** it is evidence against Part 4's reasoning rather than
/// against its conclusion. Latin-only may still be right for v1, but the UI
/// notice cannot just say Arabic will not be read — on a bilingual card the
/// name comes out wrong rather than missing.
#[test]
#[ignore = "known: unreadable script wins the name rule on size; needs lexical evidence"]
fn the_latin_name_wins_over_unreadable_script() {
    let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        fixture(include_str!("fixtures/card_mixed_script.json")),
    ));

    assert_eq!(
        parsed.name.as_ref().map(|f| f.value.as_str()),
        Some("Rosalyn Vance"),
        "the name was taken by unreadable script; notes held: {:?}",
        parsed.notes,
    );
}

/// What does survive a bilingual card, so the loss is bounded rather than
/// guessed at. Half of Part 4's premise holds: the ways of reaching somebody
/// are all in Latin and all come through.
#[test]
fn a_bilingual_cards_contact_routes_still_come_through() {
    let parsed = parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(
        fixture(include_str!("fixtures/card_mixed_script.json")),
    ));

    assert_eq!(
        parsed.emails.iter().map(|f| f.value.as_str()).collect::<Vec<_>>(),
        vec!["rvance@example.ae"],
        "the address was lost — note it is printed with a space on both sides of the @",
    );
    assert_eq!(parsed.phones.len(), 2, "a telephone number was lost");
}

/// **The before-number, across every card in the corpus.**
///
/// Not a target and not an approval — a baseline that moves loudly. The plan
/// asks for checkpoint 3 to be measured over thirty to fifty cards; there are
/// eight, so this is coarse. What it lacks in sample it makes up in the one
/// thing a bigger sample would not have given for free: the failures have
/// *different causes*, which is what decides whether a classifier is the answer
/// or a few more rules would do.
///
/// ```text
///   two_column          OK
///   two_column_logo     the logo lockup is taller than the name
///   logo_second_shot    the same, garbled differently by the camera angle
///   three_column        the name welded into the title, so the rule fell
///                       through to "fO in" — two social glyphs
///   label_beside_num    the logo won
///   name_at_bottom      the name is last and alone, so a heading won
///   mixed_heights       OK
///   mixed_script        unreadable Arabic won
/// ```
///
/// **Two of eight, with six distinct causes, and not one of them a threshold.**
/// Every failure is a question about what a line *means* rather than where it
/// sits — which is precisely the split the plan draws between the regex fields
/// and the classified ones. The contact routes bear that out from the other
/// side: after this week's fixes every card gives up its email and its
/// telephones, and those are the fields regex owns.
///
/// Update this deliberately when it changes. A number that quietly improves is
/// as much a problem as one that quietly rots — neither gets read.
#[test]
fn the_name_baseline_across_the_corpus_is_recorded() {
    let corpus: Vec<(&str, Vec<TextSegment>, &str)> = vec![
        ("two_column", card_two_column(), "Firstname Lastname"),
        ("two_column_logo", card_two_column_logo(), "Firstname Lastname"),
        (
            "logo_second_shot",
            fixture(include_str!("fixtures/card_two_column_logo_second_shot.json")),
            "Firstname Lastname",
        ),
        ("three_column", card_three_column(), "Nicolas Wong"),
        ("label_beside_num", card_label_beside_number(), "Owen Hart"),
        (
            "name_at_bottom",
            fixture(include_str!("fixtures/card_name_at_the_bottom.json")),
            "Aidan",
        ),
        (
            "mixed_heights",
            fixture(include_str!("fixtures/card_mixed_line_heights.json")),
            "DEVANI MEHTA",
        ),
        ("mixed_script", fixture(include_str!("fixtures/card_mixed_script.json")), "Rosalyn Vance"),
    ];

    let right: Vec<&str> = corpus
        .into_iter()
        .filter(|(_, segments, want)| {
            let parsed =
                parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(segments.clone()));
            parsed.name.as_ref().map(|f| f.value.as_str()) == Some(*want)
        })
        .map(|(label, _, _)| label)
        .collect();

    assert_eq!(
        right,
        vec!["two_column", "mixed_heights"],
        "the set of cards whose name is read correctly has changed — update this \
         baseline deliberately and say why",
    );
}

/// **The simple form of gutter detection would do nothing on these cards.**
///
/// §2.1.4 proposes replacing the gap threshold with whitespace analysis: project
/// the boxes onto the x-axis, find the corridors of emptiness, cut there. The
/// appeal is that it needs no guessed distance — a column gutter is structural
/// where a tab is a one-off.
///
/// Measured before building it, the plain version does not work. Projecting a
/// whole card finds **no corridor at all** on four of the five single-card
/// fixtures, this one included, and none whatsoever between the name and the
/// title that falsified `MAX_GAP`. The reason is ordinary card design: the
/// address block underneath runs the full width of the card, so every x is
/// covered by something and no column of whitespace survives the projection.
///
/// The recursive form — horizontal bands first, corridors inside each band —
/// does better, but **not on the pair that matters**. It separates the name
/// from the logo on the two-column card, keeps `HSI` with `LIGHTING`, and keeps
/// the `FAX` caption with its number. It does not separate the name from the
/// title here, at any of the four band and corridor settings tried.
///
/// **An earlier revision of this note claimed it did.** That came from a crude
/// probe which cut at every four-pixel horizontal gap; the separation was the
/// over-splitting artefact that same note warned about, not column detection.
/// The claim was wrong and is withdrawn — see
/// `the_name_and_its_title_cannot_be_separated_by_any_projection` below for the
/// structural reason, which no choice of parameters reaches.
///
/// Kept as a test rather than a note because it is a measurement, and a note
/// would go stale the first time somebody moved a fixture.
#[test]
fn a_whole_card_projection_finds_no_column_corridor() {
    let segments = card_three_column();

    let left = segments.iter().map(|s| s.left as i32).min().unwrap();
    let right = segments.iter().map(|s| s.right as i32).max().unwrap();
    let mut covered = vec![false; (right - left + 1) as usize];
    for segment in &segments {
        for x in (segment.left as i32)..=(segment.right as i32) {
            covered[(x - left) as usize] = true;
        }
    }

    // The widest run of x that no box covers, anywhere on the card.
    let mut widest = 0;
    let mut run = 0;
    for is_covered in &covered {
        run = if *is_covered { 0 } else { run + 1 };
        widest = widest.max(run);
    }

    assert!(
        widest < 10,
        "a corridor of {widest}px turned up — if this card now has one, the note \
         on this test is out of date and the simple projection may be worth \
         revisiting",
    );
}

/// **How many cards each photograph holds, across the whole corpus.**
///
/// The companion to `the_name_baseline_across_the_corpus_is_recorded`, and the
/// test that earns the two splitting policies their place. Both were added
/// together and only one of them was caught by anything: disabling "a card
/// carries a telephone number" left every test in the suite passing, which
/// meant it was an untested change sitting in the middle of the guard. An
/// untested fix that happens to be right is indistinguishable from one that is
/// wrong, and this is where the difference would have surfaced — months later,
/// in a splitter nobody had looked at since.
///
/// Nine photographs: eight of one card, one of two. Every one is now cut
/// correctly, where six of nine were over-split before. Unlike the name
/// baseline this asserts *correctness* rather than recording a failure, so it
/// may be tightened but should never be relaxed to fit.
#[test]
fn every_photograph_is_cut_into_the_right_number_of_cards() {
    let corpus: Vec<(&str, Vec<TextSegment>, usize)> = vec![
        ("two_column", card_two_column(), 1),
        ("two_column_logo", card_two_column_logo(), 1),
        (
            "logo_second_shot",
            fixture(include_str!("fixtures/card_two_column_logo_second_shot.json")),
            1,
        ),
        ("three_column", card_three_column(), 1),
        ("label_beside_num", card_label_beside_number(), 1),
        (
            "name_at_bottom",
            fixture(include_str!("fixtures/card_name_at_the_bottom.json")),
            1,
        ),
        (
            "mixed_heights",
            fixture(include_str!("fixtures/card_mixed_line_heights.json")),
            1,
        ),
        ("mixed_script", fixture(include_str!("fixtures/card_mixed_script.json")), 1),
        ("two_cards", card_two_cards_one_photograph(), 2),
    ];

    let wrong: Vec<String> = corpus
        .into_iter()
        .filter_map(|(label, segments, want)| {
            let got = split_cards(segments).len();
            (got != want).then(|| format!("{label}: wanted {want}, got {got}"))
        })
        .collect();

    assert!(wrong.is_empty(), "photographs cut into the wrong number of cards: {wrong:?}");
}

/// **What the three-column card would read as, if the columns were cut apart.**
///
/// The same fixture with its middle and right columns pushed far enough away
/// that nothing can weld across them — which is what correct line assembly
/// would produce, arrived at by moving the boxes rather than by fixing the
/// rule. Everything else in the parser runs untouched.
///
/// It reads perfectly: the name, the title and the company all correct, where
/// today the card gives `fO in` for the name and a three-line weld for the
/// title. **So the weld is the whole of what is wrong with this card**, and no
/// other rule is waiting behind it.
///
/// That is the target for the recursive cut in §2.1.4, and the reason to build
/// it: this test says what success looks like before the work starts, so a cut
/// that lands and does *not* turn `a_three_column_cards_columns_do_not_weld`
/// green has broken something else rather than merely fallen short.
///
/// It also bounds the prize honestly. Of the six cards whose name is read
/// wrongly, this is the only one where line assembly is the cause — the other
/// five have correct lines and a wrong label, and no cut reaches them.
#[test]
fn the_three_column_card_reads_correctly_once_its_columns_are_apart() {
    let separated: Vec<TextSegment> = card_three_column()
        .iter()
        .map(|segment| {
            // Right column, then middle column; the left stays where it is.
            let shift = if segment.left >= 1900.0 {
                6000.0
            } else if segment.left >= 1000.0 {
                3000.0
            } else {
                0.0
            };
            TextSegment {
                left: segment.left + shift,
                top: segment.top,
                right: segment.right + shift,
                bottom: segment.bottom,
                text: segment.text.clone(),
            }
        })
        .collect();

    let parsed =
        parse_card(&pdf_core::contacts::parse::RecognisedCard::around_text(separated));

    assert_eq!(parsed.name.as_ref().map(|f| f.value.as_str()), Some("Nicolas Wong"));
    assert_eq!(parsed.title.as_ref().map(|f| f.value.as_str()), Some("VP and Co-Founder"));
    assert_eq!(
        parsed.company.as_ref().map(|f| f.value.as_str()),
        Some("Riverton VOLTARIS Photoelectric Technology Co, Ltd"),
    );
}

/// **Why no projection separates the name from the title, whatever its
/// parameters.**
///
/// Two facts about this card, asserted here because together they close off a
/// whole family of solutions:
///
/// 1. The name and the title **share a row** — they overlap vertically by more
///    than half the shorter box, which is what put them in front of
///    `shares_a_line` in the first place. So no horizontal cut can place them
///    in different bands: they are the same band by construction.
/// 2. The corridor between them, x 930 to 1015, is **crossed by other boxes** —
///    the email line immediately below spans 131 to 1317, and five full-width
///    lines beneath span most of the card.
///
/// A projection finds a corridor only where one persists across the rows it is
/// projecting. Here it exists on exactly one row, and a single row projected
/// against itself is just the word-gap question again — the very question that
/// falsified `MAX_GAP`. Widening the band, narrowing the corridor, cutting
/// deeper: none of it reaches this, because the corridor is not there to find.
///
/// **So §2.1.4 is worth building and will not fix the card that motivated it.**
/// It repairs the two-column cases and leaves this one. What actually separates
/// `Nicolas Wong` from `VP and Co-Founder` is knowing that one is a person and
/// the other a role, which is lexical, which is the classifier.
///
/// Column alignment was the other candidate and it fails differently: the
/// title's left edge matches the line below it exactly, at 1015, so alignment
/// would separate this pair — but `HSI` aligns with the colour words beneath it
/// while `LIGHTING` aligns with nothing, so alignment splits the logo lockup
/// that must stay whole. Recorded so it is not rediscovered.
#[test]
fn the_name_and_its_title_cannot_be_separated_by_any_projection() {
    let segments = card_three_column();
    let find = |text: &str| segments.iter().find(|s| s.text == text).expect(text).clone();

    let name = find("Nicolas Wong");
    let title = find("VP and Co-Founder");

    // 1. They share a row.
    let overlap = name.bottom.min(title.bottom) - name.top.max(title.top);
    let shorter = (name.bottom - name.top).min(title.bottom - title.top);
    assert!(
        overlap > shorter * 0.5,
        "the name and the title no longer share a row — this card has changed \
         and the reasoning in this note needs revisiting",
    );

    // 2. Something else crosses the corridor between them.
    let (left, right) = (name.right, title.left);
    let crossing: Vec<&str> = segments
        .iter()
        .filter(|s| s.text != name.text && s.text != title.text)
        .filter(|s| s.left <= right && s.right >= left)
        .map(|s| s.text.as_str())
        .collect();
    assert!(
        !crossing.is_empty(),
        "nothing crosses the corridor between the name and the title any more, \
         so a projection could find it after all — revisit §2.1.4",
    );
}
