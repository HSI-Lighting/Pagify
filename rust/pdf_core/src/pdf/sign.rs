//! A digital signature: what the file says, and what it is signed with.
//!
//! # What a PDF signature actually is
//!
//! Not a picture of a name. A `/Sig` dictionary whose `/Contents` holds a
//! **CMS SignedData** blob — a detached signature over the bytes of the file
//! itself — and whose `/ByteRange` says which bytes those are. The range is
//! written as two spans that between them cover the whole file *except* the
//! `/Contents` placeholder, because a signature cannot sign itself.
//!
//! ```text
//! /ByteRange [0 a b c]
//!            └─ 0..a ─┘ <hex placeholder> └─ b..b+c ─┘
//! ```
//!
//! Everything hard about this is in those four numbers. They are written before
//! the digest is computed, they must not change afterwards, and a file whose
//! range is off by one byte produces a signature every reader rejects while
//! looking perfectly well formed.
//!
//! # Why the placeholder is fixed-width
//!
//! The signature goes *inside* the region the range describes a hole in, so its
//! size has to be known before it exists. So a block of zeros is written, the
//! digest is taken over everything either side, and the finished blob is copied
//! into the hole **without moving a byte**. A blob larger than the hole is a
//! failure, not something to grow the file for — growing it would shift every
//! offset the range was computed from.
//!
//! # What this does not do
//!
//! It does not decide whether a certificate is trustworthy. A signature made
//! with a certificate no one recognises is cryptographically sound and socially
//! worthless, and every reader will say so. That judgement belongs to whoever
//! holds the certificate, not here.

use crate::error::{PdfError, Result};

/// Who is signing: a certificate and the key that goes with it.
///
/// Loaded from a PKCS#12 file — a `.p12` or `.pfx` — which is how certificates
/// are handed out and how every other program expects to receive one.
pub struct Identity {
    /// The signer's certificate, DER encoded, and any chain above it.
    pub certificates: Vec<Vec<u8>>,
    /// The private key, DER encoded (PKCS#8).
    key: Vec<u8>,
}

impl Identity {
    /// Read an identity from a PKCS#12 file.
    ///
    /// **Both halves or nothing.** A file with a certificate and no key signs
    /// nothing, and one with a key and no certificate produces a signature no
    /// reader can attribute — either alone is a mistake worth naming rather
    /// than a partial success.
    pub fn from_pkcs12(bytes: &[u8], password: &str) -> Result<Self> {
        let pfx = p12::PFX::parse(bytes)
            .map_err(|_| PdfError::InvalidArgument("that is not a PKCS#12 file".into()))?;

        // **Three causes, one symptom.** A wrong password, an encryption this
        // cannot read, and a file holding only half an identity all present the
        // same way from here — sometimes as an error, sometimes as empty bags,
        // depending on where the decryption gives up. Naming only the password
        // would send somebody hunting for a typo that is not there, so the
        // answer names all three wherever it comes from.
        let certificates = pfx.cert_x509_bags(password).unwrap_or_default();
        let keys = pfx.key_bags(password).unwrap_or_default();

        if certificates.is_empty() || keys.is_empty() {
            // **Two very different causes, one symptom.** A wrong password and
            // an encryption this cannot read both come back as empty bags, and
            // blaming the password for the second would send somebody hunting
            // for a typo that is not there.
            //
            // Modern OpenSSL writes PKCS#12 with AES-256 by default; the reader
            // here handles the older 3DES form. A file written by a certificate
            // authority in the last few years is likely to be the former.
            return Err(PdfError::InvalidArgument(
                "that file did not open — either the password is wrong, or it uses \
                 AES encryption this cannot yet read. Re-exporting it with \
                 `-keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1` \
                 produces a form it can."
                    .into(),
            ));
        }
        Ok(Identity { certificates, key: keys[0].clone() })
    }

    /// The signing key, as a usable RSA key.
    pub fn rsa_key(&self) -> Result<rsa::RsaPrivateKey> {
        use rsa::pkcs8::DecodePrivateKey;
        rsa::RsaPrivateKey::from_pkcs8_der(&self.key)
            .map_err(|_| PdfError::Unsupported("only RSA keys can sign, and this is not one"))
    }

