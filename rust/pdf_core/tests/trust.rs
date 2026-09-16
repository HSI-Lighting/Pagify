//! Trust — whether the certificate a signature verified under is one to
//! believe — checked the way the audit checks: one adversarial fixture per
//! claim, signed at test time, read back through the same code a reader runs.
//!
//! The identities: `test-root-sm2.der` is a self-signed SM2 root that stands
//! in for HSI's; `test-leaf-sm2.p12` and `test-revoked-leaf-sm2.p12` were
//! issued from it (serials 0x1001 and 0x1002); `test-lookalike-root-sm2.der`
//! carries the root's exact subject name on a different key, and
//! `test-lookalike-leaf-sm2.p12` was issued from *that* with the good leaf's
//! subject, issuer name and serial. `test-signer-sm2.p12` is self-signed and
//! chains to nothing. None of them is pinned into the binary; the tests hand
//! the roots in, which is the point of `check_with`.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test trust
//! ```

mod harness;

use pdf_core::pdf::trust::{Anchors, Trust};
use pdf_core::pdf::validate::{self, Signature, Verdict};
use pdf_core::pdf::{sign, File};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(harness::fixture_path(name)).expect("the fixture is committed")
}

fn identity(p12: &str) -> sign::Identity {
    sign::Identity::from_pkcs12(&fixture(p12), "pagify").expect("read the identity")
}

/// `two-column.pdf`, signed with the named identity.
fn signed_with(p12: &str) -> Vec<u8> {
    let bytes = fixture("two-column.pdf");
    let file = File::parse(&bytes).expect("parse");
    sign::sign(&file, &identity(p12), &sign::Reason::default()).expect("sign")
}

/// The test root pinned, with the given denylist.
fn anchors(denylist: &str) -> Anchors {
    Anchors::new(&fixture("test-root-sm2.der"), denylist).expect("anchors")
}

fn the_one(bytes: &[u8], anchors: &Anchors) -> Signature {
    let file = File::parse(bytes).expect("parse");
    let mut found = validate::check_with(&file, bytes, anchors).expect("check");
    assert_eq!(found.len(), 1, "one signature expected");
    found.remove(0)
}

/// **The happy path**: a leaf the pinned root issued signs a document, and
/// the document reads as unaltered and pinned — the only pair that earns the
/// tick, and `is_good` says so.
#[test]
fn a_leaf_issued_by_the_pinned_root_is_pinned() {
    let bytes = signed_with("test-leaf-sm2.p12");
    let found = the_one(&bytes, &anchors(""));
    assert_eq!(found.verdict, Verdict::Unaltered, "{}", found.verdict.describe());
    assert_eq!(found.trust, Some(Trust::Pinned));
    assert_eq!(found.signer.as_deref(), Some("CN=Pagify Test Leaf (leaf),O=Pagify"));
    assert!(found.is_good());
}

/// **A green tick for anyone with the `sm2` crate** is what this guards
/// against: a self-signed leaf verifies perfectly, and is unrecognised.
#[test]
fn a_self_signed_leaf_not_from_the_root_is_unrecognised() {
    let bytes = signed_with("test-signer-sm2.p12");
    let found = the_one(&bytes, &anchors(""));
    assert_eq!(found.verdict, Verdict::Unaltered);
    assert_eq!(found.trust, Some(Trust::Unrecognised));
    assert!(!found.is_good());
    // The subject is still reported, for a person to decide.
    assert_eq!(found.signer.as_deref(), Some("CN=Pagify SM2 Test Signer,O=Pagify"));
}

/// **A revoked key still trusted** is what this guards against: the same
/// root, a leaf on the denylist by issuer and serial — revoked, and not good.
#[test]
fn a_leaf_on_the_denylist_is_revoked() {
    let bytes = signed_with("test-revoked-leaf-sm2.p12");
    let denied = anchors("1002 CN=Pagify Test Root,O=Pagify");
    let found = the_one(&bytes, &denied);
    assert_eq!(found.verdict, Verdict::Unaltered);
    assert_eq!(found.trust, Some(Trust::Revoked));
    assert!(!found.is_good());

    // The same leaf under a denylist that names a different serial, or the
    // same serial under a different issuer, is not revoked: the key is issuer
    // AND serial.
    assert_eq!(the_one(&bytes, &anchors("1001 CN=Pagify Test Root,O=Pagify")).trust, Some(Trust::Pinned));
    assert_eq!(the_one(&bytes, &anchors("1002 CN=Somebody Else,O=Pagify")).trust, Some(Trust::Pinned));
    // Written with leading zeros, it is the same serial.
    assert_eq!(the_one(&bytes, &anchors("001002 CN=Pagify Test Root,O=Pagify")).trust, Some(Trust::Revoked));
}

/// **Subject-name matching instead of signature checking** is what this
/// guards against: a root that carries HSI's exact name on a different key,
/// and a leaf from it with the good leaf's name, issuer name and serial. The
/// leaf's certificate signature does not verify under the pinned root's key,
/// so it is unrecognised — and being on the denylist by that name and serial
/// does not make it "revoked" either, because it never chained.
#[test]
fn a_leaf_from_a_root_that_only_looks_like_the_pinned_one_is_unrecognised() {
    let bytes = signed_with("test-lookalike-leaf-sm2.p12");
    let found = the_one(&bytes, &anchors(""));
    assert_eq!(found.verdict, Verdict::Unaltered, "the signature itself is sound");
    assert_eq!(found.trust, Some(Trust::Unrecognised));
    assert_eq!(found.signer.as_deref(), Some("CN=Pagify Test Leaf (leaf),O=Pagify"), "the same name as the good leaf");
    assert_eq!(the_one(&bytes, &anchors("1001 CN=Pagify Test Root,O=Pagify")).trust, Some(Trust::Unrecognised));

    // And pinned under its own root, it would be pinned — the roots are what
    // differ, nothing else.
    let lookalike = Anchors::new(&fixture("test-lookalike-root-sm2.der"), "").expect("anchors");
    assert_eq!(the_one(&bytes, &lookalike).trust, Some(Trust::Pinned));
    // Both roots pinned — the overlap during a rotation — and each leaf finds
    // its own.
    let mut both = fixture("test-root-sm2.der");
    both.extend(fixture("test-lookalike-root-sm2.der"));
    let both = Anchors::new(&both, "").expect("anchors");
    assert_eq!(the_one(&bytes, &both).trust, Some(Trust::Pinned));
    assert_eq!(the_one(&signed_with("test-leaf-sm2.p12"), &both).trust, Some(Trust::Pinned));
}

