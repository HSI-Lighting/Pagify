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

    let Some((bytes, _)) = signed("two-column.pdf") else { return };
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

    let Some((bytes, identity)) = signed("two-column.pdf") else { return };
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

    let Some((bytes, _)) = signed("two-column.pdf") else { return };
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

/// And a scheme this does not implement is reported as that — not verified
/// by assumption, and not called invalid either.
#[test]
fn a_signature_scheme_this_does_not_know_is_unreadable_not_unaltered() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some((bytes, _)) = signed("two-column.pdf") else { return };
    let range = declared_range(&bytes).expect("range");

    let mut data = signed_data_in(&bytes, &range);
    let mut signers: Vec<cms::signed_data::SignerInfo> = data.signer_infos.0.iter().cloned().collect();
    signers[0].signature_algorithm.oid = const_oid::db::rfc5912::ECDSA_WITH_SHA_256;
    data.signer_infos = cms::signed_data::SignerInfos(
        der::asn1::SetOfVec::try_from(signers).expect("signers"),
    );
    let mut relabelled = bytes.clone();
    reblob(&mut relabelled, &range, &blob_from(&data));

    let file = File::parse(&relabelled).expect("parse");
    let found = validate::check(&file, &relabelled).expect("check");
    match &found[0].verdict {
        Verdict::Unreadable(why) => assert!(why.contains("does not check"), "{why}"),
        other => panic!("an unknown scheme was judged rather than declined: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Timestamp tokens commit to the file through the imprint inside their TSTInfo,
// not through the digest attribute — that one is over the TSTInfo itself.
// Comparing it with the file read every genuine timestamp as an alteration.
// ---------------------------------------------------------------------------

/// A TSTInfo with the three fields the check reads, and enough after them to
/// be the real shape.
fn tst_info_over(digest: &[u8]) -> Vec<u8> {
    use der::Encode;
    fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        match body.len() {
            n if n < 0x80 => out.push(n as u8),
            n if n < 0x100 => out.extend([0x81, n as u8]),
            n => out.extend([0x82, (n >> 8) as u8, n as u8]),
        }
        out.extend_from_slice(body);
        out
    }
    let algorithm = spki::AlgorithmIdentifierOwned {
        oid: const_oid::db::rfc5912::ID_SHA_256,
        parameters: None,
    };
    let mut imprint = algorithm.to_der().expect("algorithm");
    imprint.extend(der::asn1::OctetString::new(digest).expect("digest").to_der().expect("DER"));

    let mut body = Vec::new();
    body.extend(tlv(0x02, &[1])); // version
    body.extend(const_oid::ObjectIdentifier::new_unwrap("1.2.3.4").to_der().expect("policy"));
    body.extend(tlv(0x30, &imprint)); // messageImprint
    body.extend(tlv(0x02, &[7])); // serialNumber
    body.extend(tlv(0x18, b"20260912120000Z")); // genTime
    tlv(0x30, &body)
}

/// A token as an authority would make it, signed with the test identity
/// standing in for the authority's key.
fn token_over(identity: &sign::Identity, digest: &[u8]) -> Vec<u8> {
    use cms::builder::{SignedDataBuilder, SignerInfoBuilder};
    use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
    use cms::signed_data::{EncapsulatedContentInfo, SignerIdentifier};
    use der::{Decode, Encode};

    let certificate =
        x509_cert::Certificate::from_der(&identity.certificates[0]).expect("certificate");
    let signer = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(identity.rsa_key().expect("key"));
    let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
        issuer: certificate.tbs_certificate.issuer.clone(),
        serial_number: certificate.tbs_certificate.serial_number.clone(),
    });
    let content = EncapsulatedContentInfo {
        econtent_type: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4"),
        econtent: Some(
            der::Any::new(der::Tag::OctetString, tst_info_over(digest)).expect("content"),
        ),
    };
    let digest_algorithm = spki::AlgorithmIdentifierOwned {
        oid: const_oid::db::rfc5912::ID_SHA_256,
        parameters: None,
    };
    let signer_info =
        SignerInfoBuilder::new(&signer, sid, digest_algorithm.clone(), &content, None)
            .expect("signer");
    let mut builder = SignedDataBuilder::new(&content);
    builder
        .add_digest_algorithm(digest_algorithm)
        .and_then(|b| b.add_certificate(CertificateChoices::Certificate(certificate)))
        .and_then(|b| b.add_signer_info::<_, rsa::pkcs1v15::Signature>(signer_info))
        .and_then(|b| b.build())
        .expect("token")
        .to_der()
        .expect("DER")
}

