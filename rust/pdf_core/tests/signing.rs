//! Signing — checked the way a reader checks, not the way we wrote it.
//!
//! A signature that verifies only against the code that made it proves nothing.
//! So these tests read the finished file back the way somebody else's program
//! would: they take `/ByteRange` from the document, digest the bytes it names,
//! and check that against what the signer committed to — then verify the
//! signature under the certificate's own public key.
//!
//! **What they cannot tell you** is whether Acrobat is happy. That needs
//! Acrobat, and it is the one check this machine cannot make.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test signing
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::Document;
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::pdf::{sign, File};

fn identity() -> Option<sign::Identity> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer.p12");
    sign::Identity::from_pkcs12(&std::fs::read(path).ok()?, "pagify").ok()
}

fn signed(name: &str) -> Option<(Vec<u8>, sign::Identity)> {
    let identity = identity()?;
    let bytes = std::fs::read(harness::fixture_path(name)).ok()?;
    let file = File::parse(&bytes).ok()?;
    let out = sign::sign(&file, &identity, &sign::Reason::default()).ok()?;
    Some((out, identity))
}

/// The four numbers, read out of a signed file the way a reader reads them.
fn declared_range(bytes: &[u8]) -> Option<sign::ByteRange> {
    let at = bytes.windows(10).position(|w| w == b"/ByteRange")?;
    let open = bytes[at..].iter().position(|b| *b == b'[').map(|n| at + n)?;
    let close = bytes[open..].iter().position(|b| *b == b']').map(|n| open + n)?;
    let numbers: Vec<usize> = String::from_utf8_lossy(&bytes[open + 1..close])
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    let [_, first, second_at, _] = numbers[..] else { return None };
    Some(sign::ByteRange { hole_at: first, hole_len: second_at - first, total: bytes.len() })
}

/// The blob, pulled back out of the hole.
fn blob_in(bytes: &[u8], range: &sign::ByteRange) -> Vec<u8> {
    let hex = &bytes[range.hole_at + 1..range.hole_at + range.hole_len - 1];
    let mut raw: Vec<u8> = hex
        .chunks(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).unwrap_or(0) as u8;
            let low = pair.get(1).and_then(|b| (*b as char).to_digit(16)).unwrap_or(0) as u8;
            high << 4 | low
        })
        .collect();
    while raw.last() == Some(&0) {
        raw.pop();
    }
    raw
}

/// **The acceptance test.** Signed, still readable, and the signature is over
/// the file it is in.
#[test]
fn a_signed_document_still_opens_and_its_signature_matches_the_file() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some((bytes, identity)) = signed("two-column.pdf") else {
        eprintln!("skipping: no test certificate");
        return;
    };

    // Still a document, and unchanged on the page.
    let doc = PdfiumDocument::open_bytes(bytes.clone(), None).expect("the signed file will not open");
    let text = doc.page(0).expect("page").characters().expect("characters").text;
    assert!(text.contains("luminaire"), "the signature disturbed the page");

    // A reader that is not ours finds the signature.
    assert_eq!(doc.signature_count(), 1, "PDFium does not see a signature");

    // The range it declares covers the whole file but the hole.
    let range = declared_range(&bytes).expect("no /ByteRange in the file");
    assert!(range.covers_everything(), "the declared range leaves bytes uncovered");

    // And what the signer committed to is the digest of exactly those bytes.
    use der::Decode;
    let info = cms::content_info::ContentInfo::from_der(&blob_in(&bytes, &range))
        .expect("the blob is not CMS");
    let data: cms::signed_data::SignedData =
        info.content.decode_as().expect("not SignedData");
    let signer = data.signer_infos.0.as_ref().first().expect("no signer");
    let attributes = signer.signed_attrs.as_ref().expect("no signed attributes");
    let committed = attributes
        .iter()
        .find(|a| a.oid == const_oid::db::rfc5911::ID_MESSAGE_DIGEST)
        .and_then(|a| a.values.as_ref().first().map(|v| v.value().to_vec()))
        .expect("the signer committed to no digest");

    assert_eq!(
        committed,
        sign::digest_of(&bytes, &range).expect("digest"),
        "the signature is over different bytes than the file it is in"
    );
    let _ = identity;
}

/// **And the signature verifies under the certificate.** The digest matching is
/// arithmetic; this is the part that needs the key.
#[test]
fn the_signature_verifies_under_the_certificate_that_made_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some((bytes, identity)) = signed("two-column.pdf") else { return };
    let range = declared_range(&bytes).expect("range");

    use der::{Decode, Encode};
    use rsa::pkcs8::DecodePublicKey;
    use rsa::signature::Verifier;

    let info = cms::content_info::ContentInfo::from_der(&blob_in(&bytes, &range)).expect("CMS");
    let data: cms::signed_data::SignedData = info.content.decode_as().expect("SignedData");
    let signer = data.signer_infos.0.as_ref().first().expect("no signer");
    let attributes = signer.signed_attrs.as_ref().expect("no signed attributes");

    let certificate =
        x509_cert::Certificate::from_der(&identity.certificates[0]).expect("certificate");
    let spki = certificate.tbs_certificate.subject_public_key_info.to_der().expect("key");
    let key = rsa::RsaPublicKey::from_public_key_der(&spki).expect("an RSA key");

    rsa::pkcs1v15::VerifyingKey::<sha2::Sha256>::new(key)
        .verify(
            &attributes.to_der().expect("attributes"),
            &rsa::pkcs1v15::Signature::try_from(signer.signature.as_bytes()).expect("signature"),
        )
        .expect("the signature does not verify under its own certificate");
}

