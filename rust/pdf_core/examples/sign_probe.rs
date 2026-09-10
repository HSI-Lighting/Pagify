//! Sign a document, then check the signature the hard way.
//!
//! Four questions, and only the last two are interesting:
//!   1. does the file still open?
//!   2. does a reader see a signature in it?
//!   3. does the digest in the blob match the bytes the range covers?
//!   4. does the signature verify under the certificate's public key?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example sign_probe -- <file.pdf> [out.pdf]
//! ```

use pdf_core::document::Document;
use pdf_core::pdf::{sign, File};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let p12 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer.p12");

    let identity = sign::Identity::from_pkcs12(&std::fs::read(p12).expect("read"), "pagify")
        .expect("the test certificate");
    println!("signing as: {}", identity.subject().unwrap_or_default());

    let bytes = std::fs::read(&path).expect("read");
    let file = File::parse(&bytes).expect("parse");
    let signed = sign::sign(
        &file,
        &identity,
        &sign::Reason { reason: "Approved".into(), location: "London".into(), ..Default::default() },
    )
    .expect("sign");
    println!("{} bytes in, {} bytes out", bytes.len(), signed.len());
    if let Some(out) = std::env::args().nth(2) {
        std::fs::write(&out, &signed).expect("write");
        println!("written to {out}");
    }

    // 1. Still a document.
    match pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(signed.clone(), None) {
        Ok(doc) => {
            let text = doc.page(0).and_then(|p| p.characters()).map(|c| c.text).unwrap_or_default();
            println!("  opens              : {} page(s), {:?}", doc.page_count(),
                text.chars().take(32).collect::<String>());
        }
        Err(e) => println!("  opens              : FAILED ({e})"),
    }

    // 2. A reader sees the signature.
    println!("  PDFium sees        : {} signature(s)", count_signatures(&signed));

    // 3. The digest in the blob is the digest of the file.
    //
    // Read from `/ByteRange` in the finished file, which is what a reader does
    // — `find_placeholder` only works before the hole is filled.
    let range = range_from_file(&signed).expect("no /ByteRange in the signed file");
    println!("  range covers all   : {}", range.covers_everything());
    let digest = sign::digest_of(&signed, &range).expect("digest");

    // 4. The signature verifies under the certificate's own key.
    println!("  verifies           : {}", verify(&signed, &range, &digest, &identity));
}

fn count_signatures(bytes: &[u8]) -> i32 {
    use pdf_core::document::pdfium_doc::PdfiumDocument;
    let Ok(doc) = PdfiumDocument::open_bytes(bytes.to_vec(), None) else { return -1 };
    doc.signature_count()
}

/// Pull the blob back out of the file and check it against the digest.
fn verify(
    signed: &[u8],
    range: &sign::ByteRange,
    digest: &[u8],
    identity: &sign::Identity,
) -> String {
    use der::{Decode, Encode};

    // The hex between the angle brackets, trailing zeros trimmed.
    let hex = &signed[range.hole_at + 1..range.hole_at + range.hole_len - 1];
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

    let Ok(info) = cms::content_info::ContentInfo::from_der(&raw) else {
        return "the blob is not CMS".into();
    };
    let Ok(data): Result<cms::signed_data::SignedData, _> = info.content.decode_as() else {
        return "the CMS is not SignedData".into();
    };
    let Some(signer) = data.signer_infos.0.as_ref().first() else {
        return "no signer in the blob".into();
    };

    // The message digest the signer committed to.
    let Some(attributes) = &signer.signed_attrs else {
        return "the signer signed no attributes".into();
    };
    // The attribute's value is a DER OCTET STRING; `value()` is already its
    // content, so nothing is stripped from it.
    let committed = attributes.iter().find_map(|a| {
        (a.oid == const_oid::db::rfc5911::ID_MESSAGE_DIGEST)
            .then(|| a.values.as_ref().first().map(|v| v.value().to_vec()))
            .flatten()
    });
    if committed.as_deref() != Some(digest) {
        return format!("the digest does not match the file ({committed:?})");
    }

    // And the signature itself, over those attributes.
    use rsa::signature::Verifier;
    use x509_cert::Certificate;
    let Ok(certificate) = Certificate::from_der(&identity.certificates[0]) else {
        return "the certificate cannot be read".into();
    };
    // From the DER of the whole key info, which needs no borrow gymnastics.
    let Ok(spki) = certificate.tbs_certificate.subject_public_key_info.to_der() else {
        return "the certificate's key cannot be read".into();
    };
    use rsa::pkcs8::DecodePublicKey;
    let Ok(key) = rsa::RsaPublicKey::from_public_key_der(&spki) else {
        return "the certificate has no RSA key".into();
    };
    let verifying = rsa::pkcs1v15::VerifyingKey::<sha2::Sha256>::new(key);
    let Ok(signed_attrs) = attributes.to_der() else {
        return "the attributes cannot be re-encoded".into();
    };
    let Ok(signature) = rsa::pkcs1v15::Signature::try_from(signer.signature.as_bytes()) else {
        return "the signature is malformed".into();
    };
    match verifying.verify(&signed_attrs, &signature) {
        Ok(()) => "yes — the digest matches and the signature is good".into(),
        Err(e) => format!("NO ({e})"),
    }
}

/// The four numbers, read back out of a signed file.
fn range_from_file(bytes: &[u8]) -> Option<sign::ByteRange> {
    let at = bytes.windows(10).position(|w| w == b"/ByteRange")?;
    let open = bytes[at..].iter().position(|b| *b == b'[').map(|n| at + n)?;
    let close = bytes[open..].iter().position(|b| *b == b']').map(|n| open + n)?;

    let numbers: Vec<usize> = String::from_utf8_lossy(&bytes[open + 1..close])
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    let [_start, first, second_at, _second_len] = numbers[..] else { return None };
    Some(sign::ByteRange {
        hole_at: first,
        hole_len: second_at - first,
        total: bytes.len(),
    })
}