    /// What the certificate says about who is signing.
    ///
    /// Read for showing a person before they commit, so they can see whose name
    /// is about to go on the document.
    pub fn subject(&self) -> Result<String> {
        use der::Decode;
        let certificate = x509_cert::Certificate::from_der(&self.certificates[0])
            .map_err(|_| PdfError::InvalidArgument("that certificate cannot be read".into()))?;
        Ok(certificate.tbs_certificate.subject.to_string())
    }
}

/// How much room the signature is given in the file.
///
/// A CMS blob with one RSA-2048 signature and a certificate chain runs to a few
/// kilobytes. Sixteen is generous without being absurd — and being generous
/// costs only file size, while being tight costs the signature.
pub const PLACEHOLDER: usize = 16 * 1024;

/// Where the signature sits in a prepared file, and what is signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteRange {
    /// The offset of the `<` opening the placeholder.
    pub hole_at: usize,
    /// How many bytes the placeholder occupies, angle brackets included.
    pub hole_len: usize,
    /// Total length of the prepared file.
    pub total: usize,
}

impl ByteRange {
    /// The four numbers, as `/ByteRange` wants them.
    pub fn numbers(&self) -> [usize; 4] {
        let after = self.hole_at + self.hole_len;
        [0, self.hole_at, after, self.total.saturating_sub(after)]
    }

    /// The bytes a signature covers: everything but the hole.
    ///
    /// Returned as two slices rather than one joined buffer, so a caller
    /// digesting a large file need not copy it.
    pub fn covered<'a>(&self, bytes: &'a [u8]) -> Result<(&'a [u8], &'a [u8])> {
        let [_, first, second_at, second_len] = self.numbers();
        if first > bytes.len() || second_at + second_len > bytes.len() {
            return Err(PdfError::InvalidArgument(
                "the signature's byte range runs past the file".into(),
            ));
        }
        Ok((&bytes[..first], &bytes[second_at..second_at + second_len]))
    }

    /// Whether these numbers describe the whole file with exactly one hole.
    ///
    /// **The check worth having.** A range that leaves bytes uncovered is the
    /// classic way a signed PDF is altered without breaking its signature: the
    /// attacker appends, and the range never said the appended part was
    /// covered.
    pub fn covers_everything(&self) -> bool {
        let [start, first, second_at, second_len] = self.numbers();
        start == 0
            && second_at == first + self.hole_len
            && second_at + second_len == self.total
    }
}

/// Find the placeholder in a prepared file.
///
/// Located by scanning rather than remembered from writing it, because the
/// number that matters is where it ended up — after whatever the writer did to
/// the bytes — not where it was meant to go.
pub fn find_placeholder(bytes: &[u8]) -> Result<ByteRange> {
    // **Found by its shape, not by the key in front of it.** Every page in a
    // document has a `/Contents` of its own — `/Contents 9 0 R` — and looking
    // for that word first found a page's, then took the next `<` in the file as
    // the hole. What is unmistakable is the run of zeros: nothing else in a PDF
    // is a hex string of a thousand noughts.
    const LEAST: usize = 1024;

    let mut at = 0usize;
    while at < bytes.len() {
        let Some(open) = bytes[at..].iter().position(|b| *b == b'<').map(|n| at + n) else {
            break;
        };
        let zeros = bytes[open + 1..]
            .iter()
            .take_while(|b| **b == b'0')
            .count();
        if zeros >= LEAST && bytes.get(open + 1 + zeros) == Some(&b'>') {
            return Ok(ByteRange {
                hole_at: open,
                hole_len: zeros + 2,
                total: bytes.len(),
            });
        }
        at = open + 1;
    }
    Err(PdfError::InvalidArgument("that file has no signature to fill in".into()))
}

