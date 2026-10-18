//! The form's calculation order, as the file on disk stores it — the Form
//! tab's Calculation Order.
//!
//! **What the calculation order is.** A PDF form can say, in `/AcroForm`'s
//! `/CO` array, the order in which fields' calculated values are recomputed —
//! a total after the line items it adds up, rather than before. Each entry is a
//! reference to a field dictionary, and the array's order is the order. A form
//! without `/CO` leaves the order open: a reader may calculate in whatever
//! order it likes.
//!
//! **Read from the file, not the open document.** PDFium fills fields but has
//! no calculation-order API, and the open document's in-memory copy cannot be
//! asked for raw dictionary entries at all, so the honest source is the bytes
//! this document was opened from. The answer says so, and says that an unsaved
//! edit is not in it.
//!
//! **Read-only.** Reordering the calculations would mean writing `/CO` and the
//! field tree — form authoring, which is out of scope for this build — so this
//! reports and nothing here changes the file.

use std::collections::BTreeSet;

use pdf_core::pdf::{File, Object};
use pdf_core::{PdfError, Result};

use crate::Session;

/// Report the fields' calculation order, numbered, or say there is none.
pub fn report(session: &Session) -> Result<Vec<String>> {
    let bytes = std::fs::read(session.path())?;
    report_bytes(&bytes)
}

/// [`report`] over a file's bytes — the whole of the work, and what the tests
/// use without opening a document.
pub fn report_bytes(bytes: &[u8]) -> Result<Vec<String>> {
    let file = File::parse(bytes)?;

    // A field's `/T` name is a **string**, and strings are what encryption
    // encrypts. This reader does not decrypt, so on an encrypted file it would
    // print ciphertext as a field name — worse than saying it cannot say.
    if file.trailer().get(b"Encrypt").is_some() {
        return Ok(answer(Some(
            "no calculation order this build can read — the file is encrypted, and field \
             names are encrypted strings this reader does not decrypt."
                .into()),
            Vec::new(),
        ));
    }

    let root = file
        .trailer()
        .get(b"Root")
        .and_then(Object::as_reference)
        .map(|(number, _)| number)
        .ok_or_else(|| PdfError::InvalidArgument("the trailer names no document catalogue".into()))?;
    let catalog = file.object(root)?;
    let Some(catalog) = catalog.as_dict() else {
        return Err(PdfError::InvalidArgument("the document catalogue is not a dictionary".into()));
    };

    let Some(acroform) = catalog.get(b"AcroForm") else {
        return Ok(answer(Some(
            "no calculation order — this document has no form: /AcroForm is absent, so \
             there are no fields to calculate."
                .into()),
            Vec::new(),
        ));
    };
    let acroform = file.resolve(acroform)?;
    let Some(acroform) = acroform.as_dict() else {
        return Ok(answer(Some(
            "no calculation order — the /AcroForm entry is not a dictionary, so it has no \
             field calculation order to read."
                .into()),
            Vec::new(),
        ));
    };

    let Some(co) = acroform.get(b"CO") else {
        return Ok(answer(Some(
            "no calculation order — /AcroForm has no /CO entry, so a reader is free to \
             calculate the fields in any order."
                .into()),
            Vec::new(),
        ));
    };
    let co = file.resolve(co)?;
    let Object::Array(entries) = co else {
        return Ok(answer(Some(
            "no calculation order — /AcroForm's /CO is not an array, so it names no order."
                .into()),
            Vec::new(),
        ));
    };
    if entries.is_empty() {
        return Ok(answer(Some(
            "no calculation order — /AcroForm's /CO array is empty, so a reader is free to \
             calculate the fields in any order."
                .into()),
            Vec::new(),
        ));
    }

    let mut lines = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        lines.push(format!("{}. {}", index + 1, field_name(&file, entry)?));
    }
    Ok(answer(None, lines))
}

/// The whole answer: the numbered fields, or the sentence saying there are
/// none, with the shared provenance/scope sentence.
fn answer(none: Option<String>, found: Vec<String>) -> Vec<String> {
    let scope = "Read from the file on disk, so an unsaved edit is not in it. Reordering the \
                 calculations is form authoring and is not built.";
    match none {
        Some(sentence) => vec![format!("{sentence} {scope}")],
        None => {
            let mut lines = found;
            let count = lines.len();
            lines.push(format!("{count} field(s) in calculation order. {scope}"));
            lines
        }
    }
}

