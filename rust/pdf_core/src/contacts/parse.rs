//! Turning recognised text into a contact.
//!
//! Input is what an OCR engine produced for one card — boxes and their words —
//! and output is a [`BusinessCard`]. Recognition itself is the platform's job
//! (ML Kit on Android, Vision on iOS); deciding which line is a name is not, and
//! lives here so both platforms answer the same way.
//!
//! # This is guesswork, and says so
//!
//! A business card has no structure. There is no rule that the name is first, or
//! that the company is largest, and plenty of cards break every convention on
//! purpose. So every field carries a confidence, and the two passes below produce
//! very different ones:
//!
//! * **Pattern matching** — an email, a URL, a phone number. Nearly certain: a
//!   thing shaped like `name@host.tld` is an email address whatever else is on
//!   the card.
//! * **Structural inference** — the name, the title, the company, the address.
//!   Typography and position, which is a guess dressed up as a rule.
//!
//! Nothing is discarded for being uncertain. A field thrown away cannot be
//! corrected by the person holding the card; a field flagged as doubtful can.
//! Whatever no rule claims goes to `notes`, and the whole recognised text is
//! always kept in `raw_text`.

use crate::contacts::{BusinessCard, Field, PhoneField, PhoneKind, Region};
use serde::{Deserialize, Serialize};

/// One recognised box of text, in the card's own pixel space.
///
/// Deliberately the same shape as the platforms' own segment types, so nothing
/// has to be reshaped on the way in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSegment {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub text: String,
}

impl TextSegment {
    fn height(&self) -> f32 {
        self.bottom - self.top
    }

    fn centre_y(&self) -> f32 {
        (self.top + self.bottom) / 2.0
    }
}

/// What was recognised, and how big the card was.
///
/// The dimensions are not decoration. "The name is the largest text in the upper
/// 60% of the card" needs a card height to take 60% of, and inferring one from
/// the segments' own bounding box is fragile: a card with text only across the
/// top would measure as a short card, and every position rule would shift with
/// it. The caller knows the real size; it should say so.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognisedCard {
    pub width: f32,
    pub height: f32,
    pub segments: Vec<TextSegment>,
    /// What [`RecognisedCard::around_text`] subtracted to bring the card to the
    /// origin, so a field's region can be reported where it actually is on the
    /// photograph. `(0, 0)` for a card already given in its own space.
    #[serde(default)]
    pub origin: (f32, f32),
}

impl RecognisedCard {
    /// A card whose edges were never found, sized to the text sitting on it.
    ///
    /// This is the whole-photograph path: no detection has run, so what arrives
    /// is a picture of a desk with a card somewhere on it, and the photograph's
    /// own dimensions describe the desk. Handing those to [`parse_card`] puts the
    /// name zone across the top of the *desk*, and on any photo where the card
    /// does not already fill the frame the name is not inside it at all.
    ///
    /// The text's own bounding box is a much better guess at the card than the
    /// frame is, so the segments are measured and moved to sit at the origin. It
    /// is still a guess — a card photographed next to a printed receipt would
    /// measure as one wide card spanning both — which is why this is a separate,
    /// named constructor rather than something [`parse_card`] does quietly. A
    /// caller that has actually found the card's rectangle should keep passing
    /// it.
    pub fn around_text(segments: Vec<TextSegment>) -> Self {
        let Some(first) = segments.first() else {
            return RecognisedCard { width: 0.0, height: 0.0, segments, origin: (0.0, 0.0) };
        };

        let (mut left, mut top) = (first.left, first.top);
        let (mut right, mut bottom) = (first.right, first.bottom);
        for segment in &segments {
            left = left.min(segment.left);
            top = top.min(segment.top);
            right = right.max(segment.right);
            bottom = bottom.max(segment.bottom);
        }

        let segments = segments
            .into_iter()
            .map(|segment| TextSegment {
                left: segment.left - left,
                top: segment.top - top,
                right: segment.right - left,
                bottom: segment.bottom - top,
                text: segment.text,
            })
            .collect();

        RecognisedCard {
            width: right - left,
            height: bottom - top,
            segments,
            origin: (left, top),
        }
    }
}

/// How many times one photograph may be divided.
///
/// Eight is 256 cards, far past anyone's desk, and it exists only so that a
/// split which somehow fails to shrink its input cannot recurse forever.
const MAX_SPLITS: u32 = 8;

/// Separate the cards in one photograph.
///
/// Somebody empties their pocket after an event and photographs six cards at
/// once. Recognition sees one picture and returns one pile of text; this decides
/// where one card ends and the next begins, so six contacts come out instead of
/// one contact made of six people.
///
/// ## Why not geometry alone
///
/// The obvious approach — cluster the text by how close it sits — does not work,
/// and the reason is worth stating because it looks like it should. On an
/// ordinary card the name sits at the top and the contact details at the bottom,
/// with a gap between them as wide as the gap between two cards on a desk. Any
/// threshold that separates two cards also cuts one card in half, and the halves
/// are the worst possible ones: a name with no way to contact them, and contact
/// details belonging to nobody.
///
/// ## What actually distinguishes two cards
///
/// **Each card is complete on its own.** A real card carries somebody's name
/// *and* a way of reaching them. One card cut in two does not: the name is on one
/// side and the phone number on the other. So a gap is only cut when both sides
/// independently look like a card — each has a contact method, and each has a
/// line that is not one.
///
/// That is what makes this fail safe. Where the evidence is not there, nothing is
/// split, and the result is the single contact this would have produced anyway.
/// Merging two people into one contact is the worse error — it is silent, and it
/// leaves somebody's phone number filed under somebody else's name — so the
/// doubtful case is resolved by not splitting.
///
/// The gaps are searched largest first, so the divide between two cards is tried
/// before any smaller gap inside one.
pub fn split_cards(segments: Vec<TextSegment>) -> Vec<RecognisedCard> {
    let mut groups = Vec::new();
    divide(segments, 0, &mut groups);
    in_reading_order(&mut groups);
    groups.into_iter().map(RecognisedCard::around_text).collect()
}

fn divide(segments: Vec<TextSegment>, depth: u32, out: &mut Vec<Vec<TextSegment>>) {
    if depth >= MAX_SPLITS || segments.len() < 4 {
        if !segments.is_empty() {
            out.push(segments);
        }
        return;
    }

    match best_split(&segments) {
        Some((first, second)) => {
            divide(first, depth + 1, out);
            divide(second, depth + 1, out);
        }
        None => out.push(segments),
    }
}

/// The widest gap that leaves a whole card on either side of it.
fn best_split(segments: &[TextSegment]) -> Option<(Vec<TextSegment>, Vec<TextSegment>)> {
    let mut candidates: Vec<(f32, Axis, f32)> = Vec::new();
    for (size, at) in gaps(segments, Axis::X) {
        candidates.push((size, Axis::X, at));
    }
    for (size, at) in gaps(segments, Axis::Y) {
        candidates.push((size, Axis::Y, at));
    }
    // Widest first: the space between two cards is bigger than any space inside
    // one, even when it is not bigger by much.
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));

    for (_, axis, at) in candidates {
        let (first, second): (Vec<TextSegment>, Vec<TextSegment>) = segments
            .iter()
            .cloned()
            .partition(|segment| axis.centre(segment) < at);

        if looks_like_a_card(&first) && looks_like_a_card(&second) {
            return Some((first, second));
        }
    }

    None
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

impl Axis {
    fn centre(self, segment: &TextSegment) -> f32 {
        match self {
            Axis::X => (segment.left + segment.right) / 2.0,
            Axis::Y => segment.centre_y(),
        }
    }

