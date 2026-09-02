//! Contacts read off business cards, and the vCard they are exported as.
//!
//! The point of the feature is a contact you can send on **and know when you
//! sent it**. So the export timestamp is not incidental here: it goes into the
//! file as `REV`, which is what makes an exported contact self-describing once
//! it has left the app.
//!
//! Everything in this module is platform-neutral and host-testable. Recognition
//! is not — that is ML Kit on Android and Vision on iOS — but what the words mean
//! and how they are written down is the same everywhere.

pub mod parse;

use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// One recognised value, and how sure we are of it.
///
/// Confidence is carried all the way to the UI rather than being thresholded
/// here, because "probably wrong" is worth showing somebody and worth keeping.
/// A field dropped for being uncertain cannot be corrected; a field flagged as
/// uncertain can.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub value: String,
    /// 0.0 to 1.0.
    pub confidence: f32,
}

impl Field {
    pub fn new(value: impl Into<String>, confidence: f32) -> Self {
        Field { value: value.into(), confidence: confidence.clamp(0.0, 1.0) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PhoneKind {
    Cell,
    Work,
    Fax,
    Home,
}

impl PhoneKind {
    /// The `TYPE=` token vCard expects.
    fn token(self) -> &'static str {
        match self {
            PhoneKind::Cell => "CELL",
            PhoneKind::Work => "WORK",
            PhoneKind::Fax => "FAX",
            PhoneKind::Home => "HOME",
        }
    }
}

/// A phone number, kept twice on purpose.
///
/// `raw` is exactly what was printed on the card and `normalised` is E.164 where
/// it could be parsed. Both are kept because normalising can be wrong — a number
/// without a country code has to be guessed at — and the printed form is the only
/// evidence of what the card actually said.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneField {
    pub raw: String,
    pub normalised: String,
    pub kind: PhoneKind,
    pub confidence: f32,
}

/// Everything read off one card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BusinessCard {
    pub name: Option<Field>,
    pub title: Option<Field>,
    pub company: Option<Field>,
    pub phones: Vec<PhoneField>,
    pub emails: Vec<Field>,
    pub urls: Vec<Field>,
    pub address: Option<Field>,
    pub notes: Option<String>,
    /// Everything the recogniser produced, never discarded.
    ///
    /// The parser will always miss something — a second phone number, a line in
    /// a script it does not handle — and this is what makes that recoverable
    /// rather than lost.
    pub raw_text: String,
}

// ------------------------------------------------------------------ writing --