/// The fully qualified name of the field a `/CO` entry names — the field's own
/// `/T` with its ancestors' partial names, dot-separated, the way a PDF form
/// spells a nested field.
fn field_name(file: &File<'_>, entry: &Object) -> Result<String> {
    let (number, mut dict) = match entry {
        Object::Reference(number, _) => {
            let object = file.object(*number)?;
            let Some(dict) = object.as_dict() else {
                return Ok(format!("(field {number} 0 R is not a dictionary)"));
            };
            (Some(*number), dict.clone())
        }
        Object::Dict(dict) | Object::Stream(dict, _) => (None, dict.clone()),
        _ => return Ok("(an entry that is not a field reference)".into()),
    };

    let mut parts: Vec<String> = Vec::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for _ in 0..MAX_PARENT_DEPTH {
        if let Some(own) = dict.get(b"T").and_then(text_string) {
            if !own.trim().is_empty() {
                parts.push(own.trim().to_string());
            }
        }
        let Some(parent) = dict.get(b"Parent") else { break };
        let parent = match parent {
            Object::Reference(number, _) => {
                if !seen.insert(*number) {
                    break;
                }
                file.object(*number)?
            }
            other => other.clone(),
        };
        let Some(parent) = parent.as_dict() else { break };
        dict = parent.clone();
    }

    if parts.is_empty() {
        return Ok(match number {
            Some(number) => format!("(a field with no /T name, {number} 0 R)"),
            None => "(a field with no /T name)".into(),
        });
    }
    parts.reverse();
    Ok(parts.join("."))
}

/// Depth cap: deeper than a real form's field hierarchy, and a guard against a
/// `/Parent` chain that points in a circle.
const MAX_PARENT_DEPTH: usize = 32;

/// A PDF text string — a literal string with its escapes decoded, or a hex
/// string, either of which may be UTF-16BE with a byte-order mark.
fn text_string(object: &Object) -> Option<String> {
    match object {
        Object::LiteralString(raw) => Some(decode(&unescape_literal(raw))),
        Object::HexString(raw) => Some(decode(&hex_bytes(raw)?)),
        _ => None,
    }
}