    fn near(self, segment: &TextSegment) -> f32 {
        match self {
            Axis::X => segment.left,
            Axis::Y => segment.top,
        }
    }

    fn far(self, segment: &TextSegment) -> f32 {
        match self {
            Axis::X => segment.right,
            Axis::Y => segment.bottom,
        }
    }
}

/// Every empty corridor running clean across the text, as (width, where).
///
/// A corridor only counts when nothing at all crosses it — a single word
/// straddling the divide means these are not two separate cards but one piece of
/// text with a hole in it.
fn gaps(segments: &[TextSegment], axis: Axis) -> Vec<(f32, f32)> {
    let mut ordered: Vec<&TextSegment> = segments.iter().collect();
    ordered.sort_by(|a, b| axis.near(a).total_cmp(&axis.near(b)));

    let Some(first) = ordered.first() else {
        return Vec::new();
    };

    let mut found = Vec::new();
    let mut reach = axis.far(first);
    for segment in ordered.iter().skip(1) {
        let near = axis.near(segment);
        if near > reach {
            found.push((near - reach, (near + reach) / 2.0));
        }
        reach = reach.max(axis.far(segment));
    }
    found
}

/// Whether this much text could stand on its own as somebody's card.
///
/// Somebody's **name** and a way of reaching them. This is the whole guard
/// against cutting one card in half: the top of a card carries the name with no
/// contact details and the bottom carries contact details with no name, so
/// neither half passes and the cut is refused.
///
/// "A line that is not a contact detail" is not enough to count as the name, and
/// the difference is the difference between this working and not. An address is
/// not a contact pattern, so a fragment holding an email and a postal address
/// passes that weaker test — and an ordinary card is duly cut just above its own
/// address, giving one contact with the name and one without. The line has to
/// look like something a person or a company is called.
fn looks_like_a_card(segments: &[TextSegment]) -> bool {
    if segments.len() < 2 {
        return false;
    }

    let mut reachable = false;
    let mut named = false;
    for line in into_lines(segments) {
        if find_email(&line.text).is_some()
            || find_url(&line.text).is_some()
            || !find_phones(&line.text).is_empty()
        {
            reachable = true;
        } else if could_be_a_name(&line.text) || is_a_company(&line.text) {
            named = true;
        }
    }

    reachable && named
}

/// Cards in the order somebody laid them out: across, then down.
fn in_reading_order(groups: &mut [Vec<TextSegment>]) {
    let top_of = |group: &Vec<TextSegment>| {
        group.iter().map(|s| s.top).fold(f32::MAX, f32::min)
    };
    let left_of = |group: &Vec<TextSegment>| {
        group.iter().map(|s| s.left).fold(f32::MAX, f32::min)
    };
    let height_of = |group: &Vec<TextSegment>| {
        group.iter().map(|s| s.bottom).fold(f32::MIN, f32::max) - top_of(group)
    };

    // Rows are worked out first and sorted on as an integer, because a
    // comparator that calls two tops equal when they are merely close is not a
    // total order — and `sort_by` is entitled to panic when handed one.
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| top_of(&groups[a]).total_cmp(&top_of(&groups[b])));

    let mut row_of = vec![0usize; groups.len()];
    let mut row = 0usize;
    for (position, &index) in order.iter().enumerate() {
        if position > 0 {
            let previous = order[position - 1];
            let tolerance = height_of(&groups[index])
                .min(height_of(&groups[previous]))
                .max(1.0)
                * 0.5;
            if top_of(&groups[index]) - top_of(&groups[previous]) > tolerance {
                row += 1;
            }
        }
        row_of[index] = row;
    }

    let mut keyed: Vec<(usize, f32, Vec<TextSegment>)> = groups
        .iter()
        .enumerate()
        .map(|(index, group)| (row_of[index], left_of(group), group.clone()))
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));

    for (slot, (_, _, group)) in keyed.into_iter().enumerate() {
        groups[slot] = group;
    }
}

/// Confidence for something a pattern matched. Not 1.0: the recogniser may
/// still have misread a character inside a perfectly well-shaped address.
const MATCHED: f32 = 0.9;

/// Confidence for something inferred from where it sits and how big it is.
const INFERRED: f32 = 0.55;

/// A weaker inference — the rule fired, but on thin evidence.
const WEAK: f32 = 0.35;

/// How far down the card the name is looked for.
const NAME_ZONE: f32 = 0.6;

/// Read a card.
pub fn parse_card(card: &RecognisedCard) -> BusinessCard {
    let lines = into_lines(&card.segments);
    let raw_text = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // Claimed lines are struck off, so a later pass cannot take an email address
    // and call it a company name.
    let mut pool: Vec<Line> = lines;
    let mut result = BusinessCard { raw_text, ..Default::default() };

    take_patterns(&mut pool, &mut result, card.origin);
    take_structure(&mut pool, &mut result, card.height, card.origin);

    // Whatever nothing claimed. Kept rather than dropped: this is where a second
    // phone number the pattern missed, or a tagline, or a second language ends
    // up, and it is the difference between "the parser missed it" and "it is
    // gone".
    let leftover: Vec<&str> = pool.iter().map(|line| line.text.as_str()).collect();
    if !leftover.is_empty() {
        result.notes = Some(leftover.join("\n"));
    }
    result
}

// ------------------------------------------------------------------- lines --

/// A row of text: one or more segments that sit at the same height.
#[derive(Debug, Clone)]
struct Line {
    text: String,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    /// The median height of this line's boxes — the proxy for font size, and
    /// what identifies a name.
    glyph_height: f32,
}

impl Line {
    fn centre_y(&self) -> f32 {
        (self.top + self.bottom) / 2.0
    }

    /// Where this line sits on the photograph, undoing the shift `around_text`
    /// applied so the rules could work in the card's own space.
    fn region(&self, origin: (f32, f32)) -> Region {
        Region {
            left: self.left + origin.0,
            top: self.top + origin.1,
            right: self.right + origin.0,
            bottom: self.bottom + origin.1,
        }
    }
}