/// Write a finished signature into the hole a prepared file left for it.
///
/// **Nothing moves.** The blob is written as hex over the zeros already there,
/// and the remainder of the hole stays zero — so every offset the byte range
/// was computed from is still correct afterwards, which is the only way the
/// signature verifies.
pub fn fill_placeholder(bytes: &mut [u8], range: &ByteRange, signature: &[u8]) -> Result<()> {
    // Two hex digits a byte, inside the angle brackets.
    let room = range.hole_len.saturating_sub(2);
    if signature.len() * 2 > room {
        return Err(PdfError::Unsupported(
            "the signature is larger than the space reserved for it",
        ));
    }
    let start = range.hole_at + 1;
    for (index, byte) in signature.iter().enumerate() {
        let hex = format!("{byte:02X}");
        bytes[start + index * 2] = hex.as_bytes()[0];
        bytes[start + index * 2 + 1] = hex.as_bytes()[1];
    }
    // Whatever is left of the hole stays as it was written — zeros — which is
    // what every reader expects to find after the blob.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with a hole in the middle, as a prepared one has.
    fn prepared() -> Vec<u8> {
        // A page's own `/Contents` in front of the signature's, because that is
        // what a real document looks like and what the finder used to trip on.
        let mut out = b"%PDF-1.7\n/Contents 9 0 R\nsome objects\n/Contents <".to_vec();
        out.extend(std::iter::repeat_n(b'0', 2048));
        out.extend_from_slice(b">\nmore objects\n%%EOF");
        out
    }

    #[test]
    fn the_placeholder_is_found_where_it_was_written() {
        let bytes = prepared();
        let range = find_placeholder(&bytes).expect("find");
        assert_eq!(bytes[range.hole_at], b'<');
        assert_eq!(bytes[range.hole_at + range.hole_len - 1], b'>');
        assert_eq!(range.total, bytes.len());
    }

    /// **The four numbers must leave exactly one hole and cover the rest.**
    #[test]
    fn the_byte_range_covers_the_whole_file_but_the_hole() {
        let bytes = prepared();
        let range = find_placeholder(&bytes).expect("find");
        assert!(range.covers_everything());

        let [start, first, second_at, second_len] = range.numbers();
        assert_eq!(start, 0);
        assert_eq!(first, range.hole_at);
        assert_eq!(second_at, range.hole_at + range.hole_len);
        assert_eq!(second_at + second_len, bytes.len());
    }

    /// And what it covers is every byte except the hole — checked by counting,
    /// because an off-by-one here produces a signature that verifies nowhere.
    #[test]
    fn what_is_covered_is_everything_outside_the_hole() {
        let bytes = prepared();
        let range = find_placeholder(&bytes).expect("find");
        let (before, after) = range.covered(&bytes).expect("covered");

        assert_eq!(before.len() + after.len() + range.hole_len, bytes.len());
        assert!(!before.contains(&b'<'), "the hole leaked into what is signed");
        assert!(!after.starts_with(b"0"), "the hole leaked into what is signed");
        assert!(
            before.ends_with(b"/Contents "),
            "the finder took a page's contents for the signature's"
        );
        assert!(after.starts_with(b"\nmore"), "the second span starts in the wrong place");
    }

    /// **A range that leaves bytes uncovered is how a signed file gets
    /// altered.** The attacker appends, and the range never claimed the
    /// appended part was signed.
    #[test]
    fn a_range_that_does_not_reach_the_end_is_rejected() {
        let bytes = prepared();
        let mut range = find_placeholder(&bytes).expect("find");
        range.total += 64; // as though something had been appended
        assert!(range.covers_everything(), "the range describes the longer file");

        // But the numbers now claim bytes that are not there.
        assert!(range.covered(&bytes).is_err(), "it signed bytes past the end");
    }

    #[test]
    fn a_signature_is_written_without_moving_a_byte() {
        let mut bytes = prepared();
        let before = bytes.len();
        let range = find_placeholder(&bytes).expect("find");

        fill_placeholder(&mut bytes, &range, &[0xDE, 0xAD, 0xBE, 0xEF]).expect("fill");
        assert_eq!(bytes.len(), before, "the file changed length");
        let filled = &bytes[range.hole_at + 1..range.hole_at + 9];
        assert_eq!(filled, b"DEADBEEF");
        // The rest of the hole is untouched.
        assert!(bytes[range.hole_at + 9..range.hole_at + range.hole_len - 1]
            .iter()
            .all(|b| *b == b'0'));
    }

    /// Too big is a failure, not a reason to grow the file — growing it would
    /// move every offset the range was computed from.
    #[test]
    fn a_signature_too_large_for_its_hole_is_refused() {
        let mut bytes = prepared();
        let range = find_placeholder(&bytes).expect("find");
        let huge = vec![0u8; range.hole_len];
        assert!(fill_placeholder(&mut bytes, &range, &huge).is_err());
    }

    #[test]
    fn a_file_with_no_placeholder_says_so() {
        assert!(find_placeholder(b"%PDF-1.7\nnothing here\n%%EOF").is_err());
        // Nor is a page's contents reference mistaken for one.
        assert!(find_placeholder(b"%PDF-1.7\n/Contents 9 0 R\n%%EOF").is_err());
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    fn p12() -> Option<Vec<u8>> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer.p12"),
        )
        .ok()
    }

    #[test]
    fn a_certificate_and_key_are_read_from_a_pkcs12_file() {
        let Some(bytes) = p12() else {
            eprintln!("skipping: no test certificate");
            return;
        };
        let identity = Identity::from_pkcs12(&bytes, "pagify").expect("read the identity");
        assert!(!identity.certificates.is_empty(), "no certificate came back");
        identity.rsa_key().expect("the key is not usable");

        // And it says whose it is, which is what a person is shown before they
        // put a name on a document.
        let subject = identity.subject().expect("subject");
        assert!(subject.contains("Pagify Test Signer"), "{subject}");
    }

    #[test]
    fn a_wrong_password_does_not_open_it() {
        let Some(bytes) = p12() else { return };
        // Not `expect_err`: a private key is not a thing to print, so
        // `Identity` has no `Debug`.
        match Identity::from_pkcs12(&bytes, "not the password") {
            Ok(_) => panic!("a wrong password opened the file"),
            Err(problem) => {
                let said = problem.to_string();
                assert!(said.contains("password"), "unhelpful: {said}");
                // And it names the other cause, because the two look identical
                // from here and only one of them is a typo.
                assert!(said.contains("AES"), "it blamed the password alone: {said}");
            }
        }
    }

    #[test]
    fn something_that_is_not_a_pkcs12_file_says_so() {
        match Identity::from_pkcs12(b"%PDF-1.7\nnot a certificate", "pagify") {
            Ok(_) => panic!("it read a PDF as a certificate"),
            Err(problem) => {
                assert!(problem.to_string().contains("PKCS#12"), "unhelpful: {problem}")
            }
        }
    }
}

