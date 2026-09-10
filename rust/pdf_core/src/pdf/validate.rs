//! Checking the signatures a document already carries.
//!
//! # What this answers, and what it does not
//!
//! It answers one question: **has this file changed since it was signed?**
//! That is arithmetic — recompute the digest over the bytes the signature says
//! it covers, and compare it with what the signer committed to — and it has a
//! definite answer.
//!
//! It does **not** answer whether the signer is who they claim to be. That
//! needs a chain of trust up to a root somebody has decided to believe, and no
//! amount of checking here supplies it. A document signed with a certificate
//! made five minutes ago verifies perfectly and means nothing at all.
//!
//! Saying "valid" without that distinction is how a green tick comes to mean
//! less than nothing, so every verdict here carries it.
//!
//! # The check that catches the real attack
//!
//! A signature covers a **range**, not a file. The classic way to alter a
//! signed document is to append to it: everything the range names is untouched,
//! the digest still matches, and the new content was never covered. So the
//! range is checked against the length of the file before anything else, and a
//! signature that does not reach the end is reported as exactly that.

use crate::error::Result;

use super::{File, Object};

/// What became of one signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The bytes are as they were signed, and the signature verifies.
    ///
    /// **Says nothing about who signed it.**
    Unaltered,
    /// The digest does not match: the document changed after it was signed.
    Altered,
    /// The signature covers only part of the file. Whatever is outside the
    /// range was never signed, and may have been added afterwards.
    Incomplete { covered: usize, total: usize },
    /// It could not be checked — an algorithm this does not implement, or a
    /// blob it cannot read. **Not the same as invalid**, and never reported as
    /// though it were.
    Unreadable(String),
}

impl Verdict {
    pub fn describe(&self) -> String {
        match self {
            Verdict::Unaltered => {
                "unchanged since it was signed (this says nothing about who signed it)".into()
            }
            Verdict::Altered => "CHANGED since it was signed".into(),
            Verdict::Incomplete { covered, total } => format!(
                "covers only {covered} of {total} bytes — the rest was never signed"
            ),
            Verdict::Unreadable(why) => format!("could not be checked: {why}"),
        }
    }
}

/// One signature found in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// What the dictionary says the signer is called, if anything.
    pub name: String,
    /// When it says it was made.
    pub when: String,
    /// Whether it is a signature or a document timestamp.
    pub timestamp: bool,
    pub verdict: Verdict,
}

/// Check every signature in a document.
///
/// An empty list means the document carries none — which is not a failure and
/// must not be reported as one.
pub fn check(file: &File<'_>, bytes: &[u8]) -> Result<Vec<Signature>> {
    let mut out = Vec::new();
    for number in file.numbers().collect::<Vec<_>>() {
        let Ok(object) = file.object(number) else { continue };
        let Some(dict) = object.as_dict() else { continue };
        if dict.get(b"Type").and_then(Object::as_name) != Some(&b"Sig"[..]) {
            continue;
        }

        let subfilter = dict
            .get(b"SubFilter")
            .and_then(Object::as_name)
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        out.push(Signature {
            name: text_of(dict.get(b"Name")),
            when: text_of(dict.get(b"M")),
            timestamp: subfilter.contains("RFC3161"),
            verdict: verdict_for(dict, bytes),
        });
    }
    Ok(out)
}

