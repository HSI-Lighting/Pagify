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

/// The SM2 test identity — the only kind that signs.
fn identity() -> sign::Identity {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer-sm2.p12");
    sign::Identity::from_pkcs12(&std::fs::read(path).expect("the SM2 identity is committed"), "pagify")
        .expect("read the identity")
}

fn signed(name: &str) -> (Vec<u8>, sign::Identity) {
    let identity = identity();
    let bytes = std::fs::read(harness::fixture_path(name)).expect("fixture");
    let file = File::parse(&bytes).expect("parse");
    let out = sign::sign(&file, &identity, &sign::Reason::default()).expect("sign");
    (out, identity)
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

/// The blob, pulled back out of the hole — cut where its DER header says it
/// ends, not where the padding's zeros begin: one signature in 256 ends in a
/// zero byte.
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
    let length = match raw.get(1).copied() {
        Some(first) if first < 0x80 => 2 + first as usize,
        Some(first) => {
            let count = (first & 0x7f) as usize;
            let mut length = 0usize;
            for byte in &raw[2..2 + count] {
                length = length * 256 + *byte as usize;
            }
            2 + count + length
        }
        None => 0,
    };
    raw.truncate(length);
    raw
}

/// **The acceptance test.** Signed, still readable, and the signature is over
/// the file it is in.
#[test]
fn a_signed_document_still_opens_and_its_signature_matches_the_file() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (bytes, identity) = signed("two-column.pdf");

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
/// arithmetic; this is the part that needs the key. Checked with the `sm2`
/// crate directly, under the certificate carried *in the blob* and the
/// standard's default distinguishing ID — the way a reader that is not ours
/// would check it, with none of `validate.rs` in the way.
#[test]
fn the_signature_verifies_under_the_certificate_that_made_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (bytes, _) = signed("two-column.pdf");
    let range = declared_range(&bytes).expect("range");

    use der::Encode;
    use sm2::dsa::signature::Verifier;
    use sm2::pkcs8::DecodePublicKey;

    let data = signed_data_in(&bytes, &range);
    let signer = data.signer_infos.0.as_ref().first().expect("no signer");
    let attributes = signer.signed_attrs.as_ref().expect("no signed attributes");
    let certificate = match data.certificates.as_ref().and_then(|c| c.0.iter().next()) {
        Some(cms::cert::CertificateChoices::Certificate(c)) => c.clone(),
        _ => panic!("no certificate travelled with the signature"),
    };
    let spki = certificate.tbs_certificate.subject_public_key_info.to_der().expect("key");
    let key = sm2::PublicKey::from_public_key_der(&spki).expect("an SM2 key");
    let verifying = sm2::dsa::VerifyingKey::new("1234567812345678", key).expect("verifying key");

    // The signature value is `SEQUENCE { INTEGER r, INTEGER s }`; read it
    // here by hand rather than through the engine's own decoder.
    let (r, s) = integer_pair(signer.signature.as_bytes());
    let mut raw = [0u8; 64];
    raw[32 - r.len()..32].copy_from_slice(&r);
    raw[64 - s.len()..].copy_from_slice(&s);
    let signature = sm2::dsa::Signature::from_slice(&raw).expect("signature");

    let message = attributes.to_der().expect("attributes");
    verifying
        .verify(&message, &signature)
        .expect("the signature does not verify under its own certificate");
    // And not under another identity string — the convention both sides
    // must share.
    let other = sm2::dsa::VerifyingKey::new("somebody else", key).expect("key");
    assert!(other.verify(&message, &signature).is_err());
}

/// Two DER INTEGERs out of a SEQUENCE, magnitudes only (the sign byte a
/// number with its top bit set carries is dropped).
fn integer_pair(der: &[u8]) -> (Vec<u8>, Vec<u8>) {
    assert_eq!(der[0], 0x30, "not a SEQUENCE");
    let mut at = 2;
    let mut numbers = Vec::new();
    while numbers.len() < 2 {
        assert_eq!(der[at], 0x02, "not an INTEGER");
        let len = der[at + 1] as usize;
        let mut value = der[at + 2..at + 2 + len].to_vec();
        if value.first() == Some(&0) {
            value.remove(0);
        }
        numbers.push(value);
        at += 2 + len;
    }
    assert_eq!(at, der.len(), "bytes after the two numbers");
    let s = numbers.pop().unwrap();
    (numbers.pop().unwrap(), s)
}

