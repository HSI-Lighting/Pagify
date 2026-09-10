//! A font's `/ToUnicode` map: which characters each character code produces.
//!
//! # Why this is needed
//!
//! Cutting text out of a content stream means turning "the reader selected
//! characters 14 to 22" into "codes 14 to 22 of this string". Those two are the
//! same number only when every code produces exactly one character, and real
//! files break that in both directions — a ligature code produces two, and a
//! soft hyphen produces one that PDFium leaves out of its extracted text
//! altogether.
//!
//! Measured on a catalogue page: an operator drawing **44 codes for 42
//! characters**, at offset zero. Two of its codes account for no character, so
//! from that point on a character index and a code index are different numbers.
//! Assuming otherwise put the cut on the wrong glyphs and moved the line.
//!
//! This reads the table that says so. It is deliberately only the part of
//! ISO 32000-1 §9.10.3 that a `/ToUnicode` CMap uses — `bfchar` and `bfrange` —
//! and anything it does not understand comes back missing rather than guessed,
//! so the caller falls back rather than cutting at an invented offset.

use std::collections::BTreeMap;

use super::object::{Lexer, Object};

/// What each character code spells.
pub type ToUnicode = BTreeMap<u32, String>;

/// Read a `/ToUnicode` CMap.
///
/// Unmapped codes are simply absent — a caller must treat that as "unknown",
/// not as "produces nothing".
pub fn parse(bytes: &[u8]) -> ToUnicode {
    let mut out = ToUnicode::new();
    let mut lexer = Lexer::new(bytes, 0);
    // Operands accumulate before their operator, exactly as in a content
    // stream: `<0003> <0020> endbfchar` is two values and a keyword.
    let mut pending: Vec<Object> = Vec::new();

    loop {
        lexer.skip_space();
        if lexer.at >= bytes.len() {
            return out;
        }
        match bytes[lexer.at] {
            b'<' | b'[' | b'(' | b'/' | b'0'..=b'9' | b'+' | b'-' | b'.' => {
                match lexer.object() {
                    Ok(object) => pending.push(object),
                    // A malformed value ends the read rather than derailing it;
                    // what was gathered so far is still true.
                    Err(_) => return out,
                }
            }
            _ => {
                let keyword = lexer.token().to_vec();
                if keyword.is_empty() {
                    return out;
                }
                match keyword.as_slice() {
                    b"beginbfchar" => pending.clear(),
                    b"beginbfrange" => pending.clear(),
                    b"endbfchar" => {
                        // Pairs: source code, then what it spells.
                        for pair in pending.chunks(2) {
                            let [from, to] = pair else { continue };
                            if let (Some(code), Some(text)) = (code_of(from), text_of(to)) {
                                out.insert(code, text);
                            }
                        }
                        pending.clear();
                    }
                    b"endbfrange" => {
                        // Triples: low, high, and either a starting value or a
                        // list with one entry per code in the range.
                        for triple in pending.chunks(3) {
                            let [low, high, target] = triple else { continue };
                            let (Some(low), Some(high)) = (code_of(low), code_of(high)) else {
                                continue;
                            };
                            // A range that runs backwards, or one long enough to
                            // be a mistake, is skipped rather than expanded.
                            if high < low || high - low > 0xFFFF {
                                continue;
                            }
                            match target {
                                Object::Array(items) => {
                                    for (offset, item) in items.iter().enumerate() {
                                        let Some(text) = text_of(item) else { continue };
                                        out.insert(low + offset as u32, text);
                                    }
                                }
                                other => {
                                    let Some(start) = text_of(other) else { continue };
                                    // The *last* unit counts up across the
                                    // range; everything before it is a prefix
                                    // shared by every code in it.
                                    let mut units: Vec<u16> = start.encode_utf16().collect();
                                    let Some(last) = units.pop() else { continue };
                                    for step in 0..=(high - low) {
                                        let Some(unit) = last.checked_add(step as u16) else {
                                            break;
                                        };
                                        let mut all = units.clone();
                                        all.push(unit);
                                        if let Ok(text) = String::from_utf16(&all) {
                                            out.insert(low + step, text);
                                        }
                                    }
                                }
                            }
                        }
                        pending.clear();
                    }
                    // `endcodespacerange` and the rest carry values this does
                    // not need; dropping them keeps the next section clean.
                    _ => pending.clear(),
                }
            }
        }
    }
}