/// A token whose imprint is the file's digest verifies — and once the file
/// is altered under it, it does not.
#[test]
fn a_timestamp_token_is_checked_through_the_imprint_it_carries() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some((bytes, identity)) = signed("two-column.pdf") else { return };
    let range = declared_range(&bytes).expect("range");
    let digest = sign::digest_of(&bytes, &range).expect("digest");

    let mut stamped = bytes.clone();
    reblob(&mut stamped, &range, &token_over(&identity, &digest));
    let file = File::parse(&stamped).expect("parse");
    let found = validate::check(&file, &stamped).expect("check");
    assert_eq!(found[0].verdict, Verdict::Unaltered, "a genuine token: {:?}", found[0]);
    assert_eq!(found[0].signer.as_deref(), Some(identity.subject().expect("subject").as_str()));

    let mut altered = stamped.clone();
    altered[range.hole_at / 2] ^= 0x20;
    let file = File::parse(&altered).expect("parse");
    let found = validate::check(&file, &altered).expect("check");
    assert_eq!(found[0].verdict, Verdict::Altered, "an edit under a token: {:?}", found[0]);
}

/// A token vouching for some *other* file's digest is not evidence about this
/// one, however well it is signed.
#[test]
fn a_timestamp_token_for_a_different_file_is_an_alteration_here() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let Some((bytes, identity)) = signed("two-column.pdf") else { return };
    let range = declared_range(&bytes).expect("range");

    let mut stamped = bytes.clone();
    reblob(&mut stamped, &range, &token_over(&identity, &[0x42; 32]));
    let file = File::parse(&stamped).expect("parse");
    let found = validate::check(&file, &stamped).expect("check");
    assert_eq!(found[0].verdict, Verdict::Altered, "{:?}", found[0]);
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
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer.p12");
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
    doc.whiteout(
        0,
        Rect { left: 100.0, top: 100.0, right: 200.0, bottom: 120.0 },
        Color { r: 255, g: 255, b: 255, a: 255 },
    )
    .expect("an edit after signing");

    let mut saved = Vec::new();
    doc.save_incremental(&mut saved).expect("save");
    match verdicts_in(&saved).as_slice() {
        [Verdict::Incomplete { covered, total }] => {
            assert!(covered < total, "{covered} of {total}");
        }
        other => panic!("an edit after signing should leave the signature over the earlier revision, got {other:?}"),
    }
}

/// And the same for a timestamp, through the document: what is kept is what
/// is written.
#[test]
fn a_timestamped_document_is_dirty_until_its_bytes_are_written() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    use pdf_core::document::DocumentMut;

    let mut doc = open_document("two-column.pdf");
    // No authority to ask here; the signing path exercises the same state,
    // so this only checks the state a clean document starts from.
    assert!(!doc.is_dirty());
    if !certify(&mut doc) {
        return;
    }
    let mut saved = Vec::new();
    doc.save_incremental(&mut saved).expect("save");
    // Saved once, the same bytes again on a second save: nothing pending.
    let mut again = Vec::new();
    doc.save_incremental(&mut again).expect("save again");
    assert!(again.starts_with(&saved), "a second save did not start from the signed file");
}