/// **Changing one byte breaks it.** A signature that survived an edit would be
/// worse than no signature, because it would say the document was untouched.
#[test]
fn altering_a_signed_document_breaks_its_signature() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (bytes, _) = signed("two-column.pdf");
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

    let (bytes, _) = signed("text-lines.pdf");
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

    let (bytes, _) = signed("two-column.pdf");
    let file = File::parse(&bytes).expect("parse");
    let found = validate::check(&file, &bytes).expect("check");

    assert_eq!(found.len(), 1, "expected one signature, got {found:?}");
    assert_eq!(found[0].verdict, Verdict::Unaltered, "{:?}", found[0]);
    assert!(!found[0].when.is_empty(), "it recorded no time");
}

/// **Changing a byte inside the signed range is caught.** This is the whole
/// point of the tool.
#[test]
fn altering_a_signed_document_is_reported_as_altered() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (bytes, _) = signed("two-column.pdf");
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

    let (bytes, _) = signed("two-column.pdf");
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
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer-sm2.p12");
    let pkcs12 = std::fs::read(certificate).expect("the SM2 identity is committed");
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

// ---------------------------------------------------------------------------
// What the blob says about itself is not evidence. Found by audit: the check
// used to compare the file's digest with the digest *inside* the signature
// and stop there — so a forger could edit the file, recompute the digest and
// write it back into the blob, with no key at all, and be told "unchanged".
// ---------------------------------------------------------------------------

/// Write a different blob into the hole: the hole is zeroed first, because a
/// shorter blob would otherwise leave the tail of the old one behind it.
fn reblob(bytes: &mut [u8], range: &sign::ByteRange, blob: &[u8]) {
    for byte in &mut bytes[range.hole_at + 1..range.hole_at + range.hole_len - 1] {
        *byte = b'0';
    }
    sign::fill_placeholder(bytes, range, blob).expect("the blob fits the hole");
}

/// The signature's `SignedData`, pulled out of the file the way a reader does.
fn signed_data_in(bytes: &[u8], range: &sign::ByteRange) -> cms::signed_data::SignedData {
    use der::Decode;
    let info = cms::content_info::ContentInfo::from_der(&blob_in(bytes, range)).expect("CMS");
    info.content.decode_as().expect("SignedData")
}

/// And back into a blob, after being tampered with.
fn blob_from(data: &cms::signed_data::SignedData) -> Vec<u8> {
    use der::Encode;
    cms::content_info::ContentInfo {
        content_type: const_oid::db::rfc5911::ID_SIGNED_DATA,
        content: der::Any::encode_from(data).expect("encode"),
    }
    .to_der()
    .expect("DER")
}

/// **The forgery.** Alter the file, recompute the digest, write it back into
/// the blob. Every number the blob carries now agrees with the file; only the
/// signature over the attributes — which the forger cannot make — disagrees.
#[test]
fn a_document_altered_and_rehashed_is_not_reported_as_unchanged() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (bytes, _) = signed("two-column.pdf");
    let range = declared_range(&bytes).expect("range");
    let before = sign::digest_of(&bytes, &range).expect("digest");

    let mut forged = bytes.clone();
    forged[range.hole_at / 2] ^= 0x20;
    let after = sign::digest_of(&forged, &range).expect("digest");
    assert_ne!(before, after);

    // The digest the signer committed to, overwritten byte for byte with the
    // one that matches the altered file. Same length, so nothing else moves.
    let blob = blob_in(&bytes, &range);
    let at = blob
        .windows(before.len())
        .position(|w| w == &before[..])
        .expect("the committed digest is in the blob");
    let mut rehashed = blob.clone();
    rehashed[at..at + after.len()].copy_from_slice(&after);
    reblob(&mut forged, &range, &rehashed);

    let file = File::parse(&forged).expect("parse");
    let found = validate::check(&file, &forged).expect("check");
    assert_eq!(found.len(), 1);
    assert!(
        matches!(found[0].verdict, Verdict::Invalid(_)),
        "a re-hashed forgery was not called invalid: {:?}",
        found[0]
    );
    assert!(found[0].signer.is_none(), "nobody signed a forgery, yet somebody is named");
    assert!(found[0].verdict.describe().contains("NOT VALID"), "{}", found[0].verdict.describe());
}