/// Group segments into lines by vertical overlap.
///
/// Recognisers return a card as scattered boxes, often one per word, and every
/// rule below is about lines. Two boxes belong to the same line when their
/// vertical spans overlap by more than half the shorter one — proportional
/// rather than a fixed tolerance, because a card mixes 8pt and 20pt text and a
/// tolerance that suits one splits the other.
fn into_lines(segments: &[TextSegment]) -> Vec<Line> {
    let mut sorted: Vec<&TextSegment> = segments
        .iter()
        .filter(|segment| !segment.text.trim().is_empty())
        .collect();
    // Left to right within a row, so two boxes at the same height are always
    // considered in the same order. A greedy pass over an unstable sort gives a
    // different answer for the same card depending on which box the recogniser
    // happened to list first, and that turns a layout bug into an intermittent
    // one.
    sorted.sort_by(|a, b| {
        a.centre_y()
            .total_cmp(&b.centre_y())
            .then_with(|| a.left.total_cmp(&b.left))
    });

    let mut lines: Vec<Vec<&TextSegment>> = Vec::new();
    for segment in sorted {
        // **Every open line, not just the last.** Two columns interleave once
        // they are no longer welded together: sorted by centre, a card gives
        // right, left, right, left — so the second box of the right column meets
        // a *left*-column line as the most recent one, fails against it, and
        // opens a line of its own. The right column would fragment into one line
        // per box, which is the bug the horizontal test uncovers rather than
        // causes.
        //
        // The best match rather than the first, so the result does not depend on
        // the order lines happen to have been opened in.
        let best = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                line.iter()
                    .filter(|other| shares_a_line(other, segment))
                    .filter_map(|other| vertical_overlap(other, segment))
                    .fold(None, |most: Option<f32>, overlap| {
                        Some(most.map_or(overlap, |m| m.max(overlap)))
                    })
                    .map(|overlap| (index, overlap))
            })
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(index, _)| index);

        match best {
            Some(index) => lines[index].push(segment),
            None => lines.push(vec![segment]),
        }
    }

    lines
        .into_iter()
        .map(|mut parts| {
            // Left to right, so the words read in order. The recogniser returns
            // them in whatever order it found them.
            parts.sort_by(|a, b| {
                a.left.partial_cmp(&b.left).unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut heights: Vec<f32> = parts.iter().map(|p| p.height()).collect();
            heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            Line {
                text: parts
                    .iter()
                    .map(|p| p.text.trim())
                    .collect::<Vec<_>>()
                    .join(" "),
                left: parts.iter().map(|p| p.left).fold(f32::MAX, f32::min),
                right: parts.iter().map(|p| p.right).fold(f32::MIN, f32::max),
                top: parts.iter().map(|p| p.top).fold(f32::MAX, f32::min),
                bottom: parts.iter().map(|p| p.bottom).fold(f32::MIN, f32::max),
                glyph_height: heights[heights.len() / 2],
            }
        })
        .collect()
}

fn shares_a_line(a: &TextSegment, b: &TextSegment) -> bool {
    vertical_overlap(a, b).is_some() && horizontal_gap(a, b) < a.height().min(b.height()) * MAX_GAP
}

/// How much two boxes share vertically, or `None` if not enough to be one line.
///
/// Proportional to the shorter box rather than a fixed tolerance, because a card
/// mixes 8pt and 20pt text and one number cannot serve both.
fn vertical_overlap(a: &TextSegment, b: &TextSegment) -> Option<f32> {
    let overlap = a.bottom.min(b.bottom) - a.top.max(b.top);
    let shorter = a.height().min(b.height());
    (shorter > 0.0 && overlap > shorter * 0.5).then_some(overlap)
}

/// The empty space between two boxes horizontally, or zero if they overlap.
fn horizontal_gap(a: &TextSegment, b: &TextSegment) -> f32 {
    (b.left - a.right).max(a.left - b.right).max(0.0)
}

/// How far apart two boxes may sit and still be one line, in line heights.
///
/// **Without this, two columns weld together.** Sharing a row was the whole test,
/// and on a card with the name at the left and the logo at the right the two
/// share a row exactly. Measured on a real card: the name spans y 2382–2462 and
/// the logo's `LIGHTING` spans 2394–2462 — an overlap of 68 against a shorter
/// height of 68 — so they merged, the rest of the logo chained in, and the title
/// chained onto that. One field came back reading
/// `Marketing Manager Abdul Ajees HSI LIGHTING`. The same card also welded its
/// address to a GPS line and its email to its website.
///
/// In line heights rather than pixels, so it holds at any resolution and for any
/// size of card in the frame.
///
/// Two is deliberately generous. The gap it must not allow was **eighteen** line
/// heights, and the gaps it must allow are the spaces between words — well under
/// one. Nothing in the evidence argues for a tighter number, and a tight one
/// would start splitting real lines apart.
const MAX_GAP: f32 = 2.0;

// ----------------------------------------------------------------- patterns --

/// Take everything a pattern can claim: emails, URLs, phone numbers.
fn take_patterns(pool: &mut Vec<Line>, card: &mut BusinessCard, origin: (f32, f32)) {
    let mut claimed = Vec::new();

    for (index, line) in pool.iter().enumerate() {
        let lower = line.text.to_lowercase();

        if let Some(email) = find_email(&line.text) {
            card.emails.push(Field::new(email, MATCHED).at(Some(line.region(origin))));
            claimed.push(index);
            continue;
        }
        if let Some(url) = find_url(&line.text) {
            card.urls.push(Field::new(url, MATCHED).at(Some(line.region(origin))));
            claimed.push(index);
            continue;
        }
        let numbers = find_phones(&line.text);
        if !numbers.is_empty() {
            for number in numbers {
                card.phones.push(PhoneField {
                    normalised: normalise(&number.raw),
                    raw: number.raw,
                    // The label beside the number when there was one — getting
                    // this wrong sends somebody to a fax machine. Failing that,
                    // a word anywhere on the line, which catches the trailing
                    // "(mobile)" form.
                    kind: number.kind.unwrap_or_else(|| phone_kind(&lower)),
                    confidence: MATCHED,
                    region: Some(line.region(origin)),
                });
            }
            claimed.push(index);
        }
    }

    strike_off(pool, &claimed);
}

/// The first thing on the line shaped like an address.
fn find_email(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|word| {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        let (local, host) = word.split_once('@')?;
        // A host with no dot is not a domain, and a local part with none of it
        // is not an address — both appear on cards as decoration.
        if local.is_empty() || !host.contains('.') || host.ends_with('.') {
            return None;
        }
        Some(word.to_string())
    })
}

fn find_url(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|word| {
        let trimmed = word.trim_end_matches(|c: char| ".,;:".contains(c));
        let lower = trimmed.to_lowercase();
        // An email contains a dot and a host too, so it must be excluded here or
        // every address is also claimed as a website.
        if lower.contains('@') {
            return None;
        }
        if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.")
        {
            return Some(trimmed.to_string());
        }
        // A bare domain, which is how most cards print one.
        let looks_like_a_domain = lower.matches('.').count() >= 1
            && KNOWN_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix));
        looks_like_a_domain.then(|| trimmed.to_string())
    })
}

/// Enough of the common ones to catch a bare domain without a scheme.
const KNOWN_SUFFIXES: &[&str] = &[
    ".com", ".net", ".org", ".io", ".co", ".ae", ".uk", ".de", ".fr", ".in",
    ".jp", ".cn", ".ch", ".it", ".es", ".nl", ".se", ".no", ".dk", ".info",
    ".biz", ".me", ".app", ".dev", ".sa", ".qa", ".om", ".bh", ".kw",
];

/// A run of digits long enough to be a telephone number.
///
/// Counted in digits rather than matched as a shape, because a printed number is
/// spaced, bracketed and dashed differently in every country and a pattern that
/// insists on one of them misses the rest.
/// A number found on a line, with the kind the label beside it implied.
pub(crate) struct FoundPhone {
    pub raw: String,
    /// `None` when nothing labelled it, so the caller falls back to the line.
    pub kind: Option<PhoneKind>,
}

/// Every number on a line.
///
/// **A label is evidence, not noise.** "Tel", "Mob", "Fax", "Direct" and the rest
/// are printed next to a number precisely to say it is one, so once a label has
/// been recognised the digits after it are a phone number whatever shape they are
/// in — spaced, dotted, bracketed, hyphenated, or with the country code split off.
/// Working the other way round, and hoping a number matches one of the formats a
/// pattern knows, fails on the first card from a country whose convention was not
/// on the list.
///
/// Reading the label first is also what fixes the reverse mistake: the guard that
/// stops a street address being read as a phone number counts the letters on the
/// line, and the label's own letters used to count towards it. `Mobile 050 123
/// 4567` and `Phone 020 7946 0000` were rejected outright — the two most ordinary
/// ways a card writes a number, thrown out by the rule meant to protect addresses
/// from being misread. Stripping the label before counting keeps the guard and
/// removes the false positive.
///
/// A line can hold more than one number — `T: 020 7946 0000 / F: 020 7946 0001` is
/// an ordinary way to print a landline and a fax — so this returns all of them,
/// each with its own kind.
fn find_phones(text: &str) -> Vec<FoundPhone> {
    let mut found = Vec::new();

    for chunk in phone_chunks(text) {
        let (kind, rest) = strip_phone_label(&chunk);

        let digits = rest.chars().filter(char::is_ascii_digit).count();
        // Below seven is a door number or a suite; above fifteen is longer than
        // E.164 allows and is usually two numbers run together.
        if !(7..=15).contains(&digits) {
            continue;
        }

        // Letters *after* the label mean a sentence that happens to contain
        // numbers — a street address, most often — rather than a number. This
        // runs whether or not a label was found, so "M Anderson +44 7700 900123"
        // is not read as a mobile just because it opens with an M.
        if rest.chars().filter(|c| c.is_alphabetic()).count() > 4 {
            continue;
        }

        let number: String = rest
            .chars()
            .filter(|c| c.is_ascii_digit() || "+()- .".contains(*c))
            .collect();
        let number = number.trim().trim_matches(['.', '-']).trim().to_string();
        if !number.is_empty() {
            found.push(FoundPhone { raw: number, kind });
        }
    }

    found
}