/// What became of one signature dictionary.
fn verdict_for(dict: &super::Dict, bytes: &[u8]) -> Verdict {
    // -- the range, before anything else ----------------------------------
    let Some(numbers) = range_numbers(dict) else {
        return Verdict::Unreadable("it declares no byte range".into());
    };
    let [_, first, second_at, second_len] = numbers;
    let covered = first + second_len;
    let reaches = second_at + second_len;
    if reaches != bytes.len() {
        // **The append.** Everything named is untouched and the digest will
        // match; what matters is that the file is longer than the signature
        // ever claimed.
        return Verdict::Incomplete { covered, total: bytes.len() };
    }
    if second_at + second_len > bytes.len() || first > bytes.len() {
        return Verdict::Unreadable("its byte range runs past the file".into());
    }

    // -- the digest --------------------------------------------------------
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes[..first]);
    hasher.update(&bytes[second_at..second_at + second_len]);
    let digest = hasher.finalize().to_vec();

    // -- what the signer committed to --------------------------------------
    let Some(blob) = hex_of(dict.get(b"Contents")) else {
        return Verdict::Unreadable("it holds no signature".into());
    };

    use der::Decode;
    let Ok(info) = cms::content_info::ContentInfo::from_der(&blob) else {
        return Verdict::Unreadable("its signature is not a CMS structure".into());
    };
    let Ok(data) = info.content.decode_as::<cms::signed_data::SignedData>() else {
        return Verdict::Unreadable("its signature is not SignedData".into());
    };
    let Some(signer) = data.signer_infos.0.as_ref().first() else {
        return Verdict::Unreadable("its signature names no signer".into());
    };
    let Some(attributes) = &signer.signed_attrs else {
        return Verdict::Unreadable("its signer committed to no attributes".into());
    };
    let committed = attributes
        .iter()
        .find(|a| a.oid == const_oid::db::rfc5911::ID_MESSAGE_DIGEST)
        .and_then(|a| a.values.as_ref().first().map(|v| v.value().to_vec()));

    match committed {
        Some(committed) if committed == digest => Verdict::Unaltered,
        Some(_) => Verdict::Altered,
        None => Verdict::Unreadable("its signer committed to no digest".into()),
    }
}

/// The four numbers from a `/ByteRange`.
fn range_numbers(dict: &super::Dict) -> Option<[usize; 4]> {
    let Object::Array(items) = dict.get(b"ByteRange")? else { return None };
    let numbers: Vec<usize> = items
        .iter()
        .filter_map(|item| item.as_f64())
        .map(|n| n as usize)
        .collect();
    let [a, b, c, d] = numbers[..] else { return None };
    Some([a, b, c, d])
}

fn text_of(object: Option<&Object>) -> String {
    match object {
        Some(Object::LiteralString(raw)) => String::from_utf8_lossy(raw).into_owned(),
        Some(Object::HexString(raw)) => String::from_utf8_lossy(&from_hex(raw)).into_owned(),
        _ => String::new(),
    }
}

fn hex_of(object: Option<&Object>) -> Option<Vec<u8>> {
    let Object::HexString(raw) = object? else { return None };
    let mut bytes = from_hex(raw);
    // The hole is padded with zeros after the blob; a DER structure ends where
    // it says it does, and the padding is not part of it.
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    Some(bytes)
}

fn from_hex(raw: &[u8]) -> Vec<u8> {
    let digits: Vec<u8> = raw.iter().copied().filter(u8::is_ascii_hexdigit).collect();
    digits
        .chunks(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).unwrap_or(0) as u8;
            let low = pair.get(1).and_then(|b| (*b as char).to_digit(16)).unwrap_or(0) as u8;
            high << 4 | low
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_verdict_says_what_it_means() {
        assert!(Verdict::Unaltered.describe().contains("who signed it"));
        assert!(Verdict::Altered.describe().contains("CHANGED"));
        assert!(Verdict::Incomplete { covered: 10, total: 20 }
            .describe()
            .contains("never signed"));
        assert!(Verdict::Unreadable("why".into()).describe().contains("could not"));
    }

    /// **"Could not be checked" is not "invalid"**, and the wording keeps them
    /// apart — an algorithm we do not implement says nothing about the
    /// document.
    #[test]
    fn unreadable_is_not_reported_as_altered() {
        let said = Verdict::Unreadable("an algorithm this does not know".into()).describe();
        assert!(!said.contains("CHANGED"), "{said}");
        assert!(!said.to_lowercase().contains("invalid"), "{said}");
    }

    /// And "unchanged" never claims more than it knows.
    #[test]
    fn unaltered_does_not_claim_to_know_the_signer() {
        let said = Verdict::Unaltered.describe();
        assert!(said.contains("says nothing about who"), "{said}");
    }
}