/// A genuine signature names the certificate it verified under — the one
/// piece of identity that is evidence rather than a label.
#[test]
fn a_verified_signature_names_the_certificate_it_verified_under() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (bytes, identity) = signed("two-column.pdf");
    let file = File::parse(&bytes).expect("parse");
    let found = validate::check(&file, &bytes).expect("check");
    assert_eq!(found[0].verdict, Verdict::Unaltered, "{:?}", found[0]);
    assert_eq!(
        found[0].signer.as_deref(),
        Some(identity.subject().expect("subject").as_str()),
        "the signer named is not the certificate that signed"
    );
}

/// **Fail closed.** A blob that carries no certificate cannot be checked, and
/// "cannot be checked" is what is said — never "unchanged".
#[test]
fn a_signature_with_no_certificate_cannot_be_called_unchanged() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (bytes, _) = signed("two-column.pdf");
    let range = declared_range(&bytes).expect("range");

    let mut data = signed_data_in(&bytes, &range);
    data.certificates = None;
    let mut stripped = bytes.clone();
    reblob(&mut stripped, &range, &blob_from(&data));

    let file = File::parse(&stripped).expect("parse");
    let found = validate::check(&file, &stripped).expect("check");
    assert!(
        matches!(found[0].verdict, Verdict::Unreadable(_)),
        "a signature with nothing to check against was not called unreadable: {:?}",
        found[0]
    );
    assert_ne!(found[0].verdict, Verdict::Unaltered);
}