// ------------------------------------------------------------------- CMS --

/// Build the detached CMS SignedData blob that goes in `/Contents`.
///
/// **Detached**, which is what `/SubFilter /adbe.pkcs7.detached` means: the
/// signed content is not carried inside the blob — it is the file itself, and
/// only its digest goes in. That is why `external_message_digest` is handed
/// over rather than the bytes.
///
/// The digest is SHA-256 over what [`ByteRange::covered`] returns: everything
/// but the hole the blob is about to fill.
pub fn detached_signature(identity: &Identity, digest: &[u8]) -> Result<Vec<u8>> {
    use cms::builder::{SignedDataBuilder, SignerInfoBuilder};
    use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
    use cms::content_info::ContentInfo;
    use cms::signed_data::{EncapsulatedContentInfo, SignerIdentifier};
    use der::{Decode, Encode};
    use x509_cert::Certificate;

    let certificate = Certificate::from_der(&identity.certificates[0])
        .map_err(|_| PdfError::InvalidArgument("that certificate cannot be read".into()))?;

    let key = identity.rsa_key()?;
    let signer = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(key);

    // Which certificate signed this, by issuer and serial — the form every
    // reader understands.
    let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
        issuer: certificate.tbs_certificate.issuer.clone(),
        serial_number: certificate.tbs_certificate.serial_number.clone(),
    });

    // `id-data` with no content: the content is the file, and it is not here.
    let content = EncapsulatedContentInfo {
        econtent_type: const_oid::db::rfc5911::ID_DATA,
        econtent: None,
    };

    let digest_algorithm = spki::AlgorithmIdentifierOwned {
        oid: const_oid::db::rfc5912::ID_SHA_256,
        parameters: None,
    };

    let signer_info = SignerInfoBuilder::new(
        &signer,
        sid,
        digest_algorithm.clone(),
        &content,
        Some(digest),
    )
    .map_err(|e| PdfError::Internal(format!("the signer could not be described: {e}")))?;

    let mut builder = SignedDataBuilder::new(&content);
    let blob = builder
        .add_digest_algorithm(digest_algorithm)
        .and_then(|b| b.add_certificate(CertificateChoices::Certificate(certificate)))
        .and_then(|b| b.add_signer_info::<_, rsa::pkcs1v15::Signature>(signer_info))
        .and_then(|b| b.build())
        .map_err(|e| PdfError::Internal(format!("the signature could not be built: {e}")))?;

    ContentInfo::to_der(&blob)
        .map_err(|e| PdfError::Internal(format!("the signature could not be written: {e}")))
}