fn decode(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        // PDF 2.0 allows a UTF-8 string with a byte-order mark.
        String::from_utf8_lossy(&bytes[3..]).into_owned()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// The bytes a literal PDF string stands for: `\n`, `\(` and the octal escapes
/// the format uses, rather than the escaped text the file holds.
fn unescape_literal(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0;
    while at < raw.len() {
        if raw[at] != b'\\' || at + 1 >= raw.len() {
            out.push(raw[at]);
            at += 1;
            continue;
        }
        at += 1;
        match raw[at] {
            b'n' => {
                out.push(b'\n');
                at += 1;
            }
            b'r' => {
                out.push(b'\r');
                at += 1;
            }
            b't' => {
                out.push(b'\t');
                at += 1;
            }
            b'b' => {
                out.push(0x08);
                at += 1;
            }
            b'f' => {
                out.push(0x0C);
                at += 1;
            }
            b'(' | b')' | b'\\' => {
                out.push(raw[at]);
                at += 1;
            }
            // A backslash before a newline is a line continuation: neither
            // character is part of the string.
            b'\n' => at += 1,
            b'\r' => {
                at += 1;
                if at < raw.len() && raw[at] == b'\n' {
                    at += 1;
                }
            }
            digit if (b'0'..=b'7').contains(&digit) => {
                let mut value: u32 = 0;
                let mut taken = 0;
                while taken < 3 && at < raw.len() && (b'0'..=b'7').contains(&raw[at]) {
                    value = value * 8 + u32::from(raw[at] - b'0');
                    at += 1;
                    taken += 1;
                }
                out.push((value & 0xFF) as u8);
            }
            other => {
                // A backslash before anything else stands for that character.
                out.push(other);
                at += 1;
            }
        }
    }
    out
}

fn hex_bytes(raw: &[u8]) -> Option<Vec<u8>> {
    let digits: Vec<u8> = raw
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if !digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut out = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks(2) {
        let high = (pair[0] as char).to_digit(16)?;
        // An odd final digit is a whole byte on its own, per the format.
        let low = match pair.get(1) {
            Some(low) => (*low as char).to_digit(16)?,
            None => 0,
        };
        out.push(((high << 4) | low) as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small, valid PDF with the objects given, written with a plain xref
    /// table — the shape `pdf_core::pdf::File` reads.
    fn pdf(objects: &[&str]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    /// A form whose calculation order is subtotal then total — deliberately
    /// not the order the fields appear in `/Fields`, so the order read back can
    /// only have come from `/CO`.
    fn calculated_form() -> Vec<u8> {
        pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>",
            "<< /FT /Tx /T (total) /Rect [10 10 100 30] >>",
            "<< /Fields [4 0 R 6 0 R] /CO [6 0 R 4 0 R] >>",
            "<< /FT /Tx /T (subtotal) /Parent 7 0 R >>",
            "<< /T (invoice) /Kids [6 0 R] >>",
        ])
    }

    /// The plan's answer: the fields, numbered in `/CO`'s order, with a nested
    /// field's name qualified the way a PDF form spells it.
    #[test]
    fn the_calculation_order_reads_in_the_order_stored() {
        let lines = report_bytes(&calculated_form()).expect("read");
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[0], "1. invoice.subtotal");
        assert_eq!(lines[1], "2. total");
        assert!(lines[2].contains("2 field(s) in calculation order"), "{lines:?}");
        assert!(lines[2].contains("Read from the file on disk"), "{lines:?}");
        assert!(lines[2].contains("not built"), "{lines:?}");
    }

    /// A form without `/CO` says there is no order rather than inventing one.
    #[test]
    fn a_form_without_a_co_says_there_is_none() {
        let bytes = pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>",
            "<< /Fields [] >>",
        ]);
        let lines = report_bytes(&bytes).expect("read");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("no calculation order"), "{lines:?}");
        assert!(lines[0].contains("no /CO entry"), "{lines:?}");
    }

    /// A document with no form at all is a different fact from a form with no
    /// order, and the answer names which it is.
    #[test]
    fn a_document_without_a_form_says_which_absence_it_is() {
        let bytes = pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>",
        ]);
        let lines = report_bytes(&bytes).expect("read");
        assert!(lines[0].contains("no calculation order"), "{lines:?}");
        assert!(lines[0].contains("/AcroForm is absent"), "{lines:?}");
        assert!(!lines[0].contains("no /CO entry"), "{lines:?}");
    }

    /// An entry that names no field is still a line in the order — dropping it
    /// would silently shorten the calculation order the file declares.
    #[test]
    fn an_unnamed_field_is_reported_where_it_is_in_the_order() {
        let bytes = pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>",
            "<< /Fields [5 0 R] /CO [5 0 R 6 0 R] >>",
            "<< /FT /Tx /Rect [10 10 40 20] >>",
            "<< /FT /Tx /T (named) >>",
        ]);
        let lines = report_bytes(&bytes).expect("read");
        assert_eq!(lines[0], "1. (a field with no /T name, 5 0 R)");
        assert_eq!(lines[1], "2. named");
    }

    /// A name written as a hex string with a UTF-16 byte-order mark decodes to
    /// the same words a reader shows — a name is not its bytes.
    #[test]
    fn a_utf16_name_decodes() {
        let bytes = pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>",
            "<< /Fields [5 0 R] /CO [5 0 R] >>",
            "<< /FT /Tx /T <FEFF0054006F00740061006C> >>",
        ]);
        let lines = report_bytes(&bytes).expect("read");
        assert_eq!(lines[0], "1. Total");
    }

    /// An encrypted file's field names are ciphertext to this reader, so the
    /// answer refuses to name them rather than printing garbage.
    #[test]
    fn an_encrypted_file_is_not_guessed_at() {
        let bytes = calculated_form();
        // The trailer is after the cross-reference table, so inserting the
        // encryption reference there leaves every object's offset — and
        // `startxref` — still correct.
        let at = bytes
            .windows(b"/Root 1 0 R".len())
            .position(|window| window == b"/Root 1 0 R")
            .expect("the calculated form's trailer");
        let mut patched = bytes[..at].to_vec();
        patched.extend_from_slice(b"/Root 1 0 R /Encrypt 9 0 R");
        patched.extend_from_slice(&bytes[at + b"/Root 1 0 R".len()..]);
        let lines = report_bytes(&patched).expect("read");
        assert!(lines[0].contains("encrypted"), "{lines:?}");
        assert!(!lines[0].contains("invoice.subtotal"), "{lines:?}");
    }

    /// A literal name's escapes are decoded, not shown as escape text.
    #[test]
    fn a_literal_name_unescapes_its_punctuation() {
        assert_eq!(unescape_literal(b"Line \\(1\\)\\101"), b"Line (1)A".to_vec());
        assert_eq!(unescape_literal(b"two\\\nlines"), b"twolines".to_vec());
    }
}