/// And a scheme this does not implement is reported as that — named in words
/// where there are words, as the number where there are not — and never
/// judged. The certificate carried is an SM2 one, so ECDSA is named with
/// that curve.
#[test]
fn a_signature_scheme_this_does_not_verify_is_named_not_judged() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (bytes, _) = signed("two-column.pdf");
    let range = declared_range(&bytes).expect("range");

    for (scheme, expected) in [
        (const_oid::db::rfc5912::ECDSA_WITH_SHA_256, "ECDSA SM2 / SHA-256"),
        (const_oid::ObjectIdentifier::new_unwrap("1.2.3.4"), "1.2.3.4 / SM3"),
    ] {
        let mut data = signed_data_in(&bytes, &range);
        let mut signers: Vec<cms::signed_data::SignerInfo> =
            data.signer_infos.0.iter().cloned().collect();
        signers[0].signature_algorithm.oid = scheme;
        data.signer_infos = cms::signed_data::SignerInfos(
            der::asn1::SetOfVec::try_from(signers).expect("signers"),
        );
        let mut relabelled = bytes.clone();
        reblob(&mut relabelled, &range, &blob_from(&data));

        let file = File::parse(&relabelled).expect("parse");
        let found = validate::check(&file, &relabelled).expect("check");
        match &found[0].verdict {
            Verdict::Unreadable(why) => assert!(why.contains(expected), "{why}"),
            other => panic!("an unknown scheme was judged rather than declined: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Document timestamps are not checked any more — nothing here asks a time
// authority for one, and a token is a third party's statement in a third
// party's scheme. What must not happen is the misreading the audit found: a
// token commits to the file through the imprint inside its TSTInfo, not
// through its digest attribute, and read as an ordinary signature every
// genuine timestamp came out as an alteration.
// ---------------------------------------------------------------------------

/// A timestamp token from another application is declined, and never called
/// an alteration — not over the bytes it was made for, and not over altered
/// ones either, because nothing about it was checked.
#[test]
fn a_document_timestamp_from_another_application_is_declined_never_called_altered() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (bytes, _) = signed("two-column.pdf");
    let range = declared_range(&bytes).expect("range");

    // The signer's blob, relabelled as a token: `id-ct-TSTInfo` content in
    // place of the detached `id-data`. What a real token carries there is a
    // TSTInfo; what matters here is only that the check does not read on.
    let mut data = signed_data_in(&bytes, &range);
    data.encap_content_info = cms::signed_data::EncapsulatedContentInfo {
        econtent_type: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4"),
        econtent: Some(
            der::Any::new(der::Tag::OctetString, b"not a TSTInfo".to_vec()).expect("content"),
        ),
    };
    let mut stamped = bytes.clone();
    reblob(&mut stamped, &range, &blob_from(&data));

    for altered in [false, true] {
        let mut checked = stamped.clone();
        if altered {
            checked[range.hole_at / 2] ^= 0x20;
        }
        let file = File::parse(&checked).expect("parse");
        let found = validate::check(&file, &checked).expect("check");
        match &found[0].verdict {
            Verdict::Unreadable(why) => assert!(why.contains("timestamp"), "{why}"),
            other => panic!("a token was judged rather than declined (altered: {altered}): {other:?}"),
        }
        assert!(found[0].signer.is_none(), "a token nobody checked names an authority");
    }
}

// ---------------------------------------------------------------------------
// A signature is worth nothing until it is on disk. Found by audit: the signed
// bytes were kept in memory and marked clean, a save re-serialised the
// document through PDFium and broke the byte range, and a close threw the
// signature away without asking.
// ---------------------------------------------------------------------------

fn open_document(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None)
        .expect("open")
}

fn certify(doc: &mut PdfiumDocument) -> bool {
    use pdf_core::document::DocumentMut;
    let certificate =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer-sm2.p12");
    let Ok(pkcs12) = std::fs::read(certificate) else { return false };
    doc.sign_document(&pkcs12, "pagify", &sign::Reason::default()).expect("sign");
    true
}

fn verdicts_in(bytes: &[u8]) -> Vec<Verdict> {
    let file = File::parse(bytes).expect("parse");
    validate::check(&file, bytes).expect("check").into_iter().map(|s| s.verdict).collect()
}

/// **The acceptance test.** Sign, save the ordinary way, read the file back
/// with none of the signing code in the way: unaltered.
#[test]
fn a_signed_document_saved_is_the_signed_bytes() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    use pdf_core::document::DocumentMut;

    let mut doc = open_document("two-column.pdf");
    if !certify(&mut doc) {
        return;
    }
    assert!(doc.is_dirty(), "a signature that is not on disk is unsaved work");

    let mut saved = Vec::new();
    doc.save_incremental(&mut saved).expect("save");
    assert!(!doc.is_dirty(), "the save did not count");
    assert_eq!(verdicts_in(&saved), vec![Verdict::Unaltered], "the saved file does not validate");

    // A full copy of a signed document is the same bytes: there is nothing
    // else it could honestly be.
    let mut copy = Vec::new();
    doc.save_full_copy(&mut copy).expect("copy");
    assert_eq!(copy, saved, "a copy of a signed document was re-serialised");
}