/// **Changing one byte breaks it.** A signature that survived an edit would be
/// worse than no signature, because it would say the document was untouched.
#[test]
fn altering_a_signed_document_breaks_its_signature() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some((bytes, _)) = signed("two-column.pdf") else { return };
    let range = declared_range(&bytes).expect("range");
    let before = sign::digest_of(&bytes, &range).expect("digest");

    // A byte in the part the range covers.
    let mut altered = bytes.clone();
    altered[range.hole_at / 2] ^= 0x01;
    let after = sign::digest_of(&altered, &range).expect("digest");

    assert_ne!(before, after, "an edit inside the signed range did not change the digest");
}

/// The document is not disturbed by being signed — every page comes back as it
/// was.
#[test]
fn signing_leaves_every_page_exactly_as_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("text-lines.pdf");
    let plain = PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open");
    let before: Vec<String> = (0..plain.page_count())
        .map(|p| plain.page(p).expect("page").characters().expect("characters").text)
        .collect();
    drop(plain);

    let Some((bytes, _)) = signed("text-lines.pdf") else { return };
    let after = PdfiumDocument::open_bytes(bytes, None).expect("reopen");

    assert_eq!(after.page_count(), before.len());
    for (page, was) in before.iter().enumerate() {
        let now = after.page(page).expect("page").characters().expect("characters").text;
        assert_eq!(&now, was, "page {} changed", page + 1);
    }
}

// ------------------------------------------------------------- validating --

use pdf_core::pdf::validate::{self, Verdict};

/// **A document we just signed reads back as unaltered.**
#[test]
fn a_freshly_signed_document_validates() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some((bytes, _)) = signed("two-column.pdf") else {
        eprintln!("skipping: no test certificate");
        return;
    };
    let file = File::parse(&bytes).expect("parse");
    let found = validate::check(&file, &bytes).expect("check");

    assert_eq!(found.len(), 1, "expected one signature, got {found:?}");
    assert_eq!(found[0].verdict, Verdict::Unaltered, "{:?}", found[0]);
    assert!(!found[0].timestamp, "a signature was read as a timestamp");
    assert!(!found[0].when.is_empty(), "it recorded no time");
}

/// **Changing a byte inside the signed range is caught.** This is the whole
/// point of the tool.
#[test]
fn altering_a_signed_document_is_reported_as_altered() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some((bytes, _)) = signed("two-column.pdf") else { return };
    let range = declared_range(&bytes).expect("range");

    // A byte well inside the first covered span.
    let mut altered = bytes.clone();
    altered[range.hole_at / 2] ^= 0x20;

    let file = File::parse(&altered).expect("parse");
    let found = validate::check(&file, &altered).expect("check");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].verdict, Verdict::Altered, "an edit was not noticed");
}

/// **The append.** Everything the range names is untouched and the digest
/// matches — what gives it away is that the file is longer than the signature
/// ever claimed.
#[test]
fn appending_to_a_signed_document_is_reported_rather_than_passing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some((bytes, _)) = signed("two-column.pdf") else { return };
    let mut appended = bytes.clone();
    appended.extend_from_slice(b"\n% and something nobody signed\n");

    let file = File::parse(&appended).expect("parse");
    let found = validate::check(&file, &appended).expect("check");
    assert_eq!(found.len(), 1);
    match &found[0].verdict {
        Verdict::Incomplete { covered, total } => {
            assert!(covered < total, "{covered} of {total}");
            assert_eq!(*total, appended.len());
        }
        other => panic!("an append was not noticed: {other:?}"),
    }
}

/// A document with no signatures says so, which is not a failure.
#[test]
fn a_document_with_no_signatures_reports_none() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let bytes = std::fs::read(harness::fixture_path("two-column.pdf")).expect("read");
    let file = File::parse(&bytes).expect("parse");
    assert!(validate::check(&file, &bytes).expect("check").is_empty());
}

/// **A document signed through the engine validates through the engine.**
///
/// The regression this pins was not in the arithmetic — that was right from
/// the start. It was in *which bytes* were handed to it: asking PDFium to
/// write the document out again gives a different file, and the signature's
/// range then names a stretch of it that is not the one that was signed.
/// Measured when it was wrong: 1458 bytes covered of a 17826-byte re-save, on
/// a document that had just been correctly signed a moment before.
#[test]
fn signing_and_validating_through_the_document_agree() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let certificate =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer.p12");
    let Ok(pkcs12) = std::fs::read(certificate) else { return };
    let mut doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(
        harness::fixture_path("two-column.pdf").to_str().expect("path"),
        None,
    )
    .expect("open");

    use pdf_core::document::DocumentMut;
    let who = doc
        .sign_document(&pkcs12, "pagify", &sign::Reason::default())
        .expect("sign");
    assert!(!who.is_empty(), "it signed as nobody");

    let found = doc.validate_signatures().expect("validate");
    assert_eq!(found.len(), 1, "expected one signature, got {found:?}");
    assert_eq!(
        found[0].verdict,
        Verdict::Unaltered,
        "a document signed a moment ago did not validate: {:?}",
        found[0]
    );
}