/// **Nothing pinned, nothing trusted**: against the roots compiled into this
/// binary — none, until HSI's root exists — every signature is unrecognised,
/// including one from the test root. The shipped `check` is the one used.
#[test]
fn against_the_shipped_pins_nothing_is_trusted_yet() {
    for p12 in ["test-leaf-sm2.p12", "test-signer-sm2.p12"] {
        let bytes = signed_with(p12);
        let file = File::parse(&bytes).expect("parse");
        let found = validate::check(&file, &bytes).expect("check");
        assert_eq!(found[0].verdict, Verdict::Unaltered);
        assert_eq!(found[0].trust, Some(Trust::Unrecognised), "{p12}");
        assert!(!found[0].is_good());
    }
}

/// **Trust on `Incomplete`, on purpose**: bytes appended after the signed
/// range leave the signature sound over the earlier revision, and who signed
/// that revision is known and reported — with the verdict still saying the
/// document has changed since, and no tick.
#[test]
fn an_appended_document_still_says_who_signed_the_earlier_revision() {
    let mut bytes = signed_with("test-leaf-sm2.p12");
    bytes.extend_from_slice(b"\n% a revision the signature never covered\n");
    let found = the_one(&bytes, &anchors(""));
    assert!(matches!(found.verdict, Verdict::Incomplete { .. }), "{:?}", found.verdict);
    assert_eq!(found.trust, Some(Trust::Pinned));
    assert_eq!(found.signer.as_deref(), Some("CN=Pagify Test Leaf (leaf),O=Pagify"));
    assert!(!found.is_good(), "an appended document earned the tick");

    // But a byte changed *inside* the range, with bytes appended too, is an
    // alteration — nobody signed what is there now.
    let mut altered = bytes.clone();
    altered[100] ^= 0x20;
    let found = the_one(&altered, &anchors(""));
    assert_eq!(found.verdict, Verdict::Altered);
    assert_eq!(found.trust, None);
    assert_eq!(found.signer, None);
}

/// **No signer, no trust.** A verdict that did not verify a signature —
/// altered, invalid, unreadable — carries no trust at all, never
/// `Unrecognised`, because there is nobody to judge.
#[test]
fn a_signature_that_did_not_verify_has_no_trust() {
    let good = signed_with("test-leaf-sm2.p12");
    let mut altered = good.clone();
    altered[100] ^= 0x20;
    let found = the_one(&altered, &anchors(""));
    assert_eq!(found.verdict, Verdict::Altered);
    assert_eq!(found.trust, None);

    let rsa = fixture("rsa-signed.pdf");
    let found = the_one(&rsa, &anchors(""));
    assert!(matches!(found.verdict, Verdict::Unreadable(_)));
    assert_eq!(found.trust, None);
    assert!(!found.is_good());
}

/// The certificate signature check is a real check: the leaf's `tbsCertificate`
/// under the root's key, with the pinned distinguishing ID — the same
/// primitive as a document signature, applied once more. A root whose key is
/// not SM2 pins nothing.
#[test]
fn a_root_whose_key_is_not_sm2_pins_nothing() {
    // The RSA certificate out of the frozen RSA-signed fixture, offered as a
    // root: no leaf can chain to it, and nothing panics.
    use der::Decode;
    let rsa = fixture("rsa-signed.pdf");
    let at = rsa.windows(10).position(|w| w == b"/ByteRange").expect("range");
    let open = rsa[at..].iter().position(|b| *b == b'[').map(|n| at + n).expect("[");
    let close = rsa[open..].iter().position(|b| *b == b']').map(|n| open + n).expect("]");
    let numbers: Vec<usize> = String::from_utf8_lossy(&rsa[open + 1..close])
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    let hole = &rsa[numbers[1] + 1..numbers[2] - 1];
    let blob: Vec<u8> = hole
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    // Cut where the DER header says, not where the padding's zeros begin.
    let length = match blob[1] {
        first if first < 0x80 => 2 + first as usize,
        first => {
            let count = (first & 0x7f) as usize;
            2 + count + blob[2..2 + count].iter().fold(0usize, |n, b| n * 256 + *b as usize)
        }
    };
    let info = cms::content_info::ContentInfo::from_der(&blob[..length]).expect("CMS");
    let data: cms::signed_data::SignedData = info.content.decode_as().expect("SignedData");
    let certificate = match data.certificates.as_ref().and_then(|c| c.0.iter().next()) {
        Some(cms::cert::CertificateChoices::Certificate(c)) => c.clone(),
        _ => panic!("no certificate"),
    };
    use der::Encode;
    let root = Anchors::new(&certificate.to_der().expect("DER"), "").expect("an RSA root parses");
    assert_eq!(root.root_count(), 1);
    let bytes = signed_with("test-leaf-sm2.p12");
    assert_eq!(the_one(&bytes, &root).trust, Some(Trust::Unrecognised));
}