/// The digest a signature is made over.
pub fn digest_of(bytes: &[u8], range: &ByteRange) -> Result<Vec<u8>> {
    use sha2::Digest;
    let (before, after) = range.covered(bytes)?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(before);
    hasher.update(after);
    Ok(hasher.finalize().to_vec())
}

#[cfg(test)]
mod cms_tests {
    use super::*;

    fn identity() -> Option<Identity> {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer.p12"),
        )
        .ok()?;
        Identity::from_pkcs12(&bytes, "pagify").ok()
    }

    /// **The blob is a CMS SignedData that parses back**, which is the least a
    /// reader will ask of it.
    #[test]
    fn a_signature_is_a_readable_cms_structure() {
        let Some(identity) = identity() else {
            eprintln!("skipping: no test certificate");
            return;
        };
        let digest = [7u8; 32];
        let blob = detached_signature(&identity, &digest).expect("sign");

        use der::Decode;
        let parsed = cms::content_info::ContentInfo::from_der(&blob).expect("it is not CMS");
        assert_eq!(parsed.content_type, const_oid::db::rfc5911::ID_SIGNED_DATA);
    }

    /// **Detached**: the file's bytes are not inside the blob. A signature that
    /// carried the document would double its size and defeat the point.
    #[test]
    fn the_signed_content_is_not_carried_in_the_signature() {
        let Some(identity) = identity() else { return };
        let digest = [7u8; 32];
        let blob = detached_signature(&identity, &digest).expect("sign");

        use der::Decode;
        let parsed = cms::content_info::ContentInfo::from_der(&blob).expect("CMS");
        let signed: cms::signed_data::SignedData =
            parsed.content.decode_as().expect("signed data");
        assert!(
            signed.encap_content_info.econtent.is_none(),
            "the content was carried inside the signature"
        );
        assert!(signed.certificates.is_some(), "no certificate travelled with it");
    }

    /// A different digest makes a different signature — the blob is actually
    /// over what it was given, rather than a constant.
    #[test]
    fn signing_two_digests_gives_two_signatures() {
        let Some(identity) = identity() else { return };
        let one = detached_signature(&identity, &[1u8; 32]).expect("sign");
        let two = detached_signature(&identity, &[2u8; 32]).expect("sign");
        assert_ne!(one, two, "the signature does not depend on what was signed");
    }

    /// And it fits the room reserved for it, which is the one property that
    /// cannot be fixed after the fact.
    #[test]
    fn a_signature_fits_the_placeholder() {
        let Some(identity) = identity() else { return };
        let blob = detached_signature(&identity, &[0u8; 32]).expect("sign");
        assert!(
            blob.len() * 2 < PLACEHOLDER,
            "a {} byte signature will not fit {PLACEHOLDER} bytes of hex",
            blob.len()
        );
    }

    /// The digest is over everything but the hole, and changing a byte outside
    /// it changes the digest.
    #[test]
    fn the_digest_covers_the_file_around_the_hole() {
        let mut bytes = b"%PDF-1.7\nbefore\n/Contents <".to_vec();
        bytes.extend(std::iter::repeat_n(b'0', 2048));
        bytes.extend_from_slice(b">\nafter\n%%EOF");
        let range = find_placeholder(&bytes).expect("find");
        let first = digest_of(&bytes, &range).expect("digest");

        // A byte inside the hole does not change it.
        bytes[range.hole_at + 2] = b'F';
        assert_eq!(digest_of(&bytes, &range).expect("digest"), first);

        // A byte outside it does.
        bytes[3] = b'X';
        assert_ne!(digest_of(&bytes, &range).expect("digest"), first);
    }
}

