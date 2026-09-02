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

use crate::contacts::{BusinessCard, Field, PhoneField, PhoneKind};
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
            return RecognisedCard { width: 0.0, height: 0.0, segments };
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

        RecognisedCard { width: right - left, height: bottom - top, segments }
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

    take_patterns(&mut pool, &mut result);
    take_structure(&mut pool, &mut result, card.height);

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
    sorted.sort_by(|a, b| {
        a.centre_y()
            .partial_cmp(&b.centre_y())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut lines: Vec<Vec<&TextSegment>> = Vec::new();
    for segment in sorted {
        let joined = lines.last_mut().filter(|line| {
            line.iter().any(|other: &&TextSegment| shares_a_line(other, segment))
        });
        match joined {
            Some(line) => line.push(segment),
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
                top: parts.iter().map(|p| p.top).fold(f32::MAX, f32::min),
                bottom: parts.iter().map(|p| p.bottom).fold(f32::MIN, f32::max),
                glyph_height: heights[heights.len() / 2],
            }
        })
        .collect()
}

fn shares_a_line(a: &TextSegment, b: &TextSegment) -> bool {
    let overlap = a.bottom.min(b.bottom) - a.top.max(b.top);
    let shorter = a.height().min(b.height());
    shorter > 0.0 && overlap > shorter * 0.5
}

// ----------------------------------------------------------------- patterns --

/// Take everything a pattern can claim: emails, URLs, phone numbers.
fn take_patterns(pool: &mut Vec<Line>, card: &mut BusinessCard) {
    let mut claimed = Vec::new();

    for (index, line) in pool.iter().enumerate() {
        let lower = line.text.to_lowercase();

        if let Some(email) = find_email(&line.text) {
            card.emails.push(Field::new(email, MATCHED));
            claimed.push(index);
            continue;
        }
        if let Some(url) = find_url(&line.text) {
            card.urls.push(Field::new(url, MATCHED));
            claimed.push(index);
            continue;
        }
        if let Some(number) = find_phone(&line.text) {
            card.phones.push(PhoneField {
                normalised: normalise(&number),
                raw: number,
                // From the words beside it: "M:" and "mobile" mean a mobile, and
                // getting this wrong sends somebody to a fax machine.
                kind: phone_kind(&lower),
                confidence: MATCHED,
            });
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
fn find_phone(text: &str) -> Option<String> {
    let digits = text.chars().filter(char::is_ascii_digit).count();
    if !(7..=15).contains(&digits) {
        return None;
    }
    // Letters mean it is a sentence that happens to contain numbers — a street
    // address, most often — rather than a phone number.
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let label = text
        .split(|c: char| c == ':' || c == '|')
        .next_back()
        .unwrap_or(text);
    if letters > 4 && label.chars().filter(|c| c.is_alphabetic()).count() > 4 {
        return None;
    }

    let number: String = label
        .chars()
        .filter(|c| c.is_ascii_digit() || "+()- .".contains(*c))
        .collect();
    let number = number.trim().to_string();
    (!number.is_empty()).then_some(number)
}

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
fn take_structure(pool: &mut Vec<Line>, card: &mut BusinessCard, card_height: f32) {
    let mut claimed = Vec::new();

    // The name: the biggest text in the upper part of the card. Biggest because
    // that is the one convention cards mostly keep; upper because the bottom is
    // where contact details live, and a large address line would otherwise win.
    let zone = if card_height > 0.0 { card_height * NAME_ZONE } else { f32::MAX };
    let name = pool
        .iter()
        .enumerate()
        .filter(|(_, line)| line.centre_y() <= zone && could_be_a_name(&line.text))
        .max_by(|(_, a), (_, b)| {
            a.glyph_height
                .partial_cmp(&b.glyph_height)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, line)| (index, line.clone()));

    if let Some((index, line)) = &name {
        card.name = Some(Field::new(line.text.clone(), INFERRED));
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
        card.title = Some(Field::new(line.text.clone(), INFERRED));
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
        card.company = Some(Field::new(line.text, confidence));
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
        card.address = Some(Field::new(text, INFERRED));
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
        RecognisedCard { width: WIDE, height: TALL, segments }
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
        assert_eq!(
            shot, framed,
            "where the card sat in the frame changed what was read off it",
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