/// Write a card as vCard 3.0.
///
/// **3.0 rather than 4.0.** 4.0 is newer and better specified, and support for it
/// is still patchy: iOS Contacts, Google Contacts and Outlook all take 3.0
/// without argument. A contact that will not import is worth nothing, so the
/// older and duller format wins.
///
/// `exported_at` is an RFC 3339 timestamp in UTC, written as `REV`. It is the
/// reason this feature exists — see the module note.
pub fn to_vcard(card: &BusinessCard, exported_at: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("BEGIN:VCARD".into());
    lines.push("VERSION:3.0".into());

    // N before FN. Both are written even though FN is the only one required,
    // because importers that build a sort key use N and fall back to something
    // unhelpful without it.
    if let Some(name) = &card.name {
        let (family, given) = split_name(&name.value);
        lines.push(format!("N:{};{};;;", escape(&family), escape(&given)));
        lines.push(format!("FN:{}", escape(&name.value)));
    } else if let Some(company) = &card.company {
        // A card with no person on it is a real thing — a shop, a hotline. FN is
        // mandatory, so the company stands in for it rather than emitting a
        // vCard that some importers reject outright.
        lines.push(format!("FN:{}", escape(&company.value)));
    }

    if let Some(company) = &card.company {
        lines.push(format!("ORG:{}", escape(&company.value)));
    }
    if let Some(title) = &card.title {
        lines.push(format!("TITLE:{}", escape(&title.value)));
    }

    for phone in &card.phones {
        // The normalised form where there is one: an importer can dial E.164
        // anywhere, and a number printed without a country code cannot be.
        let number = if phone.normalised.is_empty() {
            &phone.raw
        } else {
            &phone.normalised
        };
        lines.push(format!("TEL;TYPE={}:{}", phone.kind.token(), escape(number)));
    }

    for email in &card.emails {
        lines.push(format!("EMAIL;TYPE=INTERNET:{}", escape(&email.value)));
    }
    for url in &card.urls {
        lines.push(format!("URL:{}", escape(&url.value)));
    }

    if let Some(address) = &card.address {
        // ADR has seven components and we have one blob of text, so it goes in
        // the street slot. Splitting a printed address into street, city and
        // country reliably is a harder problem than reading the card was, and
        // guessing wrong moves a city into a postcode field where nobody can
        // find it.
        lines.push(format!("ADR;TYPE=WORK:;;{};;;;", escape(&address.value)));
    }

    if let Some(notes) = &card.notes {
        if !notes.trim().is_empty() {
            lines.push(format!("NOTE:{}", escape(notes)));
        }
    }

    lines.push(format!("REV:{}", escape(exported_at)));
    lines.push("END:VCARD".into());

    let mut out = String::new();
    for line in lines {
        for folded in fold(&line) {
            // CRLF, not LF. The spec says so, and importers that read the file
            // strictly will otherwise take the whole card as one malformed line.
            let _ = write!(out, "{folded}\r\n");
        }
    }
    out
}

/// Several cards in one file.
///
/// A multi-contact `.vcf` is simply concatenated cards — there is no wrapper.
pub fn to_vcards(cards: &[BusinessCard], exported_at: &str) -> String {
    cards.iter().map(|card| to_vcard(card, exported_at)).collect()
}

/// Escape the four characters that change a vCard's meaning.
///
/// The one that actually bites is the comma. A company called "Smith, Jones and
/// Partners" written unescaped is read as a *list*, and most importers keep only
/// "Smith" — silently, with no error anywhere. Backslash first, or it escapes
/// the escapes the other rules just added.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => out.push_str(r"\\"),
            ',' => out.push_str(r"\,"),
            ';' => out.push_str(r"\;"),
            '\n' => out.push_str(r"\n"),
            '\r' => {}
            _ => out.push(character),
        }
    }
    out
}