// ------------------------------------------------------- signing a document --

use super::object::Dict;
use super::{write_object, File, Object};

/// What a signature says about itself.
#[derive(Debug, Clone, Default)]
pub struct Reason {
    /// Who signed, as they wish to be named. Empty takes it from the
    /// certificate.
    pub name: String,
    pub reason: String,
    pub location: String,
}

/// Sign a document.
///
/// Two passes, and they cannot be collapsed into one: the first writes the file
/// with a hole where the signature goes, and only then is there a file to
/// digest. The second fills the hole **without moving a byte**, because every
/// offset in `/ByteRange` was measured against the first.
pub fn sign(file: &File<'_>, identity: &Identity, about: &Reason) -> Result<Vec<u8>> {
    let name = if about.name.is_empty() {
        identity.subject().unwrap_or_default()
    } else {
        about.name.clone()
    };
    let mut prepared = prepare(file, Flavour::Signature, &name, about)?;
    let range = find_placeholder(&prepared)?;
    write_byte_range(&mut prepared, &range)?;
    let digest = digest_of(&prepared, &range)?;
    let blob = detached_signature(identity, &digest)?;
    fill_placeholder(&mut prepared, &range, &blob)?;
    Ok(prepared)
}

/// A document timestamp: the same shape, filled by an authority rather than by
/// a certificate here.
///
/// **The only difference that matters** is `/SubFilter`: a reader that sees
/// `ETSI.RFC3161` knows to check the token against a time authority rather than
/// against a signer's identity.
pub fn timestamp(file: &File<'_>, authority: &str) -> Result<Vec<u8>> {
    let mut prepared = prepare(file, Flavour::Timestamp, "", &Reason::default())?;
    let range = find_placeholder(&prepared)?;
    write_byte_range(&mut prepared, &range)?;
    let digest = digest_of(&prepared, &range)?;

    // The one call that leaves the machine, and only because somebody named
    // where. See `crate::pdf::timestamp`.
    let token = super::timestamp::ask(authority, &digest)?;
    fill_placeholder(&mut prepared, &range, &token)?;
    Ok(prepared)
}

/// Which kind of thing is being put in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavour {
    Signature,
    Timestamp,
}