/// Split a line into the parts that might each be a number.
///
/// Two things divide numbers on a card: punctuation between them, and a second
/// label starting a new one. Both are handled, because `T 020 7946 0000 F 020
/// 7946 0001` is printed without any punctuation at all as often as with it.
fn phone_chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();

    for segment in text.split(['|', '/', ',', ';']) {
        let mut current = String::new();
        for token in segment.split_whitespace() {
            // A label opens a new number, so whatever came before it is finished.
            if label_kind(token).is_some() && !current.trim().is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            current.push_str(token);
            current.push(' ');
        }
        if !current.trim().is_empty() {
            chunks.push(current);
        }
    }

    chunks
}

/// Take a leading label off a chunk, and say what it meant.
fn strip_phone_label(chunk: &str) -> (Option<PhoneKind>, &str) {
    let trimmed = chunk.trim_start();
    let letters_end = trimmed
        .char_indices()
        .find(|(_, c)| !c.is_alphabetic())
        .map(|(index, _)| index)
        .unwrap_or(trimmed.len());

    match label_kind(&trimmed[..letters_end]) {
        Some(kind) => (Some(kind), &trimmed[letters_end..]),
        None => (None, trimmed),
    }
}

/// What a single word means beside a number, if anything.
///
/// Matched whole rather than by substring: a single letter has to *be* the token
/// to count, or the "f" in a company name would label a fax number.
fn label_kind(token: &str) -> Option<PhoneKind> {
    let word = token
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    if word.is_empty() {
        return None;
    }

    PHONE_LABELS
        .iter()
        .find(|(label, _)| *label == word)
        .map(|(_, kind)| *kind)
}

/// What cards print beside a number, in the forms they print it.
///
/// Single letters are here because they are extremely common on cards and
/// nowhere else on one: `T`, `M`, `F`, `D` beside a run of digits.
const PHONE_LABELS: &[(&str, PhoneKind)] = &[
    ("mobile", PhoneKind::Cell),
    ("mob", PhoneKind::Cell),
    ("cell", PhoneKind::Cell),
    ("cellular", PhoneKind::Cell),
    ("whatsapp", PhoneKind::Cell),
    ("wa", PhoneKind::Cell),
    ("m", PhoneKind::Cell),
    ("fax", PhoneKind::Fax),
    ("f", PhoneKind::Fax),
    ("home", PhoneKind::Home),
    ("res", PhoneKind::Home),
    ("telephone", PhoneKind::Work),
    ("tel", PhoneKind::Work),
    ("phone", PhoneKind::Work),
    ("ph", PhoneKind::Work),
    ("office", PhoneKind::Work),
    ("direct", PhoneKind::Work),
    ("dd", PhoneKind::Work),
    ("t", PhoneKind::Work),
    ("o", PhoneKind::Work),
];

/// Strip a printed number to something dialable.
///
/// Not full E.164: without knowing the country a number was printed in, a local
/// number cannot be given a country code, and guessing one produces a number
/// that dials somewhere wrong. So a leading `+` is kept and everything else
/// reduced to digits, and the printed form is kept alongside regardless.
fn normalise(raw: &str) -> String {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if raw.trim_start().starts_with('+') {
        format!("+{digits}")
    } else {
        digits
    }
}

fn phone_kind(lower: &str) -> PhoneKind {
    if lower.contains("fax") || lower.starts_with("f:") || lower.contains(" f:") {
        PhoneKind::Fax
    } else if lower.contains("mobile")
        || lower.contains("cell")
        || lower.contains("mob")
        || lower.starts_with("m:")
        || lower.contains(" m:")
    {
        PhoneKind::Cell
    } else if lower.contains("home") {
        PhoneKind::Home
    } else {
        PhoneKind::Work
    }
}

// ---------------------------------------------------------------- structure --