/// Break a long line into a first line and continuations.
///
/// vCard limits a line to 75 octets and continues on the next line beginning
/// with a single space. Counted in **octets, not characters**: a line of Arabic
/// or Chinese is two or three bytes per character, so counting characters folds
/// far too late and produces lines a strict parser rejects.
///
/// Splits only on character boundaries, so a multi-byte character is never cut
/// in half — which would produce invalid UTF-8 in the middle of a name.
fn fold(line: &str) -> Vec<String> {
    const LIMIT: usize = 75;

    if line.len() <= LIMIT {
        return vec![line.to_string()];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    // The first line may hold 75 octets; a continuation carries a leading space,
    // so it may hold 74 of its own.
    let mut budget = LIMIT;

    for character in line.chars() {
        let width = character.len_utf8();
        if current.len() + width > budget {
            out.push(if out.is_empty() {
                current.clone()
            } else {
                format!(" {current}")
            });
            current.clear();
            budget = LIMIT - 1;
        }
        current.push(character);
    }
    if !current.is_empty() {
        out.push(if out.is_empty() { current } else { format!(" {current}") });
    }
    out
}

/// Split a printed name into family and given parts.
///
/// The last whitespace-separated word is taken as the family name, which is
/// right for most of what appears on a Latin-script business card and wrong for
/// plenty of the rest. It is a *guess*, and it only affects sort order in the
/// importer — `FN` carries the name as printed, which is what anybody reads.
fn split_name(full: &str) -> (String, String) {
    let words: Vec<&str> = full.split_whitespace().collect();
    match words.len() {
        0 => (String::new(), String::new()),
        1 => (words[0].to_string(), String::new()),
        _ => (
            words[words.len() - 1].to_string(),
            words[..words.len() - 1].join(" "),
        ),
    }
}

// ------------------------------------------------------------------ reading --

/// Read a vCard back into a card.
///
/// This is the QR path. A growing share of business cards carry a QR code that
/// encodes a complete vCard, and when one does the data is **exact rather than
/// recognised** — no detection, no rectification, no OCR, no parsing guesswork.
/// It is the cheapest and most accurate route a card can take through this
/// feature, so every field it produces is given full confidence.
///
/// Deliberately forgiving. A vCard in a QR code was generated by somebody else's
/// software and is frequently sloppy: bare newlines instead of CRLF, lower-case
/// property names, unknown properties, a missing `VERSION`. None of that is worth
/// refusing a contact over. Anything unrecognised stays in `raw_text`.
///
/// Returns `None` only when the text is not a usable vCard — which is the case
/// worth detecting, because a QR holding a plain URL should fall through to the
/// OCR path rather than produce an empty contact.
pub fn from_vcard(text: &str) -> Option<BusinessCard> {
    if !text.to_ascii_uppercase().contains("BEGIN:VCARD") {
        return None;
    }

    let mut card = BusinessCard { raw_text: text.to_string(), ..Default::default() };
    let mut full_name: Option<String> = None;
    let mut structured_name: Option<String> = None;

    for line in unfold(text) {
        // `PROPERTY;PARAM=VALUE:the value`. Split on the first colon only: the
        // value routinely contains more of them, a URL being the obvious case.
        let Some((head, value)) = line.split_once(':') else {
            continue;
        };
        // Unescaping is deliberately *not* done yet. The structured properties
        // below are semicolon-separated lists, and a semicolon inside a value is
        // escaped — so unescaping first turns "Smith; Jones" into a two-item
        // list and throws half of it away. The round-trip test caught exactly
        // that. Split on the structure first, unescape the pieces after.
        if value.trim().is_empty() {
            continue;
        }

        let mut parts = head.split(';');
        let property = parts.next().unwrap_or("").trim().to_ascii_uppercase();
        let params: Vec<String> = parts.map(|p| p.trim().to_ascii_uppercase()).collect();

        match property.as_str() {
            "FN" => full_name = Some(unescape(value)),
            "N" => {
                // `family;given;middle;prefix;suffix`. Read back into the printed
                // order, and used only when there is no FN — FN is what somebody
                // actually wrote on the card.
                let fields = components(value);
                let family = fields.first().map(String::as_str).unwrap_or("").trim();
                let given = fields.get(1).map(String::as_str).unwrap_or("").trim();
                let joined = format!("{given} {family}").trim().to_string();
                if !joined.is_empty() {
                    structured_name = Some(joined);
                }
            }
            "ORG" => {
                // ORG is a list: `Company;Department`. The first part is the
                // company; what follows is a subdivision nobody puts on a
                // contact's name line.
                let company = components(value)
                    .into_iter()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if !company.is_empty() {
                    card.company = Some(Field::new(company, 1.0));
                }
            }
            "TITLE" | "ROLE" => card.title = Some(Field::new(unescape(value), 1.0)),
            "TEL" => card.phones.push(PhoneField {
                raw: unescape(value),
                normalised: unescape(value),
                kind: phone_kind(&params),
                confidence: 1.0,
            }),
            "EMAIL" => card.emails.push(Field::new(unescape(value), 1.0)),
            "URL" => card.urls.push(Field::new(unescape(value), 1.0)),
            "ADR" => {
                // Seven semicolon-separated components, most of them usually
                // empty. Joined back into something readable rather than kept
                // apart, because that is how the rest of this feature holds an
                // address.
                let joined = components(value)
                    .iter()
                    .map(|part| part.trim())
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                if !joined.is_empty() {
                    card.address = Some(Field::new(joined, 1.0));
                }
            }
            "NOTE" => card.notes = Some(unescape(value)),
            _ => {}
        }
    }

    card.name = full_name.or(structured_name).map(|name| Field::new(name, 1.0));

    // A vCard with nothing usable in it is not a contact. This is the QR that
    // held a bare URL, or a marketing payload wrapped in vCard syntax.
    if card.name.is_none()
        && card.company.is_none()
        && card.phones.is_empty()
        && card.emails.is_empty()
    {
        return None;
    }
    Some(card)
}

/// Split a structured value on its *unescaped* semicolons, then unescape each.
///
/// `N`, `ORG` and `ADR` are semicolon-separated lists, and a semicolon that is
/// part of a value is written escaped. So the separator has to be found *before*
/// unescaping, not after — otherwise a company written `Smith\; Jones` is read as
/// two list items and the second one is thrown away.
///
/// Not hypothetical: the round-trip test caught exactly that on its first run,
/// turning `Smith, Jones; Partners \ Co` into `Smith, Jones`.
fn components(raw: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut escaped = false;

    for character in raw.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            // Kept, not dropped — `unescape` below is what removes it, and it
            // needs the pair intact to know what was escaped.
            current.push(character);
            escaped = true;
        } else if character == ';' {
            parts.push(unescape(&current));
            current.clear();
        } else {
            current.push(character);
        }
    }
    parts.push(unescape(&current));
    parts
}