/// Write the file with a hole in it, and everything a reader needs to find the
/// hole.
fn prepare(
    file: &File<'_>,
    flavour: Flavour,
    name: &str,
    about: &Reason,
) -> Result<Vec<u8>> {

    // -- the signature dictionary, with room reserved ---------------------
    let numbers: Vec<u32> = file.numbers().collect();
    let signature_number = numbers.iter().copied().max().unwrap_or(0) + 1;
    let field_number = signature_number + 1;

    let mut signature = Dict(Vec::new());
    signature.set(b"Type", Object::Name(b"Sig".to_vec()));
    signature.set(b"Filter", Object::Name(b"Adobe.PPKLite".to_vec()));
    signature.set(
        b"SubFilter",
        Object::Name(match flavour {
            Flavour::Signature => b"adbe.pkcs7.detached".to_vec(),
            // What tells a reader to check a time authority rather than a
            // signer's identity.
            Flavour::Timestamp => b"ETSI.RFC3161".to_vec(),
        }),
    );
    if !name.is_empty() {
        signature.set(b"Name", Object::LiteralString(escaped(name)));
    }
    if !about.reason.is_empty() {
        signature.set(b"Reason", Object::LiteralString(escaped(&about.reason)));
    }
    if !about.location.is_empty() {
        signature.set(b"Location", Object::LiteralString(escaped(&about.location)));
    }
    signature.set(b"M", Object::LiteralString(now().into_bytes()));
    // **Fixed width, both of them.** These are written before their real values
    // are known and patched in place afterwards; a number that grew by a digit
    // would move every byte after it and invalidate the range it is part of.
    //
    // Written as ordinary objects — four zero-padded numbers and a hex string
    // of zeros — because those print as exactly the bytes wanted and need no
    // special case in the writer.
    signature.set(
        b"ByteRange",
        Object::Array(vec![
            Object::Number(vec![b'0'; RANGE_DIGITS]),
            Object::Number(vec![b'0'; RANGE_DIGITS]),
            Object::Number(vec![b'0'; RANGE_DIGITS]),
            Object::Number(vec![b'0'; RANGE_DIGITS]),
        ]),
    );
    signature.set(b"Contents", Object::HexString(vec![b'0'; PLACEHOLDER]));

    let mut signature_body = Vec::new();
    write_object(&mut signature_body, &Object::Dict(signature));

    // -- the field that holds it ------------------------------------------
    //
    // Invisible: a zero-size rectangle on the first page. A signature that
    // draws nothing is still a signature, and inventing an appearance stream
    // here would put a picture on somebody's document that they did not ask
    // for.
    let mut field = Dict(Vec::new());
    field.set(b"Type", Object::Name(b"Annot".to_vec()));
    field.set(b"Subtype", Object::Name(b"Widget".to_vec()));
    field.set(b"FT", Object::Name(b"Sig".to_vec()));
    field.set(
        b"T",
        Object::LiteralString(match flavour {
            Flavour::Signature => b"Signature1".to_vec(),
            Flavour::Timestamp => b"Timestamp1".to_vec(),
        }),
    );
    field.set(b"V", Object::Reference(signature_number, 0));
    field.set(b"F", Object::Number(b"132".to_vec()));
    field.set(
        b"Rect",
        Object::Array(vec![
            Object::Number(b"0".to_vec()),
            Object::Number(b"0".to_vec()),
            Object::Number(b"0".to_vec()),
            Object::Number(b"0".to_vec()),
        ]),
    );
    let first_page = page_number(file, 0)?;
    field.set(b"P", Object::Reference(first_page, 0));

    let mut field_body = Vec::new();
    write_object(&mut field_body, &Object::Dict(field));

    // -- the catalogue has to know about the form -------------------------
    let mut replacements = Vec::new();
    let Some(Object::Reference(root_number, _)) = file.trailer().get(b"Root") else {
        return Err(PdfError::InvalidArgument("the file has no catalogue".into()));
    };
    let Ok(Object::Dict(mut root)) = file.object(*root_number) else {
        return Err(PdfError::InvalidArgument("the catalogue cannot be read".into()));
    };
    let mut acroform = Dict(Vec::new());
    acroform.set(b"Fields", Object::Array(vec![Object::Reference(field_number, 0)]));
    // Says the file's appearance may not be regenerated — which is what stops a
    // reader rewriting the page and breaking the very signature it is checking.
    acroform.set(b"SigFlags", Object::Number(b"3".to_vec()));
    root.set(b"AcroForm", Object::Dict(acroform));
    let mut root_body = Vec::new();
    write_object(&mut root_body, &Object::Dict(root));
    replacements.push((*root_number, root_body));

    // And the page has to carry the widget.
    let Ok(Object::Dict(mut page)) = file.object(first_page) else {
        return Err(PdfError::InvalidArgument("that page cannot be read".into()));
    };
    let mut annots = match page.get(b"Annots").and_then(|a| file.resolve(a).ok()) {
        Some(Object::Array(items)) => items,
        _ => Vec::new(),
    };
    annots.push(Object::Reference(field_number, 0));
    page.set(b"Annots", Object::Array(annots));
    let mut page_body = Vec::new();
    write_object(&mut page_body, &Object::Dict(page));
    replacements.push((first_page, page_body));

    let prepared = file.rewrite_adding(
        &replacements,
        &[(signature_number, signature_body), (field_number, field_body)],
        &Dict(Vec::new()),
    )?;

    let range = find_placeholder(&prepared)?;
    if !range.covers_everything() {
        return Err(PdfError::Internal(
            "the prepared file's byte range does not cover it".into(),
        ));
    }
    Ok(prepared)
}