/// An edit after signing is saved as a later revision: the signature still
/// covers what it covered, and the check says the rest was never signed.
#[test]
fn an_edit_after_signing_is_saved_as_a_revision_the_signature_does_not_cover() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    use pdf_core::document::{Color, DocumentMut, Rect};

    let mut doc = open_document("two-column.pdf");
    if !certify(&mut doc) {
        return;
    }
    let mut signed = Vec::new();
    doc.save_incremental(&mut signed).expect("save the signed file");
    doc.whiteout(
        0,
        Rect { left: 100.0, top: 100.0, right: 200.0, bottom: 120.0 },
        Color { r: 255, g: 255, b: 255, a: 255 },
    )
    .expect("an edit after signing");

    let mut saved = Vec::new();
    doc.save_incremental(&mut saved).expect("save");
    // The signed bytes are the start of the saved file, untouched — not
    // merely "the range does not reach the end", which a full rewrite with
    // the old numbers in it also says. Found when the check started looking
    // at the digest before calling anything incomplete: a whiteout after
    // signing had been re-serialising the whole file.
    assert!(saved.starts_with(&signed), "the signed bytes were not kept in place");
    let file = File::parse(&saved).expect("parse");
    let found = validate::check(&file, &saved).expect("check");
    match found.as_slice() {
        [found] => match found.verdict {
            Verdict::Incomplete { covered, total } => {
                assert_eq!(covered, signed.len(), "the signature covers the signed file exactly");
                assert!(covered < total, "{covered} of {total}");
                assert_eq!(found.signer.as_deref(), Some("CN=Pagify SM2 Test Signer,O=Pagify"));
            }
            ref other => panic!("an edit after signing should leave the signature over the earlier revision, got {other:?}"),
        },
        other => panic!("one signature expected, got {other:?}"),
    }

    // And the edit is there: the whiteout is on the page of the saved file.
    let reopened = PdfiumDocument::open_bytes(saved, None).expect("reopen");
    assert_eq!(reopened.page(0).expect("page").characters().expect("characters").text,
        doc.page(0).expect("page").characters().expect("characters").text);
}

// ---------------------------------------------------------------------------
// What the blob says about itself, and the certificates it can be checked
// against.
// ---------------------------------------------------------------------------

/// **The identifiers, read back from the DER.** SM3 is `1.2.156.10197.1.401`
/// wherever a digest algorithm is named, SM2-with-SM3 is `1.2.156.10197.1.501`,
/// and what the signer committed to is the SM3 digest of the bytes the range
/// covers — not SHA-256, which the RSA signer used to commit to.
#[test]
fn a_signature_names_the_sm_algorithms_by_their_oids() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let (bytes, _) = signed("two-column.pdf");
    let range = declared_range(&bytes).expect("range");
    let data = signed_data_in(&bytes, &range);
    let sm3 = const_oid::ObjectIdentifier::new_unwrap("1.2.156.10197.1.401");
    let sm2_with_sm3 = const_oid::ObjectIdentifier::new_unwrap("1.2.156.10197.1.501");

    assert_eq!(data.digest_algorithms.iter().map(|a| a.oid).collect::<Vec<_>>(), vec![sm3]);
    let signer = data.signer_infos.0.as_ref().first().expect("signer");
    assert_eq!(signer.digest_alg.oid, sm3);
    assert_eq!(signer.signature_algorithm.oid, sm2_with_sm3);
    let committed = signer
        .signed_attrs
        .as_ref()
        .expect("attributes")
        .iter()
        .find(|a| a.oid == const_oid::db::rfc5911::ID_MESSAGE_DIGEST)
        .and_then(|a| a.values.as_ref().first().map(|v| v.value().to_vec()))
        .expect("digest");
    assert_eq!(committed, sign::digest_of(&bytes, &range).expect("digest"));
    let (before, after) = range.covered(&bytes).expect("covered");
    let sm3_by_hand = {
        use sm3::Digest;
        sm3::Sm3::new().chain_update(before).chain_update(after).finalize().to_vec()
    };
    assert_eq!(committed, sm3_by_hand, "the digest is not SM3 of the covered bytes");
    let sha256 = {
        use sha2::Digest;
        sha2::Sha256::new().chain_update(before).chain_update(after).finalize().to_vec()
    };
    assert_ne!(committed, sha256, "that is SHA-256");

    // The certificate's key is an SM2 key, said the way X.509 says it: an
    // EC key on the curve 1.2.156.10197.1.301.
    let certificate = match data.certificates.as_ref().and_then(|c| c.0.iter().next()) {
        Some(cms::cert::CertificateChoices::Certificate(c)) => c.clone(),
        _ => panic!("no certificate travelled with the signature"),
    };
    let spki = &certificate.tbs_certificate.subject_public_key_info;
    assert_eq!(spki.algorithm.oid, const_oid::db::rfc5912::ID_EC_PUBLIC_KEY);
    let curve = spki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|p| p.decode_as::<const_oid::ObjectIdentifier>().ok())
        .expect("the key names no curve");
    assert_eq!(curve, const_oid::ObjectIdentifier::new_unwrap("1.2.156.10197.1.301"));
}