/// Join continuation lines back onto the lines they belong to.
///
/// Must happen **before** unescaping: a fold can land in the middle of an escape
/// sequence, and unescaping first would read the backslash and the comma as
/// separate characters on separate lines.
///
/// Accepts bare newlines as well as CRLF. The spec says CRLF; a vCard that came
/// out of a QR code frequently disagrees.
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if let Some(rest) = raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t')) {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        if !raw.trim().is_empty() {
            out.push(raw.to_string());
        }
    }
    out
}

/// The inverse of [`escape`].
///
/// Both cases of the newline escape are accepted; some writers use the upper one.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(',') => out.push(','),
            Some(';') => out.push(';'),
            // An escape of something that did not need escaping: keep the
            // character and drop the backslash, which is what every importer
            // does and what the writer almost certainly meant.
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn phone_kind(params: &[String]) -> PhoneKind {
    let joined = params.join(";");
    if joined.contains("FAX") {
        PhoneKind::Fax
    } else if joined.contains("CELL") || joined.contains("MOBILE") {
        PhoneKind::Cell
    } else if joined.contains("HOME") {
        PhoneKind::Home
    } else {
        PhoneKind::Work
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_card() -> BusinessCard {
        BusinessCard {
            name: Some(Field::new("Yaseen Anwar", 0.9)),
            title: Some(Field::new("Design Engineer", 0.7)),
            company: Some(Field::new("HSI Lighting", 0.8)),
            phones: vec![PhoneField {
                raw: "+971 50 123 4567".into(),
                normalised: "+971501234567".into(),
                kind: PhoneKind::Cell,
                confidence: 0.9,
            }],
            emails: vec![Field::new("dev@hsilighting.com", 0.95)],
            urls: vec![Field::new("https://www.hsilighting.com", 0.9)],
            address: None,
            notes: None,
            raw_text: "Yaseen Anwar".into(),
        }
    }

    /// Build a vCard from lines, so no test has to spell out line endings.
    fn vcard_of(lines: &[&str]) -> String {
        let mut out = String::from("BEGIN:VCARD\r\nVERSION:3.0\r\n");
        for line in lines {
            out.push_str(line);
            out.push_str("\r\n");
        }
        out.push_str("END:VCARD\r\n");
        out
    }

    // ------------------------------------------------------------- writing --

    #[test]
    fn a_card_becomes_a_vcard() {
        let vcard = to_vcard(&a_card(), "2026-08-27T10:22:31Z");
        assert!(vcard.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"));
        assert!(vcard.ends_with("END:VCARD\r\n"));
        assert!(vcard.contains("FN:Yaseen Anwar\r\n"));
        assert!(vcard.contains("ORG:HSI Lighting\r\n"));
        assert!(vcard.contains("TEL;TYPE=CELL:+971501234567\r\n"));
        assert!(vcard.contains("EMAIL;TYPE=INTERNET:dev@hsilighting.com\r\n"));
    }

    /// The whole point of the feature, in one assertion.
    #[test]
    fn the_export_date_is_in_the_file() {
        let vcard = to_vcard(&a_card(), "2026-08-27T10:22:31Z");
        assert!(
            vcard.contains("REV:2026-08-27T10:22:31Z\r\n"),
            "the exported contact does not say when it was exported:\n{vcard}",
        );
    }

    #[test]
    fn every_line_ends_the_way_the_spec_says() {
        let vcard = to_vcard(&a_card(), "2026-08-27T10:22:31Z");
        for line in vcard.split("\r\n").filter(|l| !l.is_empty()) {
            assert!(!line.contains('\n'), "line {line:?} holds a bare newline");
            assert!(!line.contains('\r'), "line {line:?} holds a stray return");
        }
    }

    /// The failure that loses half a company name, silently.
    #[test]
    fn a_comma_in_a_company_name_survives() {
        let mut card = a_card();
        card.company = Some(Field::new("Smith, Jones and Partners", 0.8));
        let vcard = to_vcard(&card, "2026-08-27T10:22:31Z");
        assert!(
            vcard.contains(r"ORG:Smith\, Jones and Partners"),
            "the comma was not escaped, so importers keep only \"Smith\":\n{vcard}",
        );
    }

    #[test]
    fn semicolons_and_backslashes_are_escaped_too() {
        let mut card = a_card();
        card.company = Some(Field::new(r"A;B\C", 0.8));
        let vcard = to_vcard(&card, "2026-08-27T10:22:31Z");
        // The backslash escapes first, or it would escape the escapes.
        assert!(vcard.contains(r"ORG:A\;B\\C"), "got:\n{vcard}");
    }

    #[test]
    fn a_multi_line_note_becomes_one_line() {
        let mut card = a_card();
        card.notes = Some("first\nsecond".into());
        let vcard = to_vcard(&card, "2026-08-27T10:22:31Z");
        assert!(vcard.contains(r"NOTE:first\nsecond"), "got:\n{vcard}");
    }

    #[test]
    fn a_long_line_folds_at_seventy_five_octets() {
        let mut card = a_card();
        card.notes = Some("x".repeat(200));
        let vcard = to_vcard(&card, "2026-08-27T10:22:31Z");

        for line in vcard.split("\r\n").filter(|l| !l.is_empty()) {
            assert!(line.len() <= 75, "line is {} octets: {line:?}", line.len());
        }
        assert!(vcard.contains("\r\n x"), "no continuation line was written");
    }

    /// The case that makes octets rather than characters matter.
    #[test]
    fn folding_a_non_latin_line_never_splits_a_character() {
        let mut card = a_card();
        // Arabic: two octets per character, so a character count would fold at
        // roughly twice the legal length.
        let arabic = "العربية ".repeat(20);
        card.notes = Some(arabic.clone());
        let vcard = to_vcard(&card, "2026-08-27T10:22:31Z");

        for line in vcard.split("\r\n").filter(|l| !l.is_empty()) {
            assert!(line.len() <= 75, "line is {} octets: {line:?}", line.len());
        }
        // Reassembling must give back what went in — a fold that cut a character
        // in half would not round-trip.
        let unfolded: String = vcard
            .split("\r\n")
            .map(|line| line.strip_prefix(' ').unwrap_or(line))
            .collect();
        assert!(unfolded.contains(arabic.trim_end()), "folding damaged the text");
    }

    #[test]
    fn a_card_with_no_person_still_has_a_name() {
        let mut card = a_card();
        card.name = None;
        let vcard = to_vcard(&card, "2026-08-27T10:22:31Z");
        // FN is mandatory; without it some importers reject the card outright.
        assert!(vcard.contains("FN:HSI Lighting\r\n"), "got:\n{vcard}");
    }

    #[test]
    fn the_printed_number_is_used_when_it_could_not_be_normalised() {
        let mut card = a_card();
        card.phones[0].normalised = String::new();
        let vcard = to_vcard(&card, "2026-08-27T10:22:31Z");
        assert!(vcard.contains("TEL;TYPE=CELL:+971 50 123 4567\r\n"), "got:\n{vcard}");
    }

    #[test]
    fn several_cards_concatenate() {
        let vcard = to_vcards(&[a_card(), a_card()], "2026-08-27T10:22:31Z");
        assert_eq!(vcard.matches("BEGIN:VCARD").count(), 2);
        assert_eq!(vcard.matches("END:VCARD").count(), 2);
    }

    #[test]
    fn a_name_splits_into_family_and_given() {
        assert_eq!(split_name("Yaseen Anwar"), ("Anwar".into(), "Yaseen".into()));
        assert_eq!(split_name("Cher"), ("Cher".into(), "".into()));
        assert_eq!(
            split_name("Maria del Carmen Ruiz"),
            ("Ruiz".into(), "Maria del Carmen".into()),
        );
        assert_eq!(split_name(""), ("".into(), "".into()));
    }

    // ------------------------------------------------------------- reading --

    /// The property test the plan asks for: what we write, we can read.
    ///
    /// The only check that covers escaping and unescaping *together*. Either can
    /// be wrong in a way its own test does not notice — an escape that adds a
    /// backslash the reader then keeps is invisible until something makes the
    /// round trip.
    #[test]
    fn what_we_write_we_can_read_back() {
        let mut original = a_card();
        original.company = Some(Field::new(r"Smith, Jones; Partners \ Co", 0.8));
        original.notes = Some("first line\nsecond line".into());
        original.address = Some(Field::new("PO Box 1234, Dubai", 0.7));

        let text = to_vcard(&original, "2026-08-27T10:22:31Z");
        let read = from_vcard(&text).expect("our own vCard should parse");

        assert_eq!(
            original.name.as_ref().map(|f| &f.value),
            read.name.as_ref().map(|f| &f.value),
        );
        assert_eq!(
            original.company.as_ref().map(|f| &f.value),
            read.company.as_ref().map(|f| &f.value),
            "a comma, a semicolon and a backslash did not survive the round trip",
        );
        assert_eq!(
            original.title.as_ref().map(|f| &f.value),
            read.title.as_ref().map(|f| &f.value),
        );
        assert_eq!(original.notes, read.notes, "the newline did not survive");
        assert_eq!(original.emails[0].value, read.emails[0].value);
        assert_eq!(original.urls[0].value, read.urls[0].value);
        assert_eq!(original.phones[0].normalised, read.phones[0].raw);
        assert_eq!(original.phones[0].kind, read.phones[0].kind);
    }

    /// A folded line has to survive too, and it is the case the round trip
    /// nearly misses: a fold can land inside an escape sequence.
    #[test]
    fn a_folded_line_reads_back_whole() {
        let mut original = a_card();
        // Long enough to fold several times, and full of characters that escape,
        // so a fold is likely to land beside one.
        original.notes = Some("A, B; C ".repeat(20));

        let text = to_vcard(&original, "2026-08-27T10:22:31Z");
        assert!(text.contains("\r\n "), "the note did not fold, so this proves nothing");

        let read = from_vcard(&text).expect("parse");
        assert_eq!(original.notes, read.notes);
    }

    /// The shape a real QR code holds: bare newlines, mixed case, properties we
    /// do not model. None of it is worth refusing a contact over.
    #[test]
    fn a_sloppy_qr_vcard_still_reads() {
        let text = "BEGIN:VCARD\nversion:3.0\nFN:Jane Okafor\n\
                    org:Meridian Systems;Engineering\n\
                    TEL;type=CELL;VOICE:+44 7700 900123\n\
                    email;INTERNET:jane@meridian.example\n\
                    X-SOCIALPROFILE;TYPE=linkedin:jane-okafor\n\
                    END:VCARD";

        let card = from_vcard(text).expect("a sloppy vCard is still a vCard");
        assert_eq!(card.name.unwrap().value, "Jane Okafor");
        // ORG is a list; the department is not the company.
        assert_eq!(card.company.unwrap().value, "Meridian Systems");
        assert_eq!(card.phones[0].kind, PhoneKind::Cell);
        assert_eq!(card.emails[0].value, "jane@meridian.example");
        // The unknown property is not a contact field, but it is not lost.
        assert!(card.raw_text.contains("X-SOCIALPROFILE"));
    }

    /// Everything a QR gives is exact, not recognised — the whole reason the QR
    /// path exists. The review UI reads confidence to decide what to flag, so a
    /// parsed field claiming uncertainty would send somebody to check data that
    /// cannot be wrong.
    #[test]
    fn everything_from_a_qr_is_certain() {
        let text = to_vcard(&a_card(), "2026-08-27T10:22:31Z");
        let card = from_vcard(&text).expect("parse");
        assert_eq!(card.name.unwrap().confidence, 1.0);
        assert_eq!(card.company.unwrap().confidence, 1.0);
        assert_eq!(card.emails[0].confidence, 1.0);
        assert_eq!(card.phones[0].confidence, 1.0);
    }

    /// The case that decides whether the QR path runs at all.
    ///
    /// Most QR codes on business cards hold a URL, not a vCard. Returning an
    /// empty contact for those would put a blank card in front of somebody
    /// instead of falling through to OCR.
    #[test]
    fn a_qr_that_is_not_a_vcard_is_refused() {
        assert!(from_vcard("https://www.hsilighting.com").is_none());
        assert!(from_vcard("").is_none());
        assert!(from_vcard("WIFI:S:network;T:WPA;P:secret;;").is_none());
        assert!(
            from_vcard(&vcard_of(&[])).is_none(),
            "an empty vCard is not a contact",
        );
    }

    #[test]
    fn a_name_comes_from_n_when_there_is_no_fn() {
        let text = vcard_of(&["N:Okafor;Jane;;;", "TEL:+44 7700 900123"]);
        let card = from_vcard(&text).expect("parse");
        assert_eq!(card.name.unwrap().value, "Jane Okafor");
    }

    #[test]
    fn phone_types_are_read_from_their_parameters() {
        let text = vcard_of(&[
            "FN:Jane Okafor",
            "TEL;TYPE=FAX:+1 555 0100",
            "TEL;TYPE=HOME;VOICE:+1 555 0101",
            "TEL;TYPE=MOBILE:+1 555 0102",
            "TEL:+1 555 0103",
        ]);
        let card = from_vcard(&text).expect("parse");
        let kinds: Vec<PhoneKind> = card.phones.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            vec![PhoneKind::Fax, PhoneKind::Home, PhoneKind::Cell, PhoneKind::Work],
            "an untyped number defaults to WORK",
        );
    }

    /// A colon inside a value must not be read as the property separator.
    #[test]
    fn a_url_keeps_everything_after_its_scheme() {
        let text = vcard_of(&["FN:Jane Okafor", "URL:https://example.com:8443/a?b=c"]);
        let card = from_vcard(&text).expect("parse");
        assert_eq!(card.urls[0].value, "https://example.com:8443/a?b=c");
    }
}