/// A source code, as a hex string of one or two bytes.
fn code_of(object: &Object) -> Option<u32> {
    let Object::HexString(raw) = object else { return None };
    let digits: Vec<u8> = raw.iter().copied().filter(u8::is_ascii_hexdigit).collect();
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    let text = String::from_utf8_lossy(&digits).into_owned();
    u32::from_str_radix(&text, 16).ok()
}

/// What a destination spells, as UTF-16BE.
fn text_of(object: &Object) -> Option<String> {
    let Object::HexString(raw) = object else { return None };
    let digits: Vec<u8> = raw.iter().copied().filter(u8::is_ascii_hexdigit).collect();
    let units: Vec<u16> = digits
        .chunks(4)
        .filter(|chunk| chunk.len() == 4)
        .filter_map(|chunk| {
            u16::from_str_radix(&String::from_utf8_lossy(chunk), 16).ok()
        })
        .collect();
    // Lone surrogates and other nonsense come back as `None` rather than as a
    // replacement character, which would count as a real character and put the
    // alignment out by one.
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bfchar_section_maps_each_code_it_names() {
        let map = parse(
            b"2 beginbfchar\n<0003> <0020>\n<0024> <0041>\nendbfchar\n",
        );
        assert_eq!(map.get(&0x0003).map(String::as_str), Some(" "));
        assert_eq!(map.get(&0x0024).map(String::as_str), Some("A"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn a_bfrange_counts_up_from_its_start() {
        let map = parse(b"1 beginbfrange\n<0024> <0026> <0041>\nendbfrange\n");
        assert_eq!(map.get(&0x0024).map(String::as_str), Some("A"));
        assert_eq!(map.get(&0x0025).map(String::as_str), Some("B"));
        assert_eq!(map.get(&0x0026).map(String::as_str), Some("C"));
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn a_bfrange_with_a_list_takes_one_entry_per_code() {
        let map = parse(b"1 beginbfrange\n<0028> <002A> [<0058> <0059> <005A>]\nendbfrange\n");
        assert_eq!(map.get(&0x0028).map(String::as_str), Some("X"));
        assert_eq!(map.get(&0x0029).map(String::as_str), Some("Y"));
        assert_eq!(map.get(&0x002A).map(String::as_str), Some("Z"));
    }

    /// **The case this module exists for.** One code, two characters — so a
    /// character index and a code index stop being the same number.
    #[test]
    fn a_ligature_maps_one_code_to_several_characters() {
        let map = parse(b"1 beginbfchar\n<00FB> <00660069>\nendbfchar\n");
        assert_eq!(map.get(&0x00FB).map(String::as_str), Some("fi"));
    }

    /// And the other direction: a soft hyphen, which PDFium leaves out of the
    /// text it extracts, so the code accounts for no character at all.
    #[test]
    fn a_soft_hyphen_is_read_as_the_character_it_is() {
        let map = parse(b"1 beginbfchar\n<0011> <00AD>\nendbfchar\n");
        assert_eq!(map.get(&0x0011).map(String::as_str), Some("\u{ad}"));
    }

    #[test]
    fn both_kinds_of_section_can_appear_in_one_map() {
        let map = parse(
            b"/CIDInit /ProcSet findresource begin\n\
              1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n\
              1 beginbfchar\n<0003> <0020>\nendbfchar\n\
              1 beginbfrange\n<0024> <0025> <0041>\nendbfrange\n\
              endcmap\n",
        );
        assert_eq!(map.get(&0x0003).map(String::as_str), Some(" "));
        assert_eq!(map.get(&0x0024).map(String::as_str), Some("A"));
        assert_eq!(map.get(&0x0025).map(String::as_str), Some("B"));
    }

    /// A code space range must not be read as a mapping — it says how wide a
    /// code is, not what it spells.
    #[test]
    fn a_codespace_range_maps_nothing() {
        let map = parse(b"1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
        assert!(map.is_empty());
    }

    #[test]
    fn nonsense_stops_the_read_rather_than_derailing_it() {
        // What came before the damage is still true.
        let map = parse(b"1 beginbfchar\n<0003> <0020>\nendbfchar\n<unclosed");
        assert_eq!(map.get(&0x0003).map(String::as_str), Some(" "));
    }

    #[test]
    fn an_empty_map_is_not_an_error() {
        assert!(parse(b"").is_empty());
        assert!(parse(b"begincmap endcmap").is_empty());
    }
}