/// A signature that names SM2-with-SM3 but whose certificate holds an RSA key
/// cannot be checked — and says so, rather than calling anything invalid. The
/// RSA certificate comes from the frozen RSA-signed fixture, since nothing
/// here can make an RSA identity any more.
#[test]
fn a_signature_under_a_certificate_whose_key_is_not_sm2_is_unreadable_not_invalid() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let rsa_bytes = std::fs::read(harness::fixture_path("rsa-signed.pdf")).expect("fixture");
    let rsa_range = declared_range(&rsa_bytes).expect("range");
    let rsa_data = signed_data_in(&rsa_bytes, &rsa_range);

    // The SM2 signer, with the RSA certificate in place of its own.
    let (bytes, _) = signed("two-column.pdf");
    let range = declared_range(&bytes).expect("range");
    let mut data = signed_data_in(&bytes, &range);
    data.certificates = rsa_data.certificates.clone();
    let mut swapped = bytes.clone();
    reblob(&mut swapped, &range, &blob_from(&data));

    match &verdicts_in(&swapped)[0] {
        Verdict::Unreadable(why) => assert!(why.contains("not an SM2 key"), "{why}"),
        other => panic!("judged rather than declined: {other:?}"),
    }
}

/// **The committed fixture** — the SM2 spike's output. Signed once, and read
/// back on every run without PDFium and without the signing code in the way:
/// if this stops verifying, the check changed, not the file.
#[test]
fn the_committed_sm2_fixture_verifies() {
    let bytes = std::fs::read(harness::fixture_path("sm2-signed.pdf")).expect("fixture");
    let file = File::parse(&bytes).expect("parse");
    let found = validate::check(&file, &bytes).expect("check");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].verdict, Verdict::Unaltered, "{}", found[0].verdict.describe());
    assert_eq!(found[0].signer.as_deref(), Some("CN=Pagify SM2 Test Signer,O=Pagify"));
}