/// Name, title, company and address, from typography and position.
fn take_structure(
    pool: &mut Vec<Line>,
    card: &mut BusinessCard,
    card_height: f32,
    origin: (f32, f32),
) {
    let mut claimed = Vec::new();

    // The name: the biggest text in the upper part of the card. Biggest because
    // that is the one convention cards mostly keep; upper because the bottom is
    // where contact details live, and a large address line would otherwise win.
    let zone = if card_height > 0.0 { card_height * NAME_ZONE } else { f32::MAX };
    // A line carrying a legal suffix is not a person, whatever size it is set
    // in. Without this the rule is decided by box height alone, and box height
    // is a poor proxy for font size: a recogniser measures ink, so a smaller
    // line with descenders ("HSI Lighting LLC") measures nearly as tall as a
    // much larger line without any ("Yaseen Anwar"). On a real card, rendered
    // and read through ML Kit, the company won — the contact was filed under the
    // firm's name with the person's name in the company field.
    let name = pool
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            line.centre_y() <= zone
                && could_be_a_name(&line.text)
                && !is_a_company(&line.text)
        })
        .max_by(|(_, a), (_, b)| {
            a.glyph_height
                .partial_cmp(&b.glyph_height)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, line)| (index, line.clone()));

    if let Some((index, line)) = &name {
        card.name = Some(Field::new(line.text.clone(), INFERRED).at(Some(line.region(origin))));
        claimed.push(*index);

        // No positional guess for the title.
        //
        // "The line below the name is the title" is the obvious rule and it is
        // wrong often enough to hurt: on a two-line card — a name and a company,
        // which is most of them — it takes the company and leaves the field it
        // was meant for empty. A test caught it doing exactly that.
        //
        // So a title needs *evidence*, below, and a line that has none is left in
        // the pool for the company rule or for notes. Being unsure and saying so
        // beats being confidently wrong about which is which.
    }

    // The title: a role word is the evidence. Anywhere on the card, because a
    // title is not reliably anywhere in particular.
    if let Some((index, line)) = pool
        .iter()
        .enumerate()
        .find(|(index, line)| !claimed.contains(index) && has_a_role_word(&line.text))
    {
        card.title = Some(Field::new(line.text.clone(), INFERRED).at(Some(line.region(origin))));
        claimed.push(index);
    }

    // The company: a legal suffix is real evidence. Without one, the largest
    // remaining text, which is a guess and is scored as one.
    let company = pool
        .iter()
        .enumerate()
        .filter(|(index, _)| !claimed.contains(index))
        .find(|(_, line)| is_a_company(&line.text))
        .map(|(index, line)| (index, line.clone(), INFERRED))
        .or_else(|| {
            pool.iter()
                .enumerate()
                .filter(|(index, line)| {
                    !claimed.contains(index) && could_be_a_name(&line.text)
                })
                .max_by(|(_, a), (_, b)| {
                    a.glyph_height
                        .partial_cmp(&b.glyph_height)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(index, line)| (index, line.clone(), WEAK))
        });

    if let Some((index, line, confidence)) = company {
        let region = line.region(origin);
        card.company = Some(Field::new(line.text, confidence).at(Some(region)));
        claimed.push(index);
    }

    // The address: the longest run of unclaimed lines that looks like one.
    let address: Vec<usize> = pool
        .iter()
        .enumerate()
        .filter(|(index, line)| !claimed.contains(index) && looks_like_an_address(&line.text))
        .map(|(index, _)| index)
        .collect();

    if !address.is_empty() {
        let text = address
            .iter()
            .map(|index| pool[*index].text.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        // The union of the lines it was built from, so a multi-line address
        // highlights as one block rather than only its first line.
        let region = address
            .iter()
            .map(|index| pool[*index].region(origin))
            .reduce(|a, b| Region {
                left: a.left.min(b.left),
                top: a.top.min(b.top),
                right: a.right.max(b.right),
                bottom: a.bottom.max(b.bottom),
            });
        card.address = Some(Field::new(text, INFERRED).at(region));
        claimed.extend(address);
    }

    strike_off(pool, &claimed);
}

/// Two to four words, no digits, not obviously a company.
fn could_be_a_name(text: &str) -> bool {
    let words = text.split_whitespace().count();
    (1..=4).contains(&words)
        && !text.chars().any(|c| c.is_ascii_digit())
        && text.chars().any(char::is_alphabetic)
}

fn is_a_company(text: &str) -> bool {
    let lower = text.to_lowercase();
    COMPANY_SUFFIXES
        .iter()
        .any(|suffix| lower.split_whitespace().any(|word| word.trim_matches('.') == *suffix))
}

const COMPANY_SUFFIXES: &[&str] = &[
    "llc", "ltd", "limited", "inc", "incorporated", "gmbh", "fze", "fzc", "pvt",
    "plc", "llp", "bv", "nv", "sa", "srl", "ag", "oy", "ab", "as", "co",
    "corp", "corporation", "company", "group", "holdings", "industries",
];

fn has_a_role_word(text: &str) -> bool {
    let lower = text.to_lowercase();
    ROLE_WORDS.iter().any(|word| lower.contains(word))
}

const ROLE_WORDS: &[&str] = &[
    "manager", "director", "engineer", "ceo", "cto", "cfo", "coo", "head of",
    "president", "founder", "partner", "consultant", "designer", "architect",
    "specialist", "executive", "officer", "supervisor", "coordinator",
    "technician", "analyst", "developer", "sales", "marketing", "account",
];

fn looks_like_an_address(text: &str) -> bool {
    let lower = text.to_lowercase();
    ADDRESS_WORDS
        .iter()
        .any(|word| lower.split(|c: char| !c.is_alphanumeric()).any(|part| part == *word))
        || lower.contains("p.o. box")
        || lower.contains("po box")
}

const ADDRESS_WORDS: &[&str] = &[
    "street", "st", "road", "rd", "avenue", "ave", "lane", "drive", "suite",
    "floor", "building", "block", "box", "office", "tower", "district", "city",
    "area", "zone",
];

/// Remove claimed lines, largest index first so the rest do not shift.
fn strike_off(pool: &mut Vec<Line>, claimed: &[usize]) {
    let mut sorted: Vec<usize> = claimed.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for index in sorted.into_iter().rev() {
        if index < pool.len() {
            pool.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card 1000 × 630, which is roughly what rectification produces.
    const WIDE: f32 = 1000.0;
    const TALL: f32 = 630.0;

    /// One line of text, as a recogniser would return it.
    ///
    /// `size` is the box height, which is the only signal for font size and the
    /// thing every structural rule leans on.
    fn line(text: &str, top: f32, size: f32) -> TextSegment {
        TextSegment {
            left: 60.0,
            top,
            right: 60.0 + text.len() as f32 * size * 0.55,
            bottom: top + size,
            text: text.to_string(),
        }
    }

    fn card(segments: Vec<TextSegment>) -> RecognisedCard {
        RecognisedCard { width: WIDE, height: TALL, segments, origin: (0.0, 0.0) }
    }

    /// An ordinary card, laid out the way most are.
    fn ordinary() -> RecognisedCard {
        card(vec![
            line("Yaseen Anwar", 70.0, 34.0),
            line("Design Engineer", 118.0, 20.0),
            line("HSI Lighting LLC", 165.0, 24.0),
            line("M: +971 50 123 4567", 400.0, 16.0),
            line("dev@hsilighting.com", 440.0, 16.0),
            line("www.hsilighting.com", 480.0, 16.0),
            line("PO Box 1234, Dubai", 520.0, 16.0),
        ])
    }

    #[test]
    fn an_ordinary_card_reads() {
        let parsed = parse_card(&ordinary());

        assert_eq!(parsed.name.as_ref().unwrap().value, "Yaseen Anwar");
        assert_eq!(parsed.company.as_ref().unwrap().value, "HSI Lighting LLC");
        assert_eq!(parsed.title.as_ref().unwrap().value, "Design Engineer");
        assert_eq!(parsed.emails[0].value, "dev@hsilighting.com");
        assert_eq!(parsed.urls[0].value, "www.hsilighting.com");
        assert_eq!(parsed.phones[0].kind, PhoneKind::Cell);
        assert!(parsed.address.as_ref().unwrap().value.contains("PO Box 1234"));
    }

    /// **Each field says where on the photograph it was read.**
    ///
    /// What lets the review screen dim the picture and light up the name where it
    /// actually sits. A region that is merely plausible is worse than none: it
    /// would point confidently at the wrong line, and the whole purpose is to let
    /// somebody check the reading against the card in front of them.
    #[test]
    fn every_read_field_says_where_it_came_from() {
        let parsed = parse_card(&ordinary());

        // The fixture puts each line at a known place, so the regions can be
        // checked against the line they claim rather than merely existing.
        let name = parsed.name.unwrap().region.expect("the name has no region");
        assert_eq!((name.left, name.top, name.bottom), (60.0, 70.0, 104.0));

        let title = parsed.title.unwrap().region.expect("the title has no region");
        assert_eq!((title.top, title.bottom), (118.0, 138.0));

        let company = parsed.company.unwrap().region.expect("the company has no region");
        assert_eq!((company.top, company.bottom), (165.0, 189.0));

        let phone = parsed.phones[0].region.expect("the phone has no region");
        assert_eq!((phone.top, phone.bottom), (400.0, 416.0));

        assert!(parsed.emails[0].region.is_some());
        assert!(parsed.address.unwrap().region.is_some());
    }

    /// A card read from a QR has nowhere on the picture to point at.
    ///
    /// Pointing somewhere arbitrary would be worse than not pointing: the whole
    /// value of a highlight is that it is where the words are.
    #[test]
    fn a_qr_card_claims_no_region() {
        let card = crate::contacts::from_vcard(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Jane Okafor\r\nTEL:+441234567890\r\nEND:VCARD\r\n",
        )
        .expect("that is a vCard");

        assert!(card.name.unwrap().region.is_none());
        assert!(card.phones[0].region.is_none());
    }

    /// The whole recognised text is kept whatever the rules did with it.
    #[test]
    fn nothing_recognised_is_ever_lost() {
        let parsed = parse_card(&ordinary());
        for expected in [
            "Yaseen Anwar",
            "Design Engineer",
            "HSI Lighting LLC",
            "dev@hsilighting.com",
            "PO Box 1234, Dubai",
        ] {
            assert!(
                parsed.raw_text.contains(expected),
                "{expected:?} is missing from the raw text",
            );
        }
    }

    /// A pattern match is worth more than a guess, and the UI flags the
    /// difference — so a confident guess would send nobody to check it.
    #[test]
    fn a_matched_field_outranks_an_inferred_one() {
        let parsed = parse_card(&ordinary());
        assert!(
            parsed.emails[0].confidence > parsed.name.as_ref().unwrap().confidence,
            "an email should be more certain than a name inferred from its size",
        );
    }

    /// The name is the largest text near the top — not simply the first line.
    /// Plenty of cards put a logo or a tagline above it.
    #[test]
    fn the_name_is_the_largest_text_not_the_first() {
        let parsed = parse_card(&card(vec![
            line("Lighting for people", 30.0, 12.0),
            line("Sam Reyes", 80.0, 32.0),
            line("Meridian Systems Ltd", 140.0, 18.0),
        ]));
        assert_eq!(parsed.name.unwrap().value, "Sam Reyes");
    }

    /// Some cards carry a slogan across the bottom, set larger than the name.
    ///
    /// The name zone is what stops it winning. The line has to be one no pattern
    /// claims — an email at the bottom is struck off before the name rule ever
    /// runs, so a test using one passes whether the zone exists or not. This one
    /// did, until the mutation showed it proving nothing.
    #[test]
    fn large_text_low_on_the_card_is_not_the_name() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 22.0),
            line("Meridian Ltd", 110.0, 16.0),
            line("Bright Ideas", 560.0, 44.0),
        ]));
        assert_eq!(parsed.name.unwrap().value, "Sam Reyes");
    }

    /// **A company is never the person, whatever size it is set in.**
    ///
    /// Found on a real card rather than imagined: a rendered card read through
    /// ML Kit came back with the name "HSI Lighting LLC" and the company "Yaseen
    /// Anwar", exactly swapped. The name rule takes the largest text near the
    /// top, and a recogniser measures ink rather than font size — so "HSI
    /// Lighting LLC" at 32pt, which has descenders, measured as tall as "Yaseen
    /// Anwar" at 44pt, which has none. The legal suffix settles it and the box
    /// heights need not be trusted.
    #[test]
    fn a_company_is_not_taken_for_the_person() {
        let parsed = parse_card(&card(vec![
            // Deliberately the taller box, as the recogniser reported it.
            line("HSI Lighting LLC", 130.0, 34.0),
            line("Yaseen Anwar", 60.0, 33.0),
            line("dev@hsilighting.com", 400.0, 16.0),
        ]));

        assert_eq!(parsed.name.unwrap().value, "Yaseen Anwar");
        assert_eq!(parsed.company.unwrap().value, "HSI Lighting LLC");
    }

    /// A legal suffix is evidence; the largest-remaining rule is a guess. The
    /// confidence has to say which happened.
    #[test]
    fn a_legal_suffix_makes_a_company_more_certain() {
        let with = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("Meridian Systems Ltd", 120.0, 18.0),
        ]));
        let without = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("Meridian Systems", 120.0, 18.0),
        ]));

        assert_eq!(with.company.as_ref().unwrap().value, "Meridian Systems Ltd");
        assert_eq!(without.company.as_ref().unwrap().value, "Meridian Systems");
        assert!(
            with.company.unwrap().confidence > without.company.unwrap().confidence,
            "a guess should not claim the certainty of a match",
        );
    }

    /// An email contains a dot and a domain, so a careless URL rule claims it
    /// too — and then the address is gone from the field somebody looks for.
    #[test]
    fn an_email_is_not_also_read_as_a_website() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("sam@meridian.example", 300.0, 16.0),
        ]));
        assert_eq!(parsed.emails.len(), 1);
        assert!(parsed.urls.is_empty(), "the email was also claimed as a URL");
    }

    /// Getting this wrong sends somebody to a fax machine.
    #[test]
    fn phone_labels_decide_the_kind() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("M: +44 7700 900123", 300.0, 16.0),
            line("T: +44 20 7946 0000", 340.0, 16.0),
            line("F: +44 20 7946 0001", 380.0, 16.0),
        ]));
        let kinds: Vec<PhoneKind> = parsed.phones.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec![PhoneKind::Cell, PhoneKind::Work, PhoneKind::Fax]);
    }

    /// A label means the digits after it are a number, whatever shape they are in.
    ///
    /// Cards space, dot, bracket and hyphen numbers differently in every country,
    /// and split the country code off as often as not. The label is the reliable
    /// signal; the formatting is not.
    #[test]
    fn a_labelled_number_is_read_however_it_is_printed() {
        for line_text in [
            "Mobile 050 123 4567",
            "Phone 020 7946 0000",
            "Tel. +971 4 123 4567",
            "Tel: +971.4.123.4567",
            "M +44 (0) 7700 900123",
            "Ph - 020-7946-0000",
            "Direct line: 020 7946 0000",
            "WhatsApp +971 50 123 4567",
            "T +9714 1234567",
        ] {
            let parsed = parse_card(&card(vec![
                line("Sam Reyes", 60.0, 30.0),
                line(line_text, 300.0, 16.0),
            ]));
            assert_eq!(
                parsed.phones.len(),
                1,
                "{line_text:?} was not read as a phone number",
            );
        }
    }

    /// The two most ordinary ways a card prints a number, both of which the
    /// address guard used to reject because it counted the label's own letters.
    #[test]
    fn a_spelled_out_label_does_not_hide_its_number() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("Mobile 050 123 4567", 300.0, 16.0),
            line("Phone 020 7946 0000", 340.0, 16.0),
        ]));
        assert_eq!(parsed.phones.len(), 2, "a labelled number was thrown away");
        assert_eq!(parsed.phones[0].kind, PhoneKind::Cell);
        assert_eq!(parsed.phones[1].kind, PhoneKind::Work);
    }

    /// A landline and a fax share one line on a great many cards.
    #[test]
    fn two_numbers_on_one_line_are_both_read() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("T: 020 7946 0000 / F: 020 7946 0001", 300.0, 16.0),
        ]));
        assert_eq!(parsed.phones.len(), 2, "only one of the two numbers was read");
        assert_eq!(parsed.phones[0].kind, PhoneKind::Work);
        assert_eq!(parsed.phones[1].kind, PhoneKind::Fax);
        assert_eq!(parsed.phones[1].normalised, "02079460001");
    }

    /// Without punctuation between them, which is just as common.
    #[test]
    fn a_second_label_starts_a_second_number() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("T 020 7946 0000 F 020 7946 0001", 300.0, 16.0),
        ]));
        assert_eq!(parsed.phones.len(), 2);
        assert_eq!(parsed.phones[1].kind, PhoneKind::Fax);
    }

    /// A single letter has to *be* the label, not appear in a word — otherwise
    /// the f in a company name labels a fax number.
    #[test]
    fn a_letter_inside_a_word_is_not_a_label() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("Fabrication Unit 12345678", 300.0, 16.0),
        ]));
        assert!(
            parsed.phones.is_empty(),
            "a word beginning with a label letter was read as a number: {:?}",
            parsed.phones,
        );
    }

    /// And a label must not drag a name in with it. The guard that rejects a
    /// sentence has to run on what follows the label, not only on what precedes.
    #[test]
    fn an_initial_before_a_name_is_not_a_phone_label() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("M Anderson +44 7700 900123", 300.0, 16.0),
        ]));
        assert!(
            parsed.phones.is_empty(),
            "a name was read as a labelled phone number: {:?}",
            parsed.phones,
        );
    }

    /// A street address holds numbers and must not be read as a phone number.
    #[test]
    fn an_address_with_numbers_is_not_a_phone_number() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("221 Baker Street, London", 400.0, 16.0),
        ]));
        assert!(
            parsed.phones.is_empty(),
            "a street address was claimed as a telephone number: {:?}",
            parsed.phones,
        );
        assert!(parsed.address.is_some());
    }

    /// A number with a country code keeps it; one without is not given a guessed
    /// one, because a guessed country code dials somewhere wrong.
    #[test]
    fn normalising_never_invents_a_country_code() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("+44 7700 900123", 300.0, 16.0),
            line("020 7946 0000", 340.0, 16.0),
        ]));
        assert_eq!(parsed.phones[0].normalised, "+447700900123");
        assert_eq!(parsed.phones[1].normalised, "02079460000");
        // And the printed form survives either way.
        assert_eq!(parsed.phones[0].raw.trim(), "+44 7700 900123");
    }

    /// **A column must not fragment because another column interleaves with it.**
    ///
    /// Boxes are considered in centre order, so two columns arrive interleaved:
    /// right, left, right. Matching only against the most recently opened line —
    /// which is what this used to do — means the right column's second box meets
    /// a *left*-column line, fails against it, and opens a line of its own. The
    /// right column comes apart, one line per box.
    ///
    /// This never showed while the columns were welded together, because
    /// everything landed in one line regardless. Adding the horizontal test
    /// uncovers it rather than causing it, which is why both halves of the fix
    /// had to land together.
    ///
    /// Synthetic geometry on purpose: this is a property of the grouping rather
    /// than of any card, and the real-card fixtures do not happen to contain the
    /// interleaving that triggers it.
    #[test]
    fn a_column_is_not_fragmented_by_the_one_beside_it() {
        let parsed = parse_card(&card(vec![
            TextSegment {
                left: 200.0, top: 0.0, right: 300.0, bottom: 20.0, text: "HSI".into(),
            },
            // Sorts between the two right-hand boxes, and is far from both.
            TextSegment {
                left: 0.0, top: 9.0, right: 100.0, bottom: 19.0, text: "Sam Reyes".into(),
            },
            TextSegment {
                left: 300.0, top: 8.0, right: 420.0, bottom: 28.0, text: "LIGHTING".into(),
            },
        ]));

        let lines: Vec<&str> = parsed.raw_text.lines().collect();
        assert!(
            lines.contains(&"HSI LIGHTING"),
            "the right column fragmented; lines were {lines:?}",
        );
        assert!(
            lines.contains(&"Sam Reyes"),
            "the left column was disturbed; lines were {lines:?}",
        );
    }

    /// Recognisers return a card as scattered boxes, often one per word.
    #[test]
    fn words_on_the_same_row_become_one_line() {
        let parsed = parse_card(&card(vec![
            TextSegment { left: 60.0, top: 70.0, right: 180.0, bottom: 104.0, text: "Yaseen".into() },
            TextSegment { left: 190.0, top: 72.0, right: 300.0, bottom: 106.0, text: "Anwar".into() },
        ]));
        assert_eq!(parsed.name.unwrap().value, "Yaseen Anwar");
    }

    /// A card with no person on it — a shop, a hotline — is a real thing.
    #[test]
    fn a_company_only_card_still_parses() {
        let parsed = parse_card(&card(vec![
            line("Meridian Systems Ltd", 70.0, 30.0),
            line("hello@meridian.example", 400.0, 16.0),
        ]));
        assert!(parsed.company.is_some() || parsed.name.is_some());
        assert_eq!(parsed.emails[0].value, "hello@meridian.example");
    }

    /// Whatever no rule claimed is kept, not dropped. This is where a tagline,
    /// a second language, or something the parser has no idea about ends up.
    #[test]
    fn unclaimed_text_goes_to_notes() {
        let parsed = parse_card(&card(vec![
            line("Sam Reyes", 60.0, 30.0),
            line("Meridian Systems Ltd", 120.0, 18.0),
            line("Est. 1974", 560.0, 12.0),
        ]));
        assert!(
            parsed.notes.unwrap_or_default().contains("Est. 1974"),
            "text no rule claimed was thrown away",
        );
    }

    /// An empty card is not a crash.
    #[test]
    fn nothing_recognised_is_survivable() {
        let parsed = parse_card(&card(vec![]));
        assert!(parsed.name.is_none());
        assert!(parsed.raw_text.is_empty());
    }

    /// Dimensions come from the caller for a reason: without them the name zone
    /// is meaningless. A card claiming no height must still parse rather than
    /// dividing by zero or rejecting everything.
    #[test]
    fn a_card_with_no_stated_height_still_parses() {
        let parsed = parse_card(&RecognisedCard {
            width: 0.0,
            height: 0.0,
            segments: vec![line("Sam Reyes", 60.0, 30.0), line("Meridian Ltd", 120.0, 18.0)],
            origin: (0.0, 0.0),
        });
        assert_eq!(parsed.name.unwrap().value, "Sam Reyes");
    }

    // ------------------------------------------- a card inside a photograph --

    /// The card from [`ordinary`], sitting somewhere in the middle of a photo of
    /// a desk — which is what arrives before any detection exists.
    fn photographed(offset_x: f32, offset_y: f32) -> Vec<TextSegment> {
        ordinary()
            .segments
            .into_iter()
            .map(|s| TextSegment {
                left: s.left + offset_x,
                top: s.top + offset_y,
                right: s.right + offset_x,
                bottom: s.bottom + offset_y,
                text: s.text,
            })
            .collect()
    }

    /// Why [`RecognisedCard::around_text`] exists.
    ///
    /// Passing the photograph's own dimensions puts the name zone across the top
    /// 60% of the *desk*. The card is lower than that, so the name is not in the
    /// zone and the rule cannot find it. If this ever starts passing, the
    /// constructor below has stopped earning its place.
    #[test]
    fn the_photographs_own_dimensions_lose_the_name() {
        let parsed = parse_card(&RecognisedCard {
            width: 3000.0,
            height: 4000.0,
            segments: photographed(800.0, 2600.0),
            origin: (0.0, 0.0),
        });
        assert_ne!(
            parsed.name.map(|n| n.value).unwrap_or_default(),
            "Yaseen Anwar",
            "the name was found without the card being measured, so nothing \
             below is being tested",
        );
    }

    /// And with the constructor, the same card reads the same wherever in the
    /// frame it was photographed.
    #[test]
    fn a_card_lost_in_a_photograph_reads_the_same() {
        let framed = parse_card(&ordinary());
        let shot = parse_card(&RecognisedCard::around_text(photographed(800.0, 2600.0)));

        assert_eq!(shot.name.as_ref().unwrap().value, "Yaseen Anwar");
        assert_eq!(shot.company.as_ref().unwrap().value, "HSI Lighting LLC");
        assert_eq!(shot.title.as_ref().unwrap().value, "Design Engineer");

        // Everything *read* is identical; only where it was read differs, and
        // that is the point. Regions are in the photograph's own space, so a card
        // further down the picture must report itself further down.
        assert_eq!(shot.raw_text, framed.raw_text);
        assert_eq!(
            shot.phones.iter().map(|p| p.raw.clone()).collect::<Vec<_>>(),
            framed.phones.iter().map(|p| p.raw.clone()).collect::<Vec<_>>(),
            "where the card sat in the frame changed what was read off it",
        );

        let here = shot.name.unwrap().region.expect("the name has no region");
        let there = framed.name.unwrap().region.expect("the name has no region");
        assert_eq!(
            (here.left - there.left, here.top - there.top),
            (800.0, 2600.0),
            "the region was not reported where the card actually is",
        );
    }

    /// Position within the frame must not survive into the card's own space —
    /// a card measured but not moved keeps the desk's origin, and every position
    /// rule stays as wrong as it was.
    #[test]
    fn measuring_the_text_also_moves_it_to_the_origin() {
        let measured = RecognisedCard::around_text(photographed(800.0, 2600.0));
        let topmost = measured
            .segments
            .iter()
            .fold(f32::MAX, |lowest, s| lowest.min(s.top));
        assert!(
            topmost.abs() < 0.001,
            "the topmost line sits at {topmost}, not at the top of the card",
        );
        assert!(measured.height < 600.0, "the card measured as {} tall", measured.height);
    }

    /// The JSON the platforms actually send.
    ///
    /// [`TextSegment`] is decoded by serde, which rejects the whole array over
    /// one wrong key — so a misspelled name makes *every* scan fail in the same
    /// way, looking like recognition not working rather than a typo. Nothing
    /// else covers this: the parser's own tests build segments in Rust and never
    /// go near the wire format.
    ///
    /// The literal below is what Android's `CardScanner.segmentJson` writes, and
    /// its `CardScannerTest` asserts it still writes exactly this. Either side
    /// changing alone breaks the other's test.
    #[test]
    fn the_json_android_sends_decodes() {
        let sent = r#"[
            {"left":60,"top":70,"right":300,"bottom":104,"text":"Yaseen Anwar"},
            {"left":60,"top":118,"right":280,"bottom":138,"text":"HSI Lighting LLC"}
        ]"#;

        let segments: Vec<TextSegment> =
            serde_json::from_str(sent).expect("the recognised text could not be decoded");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "Yaseen Anwar");
        assert_eq!(segments[0].bottom, 104.0);

        let parsed = parse_card(&RecognisedCard::around_text(segments));
        assert_eq!(parsed.name.unwrap().value, "Yaseen Anwar");
        assert_eq!(parsed.company.unwrap().value, "HSI Lighting LLC");
    }

    // ------------------------------------------ several cards in one picture --

    /// A second card, with nothing in common with [`ordinary`].
    fn second_card(offset_x: f32, offset_y: f32) -> Vec<TextSegment> {
        vec![
            line("Priya Raman", 70.0, 34.0),
            line("Head of Purchasing", 118.0, 20.0),
            line("Northwind Traders Ltd", 165.0, 24.0),
            line("T: +44 20 7946 0100", 400.0, 16.0),
            line("priya@northwind.example", 440.0, 16.0),
        ]
        .into_iter()
        .map(|s| TextSegment {
            left: s.left + offset_x,
            top: s.top + offset_y,
            right: s.right + offset_x,
            bottom: s.bottom + offset_y,
            text: s.text,
        })
        .collect()
    }

    /// **The case that makes a naive splitter useless.**
    ///
    /// One card has a gap between its name and its contact details as wide as the
    /// gap between two cards on a desk. Cutting there produces a name nobody can
    /// contact and a phone number belonging to nobody — worse than not splitting
    /// at all.
    #[test]
    fn one_card_is_never_cut_in_half() {
        let cards = split_cards(ordinary().segments);
        assert_eq!(cards.len(), 1, "a single card was split into pieces");

        let parsed = parse_card(&cards[0]);
        assert_eq!(parsed.name.unwrap().value, "Yaseen Anwar");
        assert_eq!(parsed.emails[0].value, "dev@hsilighting.com");
    }

    #[test]
    fn two_cards_side_by_side_are_read_separately() {
        let mut segments = ordinary().segments;
        segments.extend(second_card(1400.0, 0.0));

        let cards = split_cards(segments);
        assert_eq!(cards.len(), 2, "two cards did not come out as two");

        let read: Vec<BusinessCard> = cards.iter().map(parse_card).collect();
        assert_eq!(read[0].name.as_ref().unwrap().value, "Yaseen Anwar");
        assert_eq!(read[1].name.as_ref().unwrap().value, "Priya Raman");
        // And nothing crossed between them, which is the failure that matters:
        // one person's number filed under another person's name.
        assert_eq!(read[0].emails[0].value, "dev@hsilighting.com");
        assert_eq!(read[1].emails[0].value, "priya@northwind.example");
        assert_eq!(read[1].company.as_ref().unwrap().value, "Northwind Traders Ltd");
    }

    #[test]
    fn two_cards_one_above_the_other_are_read_separately() {
        let mut segments = ordinary().segments;
        segments.extend(second_card(0.0, 900.0));

        let cards = split_cards(segments);
        assert_eq!(cards.len(), 2);

        let read: Vec<BusinessCard> = cards.iter().map(parse_card).collect();
        assert_eq!(read[0].name.as_ref().unwrap().value, "Yaseen Anwar");
        assert_eq!(read[1].name.as_ref().unwrap().value, "Priya Raman");
    }

    /// Four on a desk, and they come back in the order they were laid out.
    #[test]
    fn a_grid_of_cards_comes_back_in_reading_order() {
        let mut segments = Vec::new();
        segments.extend(ordinary().segments); // top left
        segments.extend(second_card(1400.0, 0.0)); // top right
        segments.extend(second_card(0.0, 900.0)); // bottom left
        segments.extend(ordinary().segments.into_iter().map(|s| TextSegment {
            left: s.left + 1400.0,
            top: s.top + 900.0,
            right: s.right + 1400.0,
            bottom: s.bottom + 900.0,
            text: s.text,
        }));

        let cards = split_cards(segments);
        assert_eq!(cards.len(), 4, "four cards did not come out as four");

        let names: Vec<String> = cards
            .iter()
            .map(|card| parse_card(card).name.map(|n| n.value).unwrap_or_default())
            .collect();
        assert_eq!(
            names,
            vec!["Yaseen Anwar", "Priya Raman", "Priya Raman", "Yaseen Anwar"],
            "the cards came back in the wrong order",
        );
    }

    /// Half a card is not a card: a name with no way to reach them, or contact
    /// details belonging to nobody. This is the rule the guard rests on.
    #[test]
    fn half_a_card_is_not_mistaken_for_one() {
        let top_half = vec![
            line("Yaseen Anwar", 70.0, 34.0),
            line("Design Engineer", 118.0, 20.0),
            line("HSI Lighting LLC", 165.0, 24.0),
        ];
        let bottom_half = vec![
            line("M: +971 50 123 4567", 400.0, 16.0),
            line("dev@hsilighting.com", 440.0, 16.0),
        ];

        assert!(
            !looks_like_a_card(&top_half),
            "a name with no way of reaching them was taken for a whole card",
        );
        assert!(
            !looks_like_a_card(&bottom_half),
            "contact details belonging to nobody were taken for a whole card",
        );
    }

    /// Text that no rule can divide stays whole rather than being scattered.
    #[test]
    fn a_photograph_of_nothing_in_particular_stays_in_one_piece() {
        let cards = split_cards(vec![
            line("Est. 1974", 60.0, 14.0),
            line("Lighting for people", 100.0, 14.0),
            line("Open Monday to Friday", 140.0, 14.0),
            line("Closed on public holidays", 180.0, 14.0),
        ]);
        assert_eq!(cards.len(), 1);
    }

    #[test]
    fn splitting_nothing_is_survivable() {
        assert!(split_cards(vec![]).is_empty());
    }

    /// A photograph of a blank wall recognises nothing. That is an empty card,
    /// not a division by zero.
    #[test]
    fn measuring_nothing_is_survivable() {
        let measured = RecognisedCard::around_text(vec![]);
        assert_eq!(measured.width, 0.0);
        assert_eq!(measured.height, 0.0);
        assert!(parse_card(&measured).name.is_none());
    }
}