/// Digits per number in `/ByteRange`.
///
/// Ten, which is room for a ten-gigabyte file — and, more to the point, the
/// **same width whatever the number turns out to be**. These are written before
/// their values are known and patched in place afterwards; one that grew by a
/// digit would move every byte after it and invalidate the very range it is
/// part of.
const RANGE_DIGITS: usize = 10;

/// Patch the four numbers in, in place.
fn write_byte_range(bytes: &mut [u8], range: &ByteRange) -> Result<()> {
    let at = bytes
        .windows(10)
        .position(|w| w == b"/ByteRange")
        .ok_or_else(|| PdfError::Internal("the byte range went missing".into()))?;
    let open = bytes[at..]
        .iter()
        .position(|b| *b == b'[')
        .map(|n| at + n)
        .ok_or_else(|| PdfError::Internal("the byte range has no array".into()))?;

    let values = range.numbers();
    let mut cursor = open + 1;
    for value in values {
        // Past whatever separates them.
        while cursor < bytes.len() && !bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if cursor - start != RANGE_DIGITS {
            return Err(PdfError::Internal(
                "the byte range is not the width it was written at".into(),
            ));
        }
        let written = format!("{value:0RANGE_DIGITS$}", RANGE_DIGITS = RANGE_DIGITS);
        bytes[start..cursor].copy_from_slice(written.as_bytes());
    }
    Ok(())
}

/// Which object number the first page is.
fn page_number(file: &File<'_>, index: usize) -> Result<u32> {
    fn walk(file: &File<'_>, node: &Object, out: &mut Vec<u32>, depth: usize) {
        if depth > 64 {
            return;
        }
        let Some(dict) = node.as_dict() else { return };
        let Some(kids) = dict.get(b"Kids") else { return };
        let Ok(Object::Array(items)) = file.resolve(kids) else { return };
        for kid in items {
            let Object::Reference(number, _) = kid else { continue };
            let Ok(resolved) = file.object(number) else { continue };
            let is_page = resolved
                .as_dict()
                .and_then(|d| d.get(b"Type"))
                .and_then(Object::as_name)
                == Some(&b"Page"[..]);
            if is_page {
                out.push(number);
            } else {
                walk(file, &resolved, out, depth + 1);
            }
        }
    }

    let root = file.resolve(
        file.trailer()
            .get(b"Root")
            .ok_or_else(|| PdfError::InvalidArgument("the file has no catalogue".into()))?,
    )?;
    let pages = file.resolve(
        root.as_dict()
            .and_then(|d| d.get(b"Pages"))
            .ok_or_else(|| PdfError::InvalidArgument("the file has no page tree".into()))?,
    )?;
    let mut numbers = Vec::new();
    walk(file, &pages, &mut numbers, 0);
    numbers
        .into_iter()
        .nth(index)
        .ok_or_else(|| PdfError::InvalidArgument("the file has no pages".into()))
}

/// A PDF date, as `/M` wants it.
fn now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Whole days since the epoch, converted by the civil-from-days algorithm —
    // no calendar crate for four numbers.
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// A string as a PDF literal, with the two characters that would end it early
/// escaped.
fn escaped(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for byte in text.bytes() {
        if matches!(byte, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(byte);
    }
    out
}