/// **A decoy placeholder earlier in the document must not capture the
/// signature.** The blob's hole and the `/ByteRange` were both found by
/// scanning the whole prepared file; a `/Type /Sig` dictionary planted
/// earlier — with a long zero run and a zeroed range — took both, while
/// Pagify's own signature object was left empty and the person was told the
/// document was signed. Found by audit.
#[test]
fn a_decoy_signature_dictionary_does_not_capture_the_signature() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::pdf::{write_object, Dict, Object};

    let identity = identity();
    let source = std::fs::read(harness::fixture_path("two-column.pdf")).expect("fixture");
    let file = File::parse(&source).expect("parse");

    // The decoy, numbered before the real signature will be: a signature
    // dictionary with a 2 KiB zero run and a ten-digit-wide `/ByteRange` so
    // the old scanner would have patched it as happily as its own.
    let mut decoy = Dict(Vec::new());
    decoy.set(b"Type", Object::Name(b"Sig".to_vec()));
    decoy.set(b"Filter", Object::Name(b"Adobe.PPKLite".to_vec()));
    decoy.set(b"SubFilter", Object::Name(b"adbe.pkcs7.detached".to_vec()));
    decoy.set(
        b"ByteRange",
        Object::Array((0..4).map(|_| Object::Number(b"0000000000".to_vec())).collect()),
    );
    decoy.set(b"Contents", Object::HexString(vec![b'0'; 4096]));
    let mut decoy_body = Vec::new();
    write_object(&mut decoy_body, &Object::Dict(decoy));
    let with_decoy = file
        .rewrite_adding(&[], &[(99, decoy_body)], &Dict(Vec::new()))
        .expect("add the decoy");

    let file = File::parse(&with_decoy).expect("parse");
    let signed_bytes = sign::sign(&file, &identity, &sign::Reason::default()).expect("sign");
    let signed = File::parse(&signed_bytes).expect("the signed file parses");

    let contents = |number: u32| -> Vec<u8> {
        let object = signed.object(number).expect("object");
        match object.as_dict().and_then(|d| d.get(b"Contents")) {
            Some(Object::HexString(raw)) => raw.clone(),
            other => panic!("object {number} holds no hex Contents: {other:?}"),
        }
    };
    let is_signature = |number: u32| -> bool {
        let Ok(object) = signed.object(number) else { return false };
        let Some(dict) = object.as_dict() else { return false };
        matches!(dict.get(b"Type").and_then(Object::as_name), Some(name) if name == b"Sig".as_slice())
    };
    let signature_number = signed
        .numbers()
        .filter(|number| is_signature(*number))
        .max()
        .expect("a signature dictionary");
    assert!(signature_number > 99, "the real signature was not appended after the decoy");

    assert!(
        contents(99).iter().all(|b| *b == b'0'),
        "the decoy captured the signature"
    );
    assert!(
        contents(signature_number).iter().any(|b| *b != b'0'),
        "the real signature object is still empty"
    );

    // And a checker that reads the file agrees about which one is a signature.
    let found = validate::check_with(&signed, &signed_bytes, &pdf_core::pdf::trust::Anchors::none())
        .expect("check");
    assert_eq!(found.len(), 2);
    assert!(
        matches!(found[0].verdict, Verdict::Unreadable(_)),
        "the decoy was judged: {:?}",
        found[0].verdict
    );
    assert_eq!(found[1].verdict, Verdict::Unaltered, "{}", found[1].verdict.describe());
    assert_eq!(found[1].signer.as_deref(), Some("CN=Pagify SM2 Test Signer,O=Pagify"));
}

/// **The splice.** A signature is about a stretch of one exact file. Take a
/// genuinely signed file, split it at the Contents hole, and insert a region
/// carrying a shadow signature dictionary plus a cross-reference table at the
/// offset the unchanged final `startxref` still names: the covered bytes are
/// byte-identical to the original, so the digest and the signature still
/// verify, while the parsed object graph is attacker-chosen. The hole must
/// therefore be this dictionary's own `/Contents` and nothing else — with the
/// binding removed, this test reports `Unaltered` over a document that was
/// changed. Found by audit.
#[test]
fn a_signature_spliced_around_an_inserted_region_is_refused() {
    let (bytes, _) = signed("two-column.pdf");
    let range = declared_range(&bytes).expect("range");
    let (a, b) = (range.hole_at, range.hole_at + range.hole_len);

    // The blob the shadow dictionary will carry, exactly as the original
    // signature carries it.
    let blob = blob_in(&bytes, &range);
    let hex: String = blob.iter().map(|byte| format!("{byte:02X}")).collect();

    // The parser reads the last `startxref` in the file, which lives in the
    // covered tail and cannot be changed. That is what makes the splice work:
    // the offset value is fixed, so the attacker builds to it.
    let at = bytes.windows(9).rposition(|w| w == b"startxref").expect("startxref");
    let target: usize = String::from_utf8_lossy(&bytes[at + 9..])
        .split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .expect("the startxref offset");

    // The inserted region: object 8 (the shadow signature) at its start, a
    // minimal catalogue, padding, then the fake cross-reference table at the
    // absolute offset the trailer still names.
    let catalog = "1 0 obj\n<</Type /Catalog /Pages 2 0 R>>\nendobj\n";
    // The catalogue sits after the signature object; its offset depends on
    // that object's length, which depends on the digits of the hole length,
    // which is fixed by where the xref has to land. One settling pass is
    // enough; a loop keeps it honest if the decimal width ever changes.
    let mut hole_len = (target - a) + 200;
    let mut forged = Vec::new();
    for _ in 0..8 {
        let range_text = format!("[0 {a} {} {}]", a + hole_len, bytes.len() - b);
        let signature_object = format!(
            "8 0 obj\n<</Type /Sig /Contents <{hex}>\n/ByteRange {range_text}>>\nendobj\n"
        );
        let catalog_at = a + signature_object.len();
        let xref = format!(
            "xref\n1 1\n{catalog_at:010} 00000 n \n8 1\n{a:010} 00000 n \n\
             trailer\n<< /Size 9 /Root 1 0 R >>\n"
        );
        let pad = (target - a)
            .checked_sub(signature_object.len() + catalog.len())
            .expect("the fixture's xref table is too close to the hole");
        let length = signature_object.len() + catalog.len() + pad + xref.len();
        if length == hole_len {
            forged.clear();
            forged.extend_from_slice(&bytes[..a]); // C1, untouched
            forged.extend_from_slice(signature_object.as_bytes());
            forged.extend_from_slice(catalog.as_bytes());
            forged.extend(std::iter::repeat_n(b' ', pad));
            forged.extend_from_slice(xref.as_bytes()); // at the named offset
            forged.extend_from_slice(&bytes[b..]); // C2, untouched
            break;
        }
        hole_len = length;
    }
    // The forged file is the original plus the inserted region, which is the
    // hole the shadow dictionary declares — the original Contents hole having
    // been closed up around it.
    assert_eq!(
        forged.len(),
        bytes.len() + hole_len - (b - a),
        "the splice did not settle"
    );

    // The parser follows the attacker's table and finds the shadow signature.
    let parsed = File::parse(&forged).expect("the spliced file parses");
    let found = validate::check_with(&parsed, &forged, &pdf_core::pdf::trust::Anchors::none())
        .expect("check");
    assert_eq!(found.len(), 1, "the shadow signature was not found");
    match &found[0].verdict {
        Verdict::Unreadable(why) => assert!(why.contains("/Contents"), "{why}"),
        other => panic!("a spliced signature was judged: {other:?}"),
    }
}

/// **The fail-closed guard — the test that protects a person from reading
/// "not verified by Pagify" as "forged".** `rsa-signed.pdf` was signed by the
/// RSA path on its last day, and stands in for every document signed in
/// another application. It is reported as not checked, naming the scheme in
/// words — never unaltered, never altered, never invalid, never partly
/// covered — and that stays true when the file is altered under it or
/// appended to, because nothing about it was checked.
#[test]
fn a_document_signed_in_another_scheme_is_not_verified_and_never_judged() {
    let bytes = std::fs::read(harness::fixture_path("rsa-signed.pdf")).expect("fixture");
    let range = declared_range(&bytes).expect("range");

    let mut altered = bytes.clone();
    altered[range.hole_at / 2] ^= 0x20;
    let mut appended = bytes.clone();
    appended.extend_from_slice(b"\n% a revision the signature never covered\n");

    for (what, file) in [("as signed", &bytes), ("altered", &altered), ("appended to", &appended)] {
        let parsed = File::parse(file).expect("parse");
        let found = validate::check(&parsed, file).expect("check");
        assert_eq!(found.len(), 1, "{what}");
        match &found[0].verdict {
            Verdict::Unreadable(why) => {
                assert!(why.contains("RSA-PKCS#1v1.5 / SHA-256"), "{what}: {why}");
                assert!(why.contains("does not verify"), "{what}: {why}");
                assert!(!why.contains("1.2.840.113549"), "{what}: a number, not words: {why}");
            }
            other => panic!("{what}: an RSA signature was judged rather than declined: {other:?}"),
        }
        assert!(found[0].signer.is_none(), "{what}: nobody was checked, yet somebody is named");
        let said = found[0].verdict.describe();
        assert!(!said.contains("CHANGED"), "{what}: {said}");
        assert!(!said.contains("NOT VALID"), "{what}: {said}");
        assert!(!said.contains("never signed"), "{what}: {said}");
    }
}
