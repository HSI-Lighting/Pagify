//! Checking the signatures a document already carries.
//!
//! # What this answers, and what it does not
//!
//! It answers two questions, and the second is what makes the first worth
//! asking:
//!
//! 1. **Has this file changed since it was signed?** Recompute the digest over
//!    the bytes the signature says it covers and compare it with what the
//!    signer committed to.
//! 2. **Did the key in the certificate the signature carries make it?** The
//!    signer commits to the digest inside a set of signed attributes, and
//!    signs *those*. Without checking that signature, the digest is only a
//!    number the blob says about itself — anyone can edit the file, recompute
//!    the digest and write it back in, with no key at all. Found by audit:
//!    that is exactly what the check used to accept as "unchanged".
//!
//! Whether the signer is who they claim to be is a third question, answered
//! beside the verdict rather than inside it: [`Signature::trust`] says whether
//! the certificate the signature verified under was issued by a root Pagify is
//! built to believe — see [`super::trust`]. A document signed with a
//! certificate made five minutes ago verifies perfectly and means nothing at
//! all; it verifies under *that* certificate, whose subject is reported and
//! whose trust is reported separately, so that a person can tell the two
//! apart. The two axes stay orthogonal: the verdict is about the bytes, the
//! trust is about the signer, and the green tick needs both.
//!
//! Saying "valid" without that distinction is how a green tick comes to mean
//! less than nothing, so every verdict here carries it.
//!
//! # One scheme: SM2 over SM3
//!
//! Pagify verifies the signatures Pagify makes — SM2 over SM3, under the
//! conventions in [`super::sm`] — and no others. A document signed in another
//! application opens, reads and prints exactly as it always did; its signature
//! is reported as *not checked*, naming the scheme in words, and nothing else
//! is said about it. That is not the same as invalid, and it is never reported
//! as though it were: a verdict about the bytes — altered, only partly
//! covered — is earned by a signature this can check, and for any other it
//! would be a judgement made with no evidence, in either direction. So the
//! scheme is looked at before the range, before the digest, before anything.
//!
//! # The check that catches the other real attack
//!
//! A signature covers a **range**, not a file. The classic way to alter a
//! signed document is to append to it: everything the range names is untouched,
//! the digest still matches, and the new content was never covered. So, for a
//! signature this checks, the range is checked against the length of the file
//! before the digest is, and a signature that does not reach the end is
//! reported as exactly that.
//!
//! # Fail closed
//!
//! Anything this cannot check — a scheme it does not verify, a certificate
//! whose key is not SM2, no certificate at all — is reported as *unreadable*,
//! never as unaltered. The green tick is only ever earned.

use crate::error::Result;

use super::object::Lexer;
use super::trust::{self, Anchors, Trust};
use super::{File, Object};

/// What became of one signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The bytes are as they were signed, and the signature verifies under
    /// the certificate it carries.
    ///
    /// **Says nothing about whether that certificate is to be trusted.**
    Unaltered,
    /// The digest does not match: the document changed after it was signed.
    Altered,
    /// The digest matches but the signature does not verify under the
    /// certificate it carries — the blob vouches for the bytes, and nothing
    /// vouches for the blob. A document altered and re-hashed looks exactly
    /// like this.
    Invalid(String),
    /// The signature is sound over the revision it was made on — the first
    /// `covered` bytes of the file — and the file is longer than that now.
    /// Whatever follows was never signed: a later revision, or the append.
    Incomplete { covered: usize, total: usize },
    /// It could not be checked — an algorithm this does not implement, or a
    /// blob it cannot read. **Not the same as invalid**, and never reported as
    /// though it were.
    Unreadable(String),
}

impl Verdict {
    pub fn describe(&self) -> String {
        match self {
            Verdict::Unaltered => "unchanged since it was signed, and the signature is the \
                                   certificate's (whether to trust that certificate is said \
                                   separately)"
                .into(),
            Verdict::Altered => "CHANGED since it was signed".into(),
            Verdict::Invalid(why) => format!("NOT VALID — {why}"),
            Verdict::Incomplete { covered, total } => format!(
                "sound over the first {covered} of {total} bytes — the document was changed \
                 after it was signed, and what came after was never signed"
            ),
            Verdict::Unreadable(why) => format!("could not be checked: {why}"),
        }
    }
}

/// One signature found in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// What the dictionary says the signer is called, if anything.
    ///
    /// **A label, not a finding.** It is plain text in the file, written by
    /// whoever wrote the file. The name that means something is `signer`.
    pub name: String,
    /// When it says it was made — the signer's own clock, nobody else's.
    pub when: String,
    pub verdict: Verdict,
    /// The subject of the certificate the signature verified under — only
    /// when it did. Who *holds* that certificate is what `trust` answers.
    pub signer: Option<String>,
    /// Whether that certificate was issued by a root Pagify trusts. Set
    /// whenever the signature verified over its own range — `Unaltered` or
    /// `Incomplete` — and never otherwise: a signature that did not verify
    /// has no signer to judge.
    ///
    /// `Unaltered` + `Pinned` is the only pair that earns a full green tick.
    /// `Incomplete` + `Pinned` says who signed the earlier revision, and that
    /// the document has changed since.
    pub trust: Option<Trust>,
}

impl Signature {
    /// The one combination that means "this document, from this signer":
    /// unchanged since it was signed, and signed by someone a pinned root
    /// vouches for.
    pub fn is_good(&self) -> bool {
        self.verdict == Verdict::Unaltered && self.trust == Some(Trust::Pinned)
    }
}

/// Check every signature in a document, against the roots compiled into this
/// binary.
///
/// An empty list means the document carries none — which is not a failure and
/// must not be reported as one.
pub fn check(file: &File<'_>, bytes: &[u8]) -> Result<Vec<Signature>> {
    check_with(file, bytes, Anchors::pinned())
}

/// The same check against roots of the caller's choosing — how the tests
/// judge against a test root without one being pinned into the binary.
pub fn check_with(file: &File<'_>, bytes: &[u8], anchors: &Anchors) -> Result<Vec<Signature>> {
    let mut out = Vec::new();
    for number in file.numbers().collect::<Vec<_>>() {
        let Ok(object) = file.object(number) else { continue };
        let Some(dict) = object.as_dict() else { continue };
        if dict.get(b"Type").and_then(Object::as_name) != Some(&b"Sig"[..]) {
            continue;
        }

        // **The hole must be this dictionary's own `/Contents` and nothing
        // else**, checked before any of `verdict_for`'s crypto — see
        // [`contents_span`]. Kept out of `verdict_for` itself, which stays
        // pure crypto over a byte range with no file to locate anything in
        // (its own unit tests hand it a `Dict` and a slice with no
        // cross-reference table behind either).
        let Checked { verdict, signer, trust } = match range_numbers(dict) {
            Some([_, first, second_at, _]) if contents_span(file, number, bytes) == Some(first..second_at) => {
                verdict_for(dict, bytes, anchors)
            }
            Some(_) => Checked::failed(Verdict::Unreadable(
                "its byte range does not point at this signature's own contents".into(),
            )),
            None => Checked::failed(Verdict::Unreadable("it declares no byte range".into())),
        };
        out.push(Signature {
            name: text_of(dict.get(b"Name")),
            when: text_of(dict.get(b"M")),
            verdict,
            signer,
            trust,
        });
    }
    Ok(out)
}

/// What was learned about one signature.
struct Checked {
    verdict: Verdict,
    /// Set only when the signature verified: the subject of the certificate
    /// whose key made it.
    signer: Option<String>,
    /// And what the anchors say about that certificate.
    trust: Option<Trust>,
}

impl Checked {
    fn failed(verdict: Verdict) -> Self {
        Checked { verdict, signer: None, trust: None }
    }
}

/// `id-ct-TSTInfo`: the content type of a timestamp token, whose content is
/// the `TSTInfo` an authority signed. Not in the OID database this uses.
///
/// Recognised only to be declined. A token commits to the file through the
/// imprint *inside* its `TSTInfo`, not through the digest attribute — that
/// one is over the `TSTInfo` itself — and reading it as an ordinary
/// signature reports every genuine timestamp as an alteration. Found by
/// audit, when tokens were still checked; now that nothing here asks an
/// authority for one, a token is a third party's statement in a third
/// party's scheme, and it is reported as not checked rather than judged.
const ID_CT_TST_INFO: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");

/// What became of one signature dictionary.
fn verdict_for(dict: &super::Dict, bytes: &[u8], anchors: &Anchors) -> Checked {
    use der::Decode;

    // -- the blob, and whether it is one this checks at all -------------------
    //
    // Before the range, before the digest: a signature in a scheme this does
    // not verify is declined here with nothing computed about the document,
    // so that "not verified by Pagify" can never come out as "altered" or
    // "only partly covered". Those verdicts are earned by a signature this
    // *can* check; for any other they would be a judgement made with no
    // evidence, in either direction.
    let Some(blob) = hex_of(dict.get(b"Contents")) else {
        return Checked::failed(Verdict::Unreadable("it holds no signature".into()));
    };
    let Ok(info) = cms::content_info::ContentInfo::from_der(&blob) else {
        return Checked::failed(Verdict::Unreadable("its signature is not a CMS structure".into()));
    };
    let Ok(data) = info.content.decode_as::<cms::signed_data::SignedData>() else {
        return Checked::failed(Verdict::Unreadable("its signature is not SignedData".into()));
    };
    let Some(signer) = data.signer_infos.0.as_ref().first() else {
        return Checked::failed(Verdict::Unreadable("its signature names no signer".into()));
    };
    if data.encap_content_info.econtent_type == ID_CT_TST_INFO {
        return Checked::failed(Verdict::Unreadable(
            "it is a document timestamp from a time authority, which this does not check".into(),
        ));
    }
    let certificates = certificates_in(&data);
    if signer.signature_algorithm.oid != super::sm::ID_SM2_WITH_SM3
        || signer.digest_alg.oid != super::sm::ID_SM3
    {
        // Named in words, so that the line a person reads says what the
        // document carries rather than a number — and never that anything
        // is wrong with it.
        return Checked::failed(Verdict::Unreadable(format!(
            "it carries a signature in {}, which Pagify does not verify — only SM2 over SM3",
            words::scheme(
                &signer.signature_algorithm.oid,
                &signer.digest_alg.oid,
                certificates.first()
            )
        )));
    }
    let Some(attributes) = &signer.signed_attrs else {
        return Checked::failed(Verdict::Unreadable(
            "its signer committed to no attributes".into(),
        ));
    };
    let committed = attributes
        .iter()
        .find(|a| a.oid == const_oid::db::rfc5911::ID_MESSAGE_DIGEST)
        .and_then(|a| a.values.as_ref().first().map(|v| v.value().to_vec()));
    let Some(committed) = committed else {
        return Checked::failed(Verdict::Unreadable("its signer committed to no digest".into()));
    };

    // -- the range ------------------------------------------------------------
    let Some(numbers) = range_numbers(dict) else {
        return Checked::failed(Verdict::Unreadable("it declares no byte range".into()));
    };
    let [_, first, second_at, second_len] = numbers;
    // Checked, because the numbers are the file's: `[0 0 1e308 n]` wrapped
    // past the guard below and panicked on the slice. Found by audit.
    let Some(reaches) = second_at.checked_add(second_len) else {
        return Checked::failed(Verdict::Unreadable("its byte range does not add up".into()));
    };
    if reaches > bytes.len() || first > bytes.len() || first > second_at {
        return Checked::failed(Verdict::Unreadable("its byte range runs past the file".into()));
    }
    let signed_bytes = [&bytes[..first], &bytes[second_at..second_at + second_len]];

    // -- what the signer committed to, against the file ----------------------
    if sm3_over(&signed_bytes) != committed {
        return Checked::failed(Verdict::Altered);
    }

    // -- whether anything vouches for what was committed to ------------------
    let certificate = match verify(signer, attributes, &certificates) {
        Ok(certificate) => certificate,
        Err(verdict) => return Checked::failed(verdict),
    };
    let signer = Some(certificate.tbs_certificate.subject.to_string());
    let trust = Some(trust::trust_in(certificate, anchors));

    // -- and whether it vouches for the whole file ---------------------------
    //
    // **The append.** Everything the range names is as it was signed, under
    // a key that vouches for it — and the file is longer than the signature
    // ever claimed. Who signed the earlier revision is known and reported;
    // what came after it is not covered by anything.
    if reaches != bytes.len() {
        let verdict = Verdict::Incomplete { covered: reaches, total: bytes.len() };
        return Checked { verdict, signer, trust };
    }
    Checked { verdict: Verdict::Unaltered, signer, trust }
}

/// The exact byte span of one object's `/Contents` value, found by walking
/// from the cross-reference table's own offset for it — never by trusting
/// anything the parsed dictionary says about itself, which is precisely
/// what a forged dictionary controls.
///
/// Re-lexes rather than reusing the already-parsed [`super::Dict`]: a
/// [`super::Object`] carries no byte position once parsed, so getting one
/// means walking the bytes again, the same way [`File::object`] does for
/// its own header check — just keeping the span of the one value this
/// needs instead of throwing it away.
fn contents_span(file: &File<'_>, number: u32, bytes: &[u8]) -> Option<std::ops::Range<usize>> {
    let at = file.offset(number)?;
    let mut lexer = Lexer::new(bytes, at);

    // `n g obj` — refused rather than assumed, so a table entry that has
    // drifted is caught here instead of misreading whatever sits at `at` as
    // this object's dictionary.
    let found: u32 = std::str::from_utf8(lexer.token()).ok()?.parse().ok()?;
    if found != number {
        return None;
    }
    let _generation = lexer.token();
    lexer.expect(b"obj").ok()?;
    lexer.skip_space();
    super::object::dict_value_span(bytes, lexer.at, b"Contents")
}

/// SM3 over several stretches of bytes taken as one.
fn sm3_over(parts: &[&[u8]]) -> Vec<u8> {
    use sm3::Digest;
    let mut hasher = sm3::Sm3::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().to_vec()
}

/// Every certificate the signature travels with.
fn certificates_in(data: &cms::signed_data::SignedData) -> Vec<x509_cert::Certificate> {
    data.certificates
        .as_ref()
        .map(|set| {
            set.0
                .iter()
                .filter_map(|choice| match choice {
                    cms::cert::CertificateChoices::Certificate(c) => Some(c.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether the certificate is the one the signer names.
fn is_named(certificate: &x509_cert::Certificate, sid: &cms::signed_data::SignerIdentifier) -> bool {
    use cms::signed_data::SignerIdentifier;
    match sid {
        SignerIdentifier::IssuerAndSerialNumber(named) => {
            certificate.tbs_certificate.issuer == named.issuer
                && certificate.tbs_certificate.serial_number == named.serial_number
        }
        SignerIdentifier::SubjectKeyIdentifier(named) => {
            use der::Decode;
            certificate
                .tbs_certificate
                .extensions
                .iter()
                .flatten()
                .filter(|e| e.extn_id == const_oid::db::rfc5280::ID_CE_SUBJECT_KEY_IDENTIFIER)
                .any(|e| {
                    x509_cert::ext::pkix::SubjectKeyIdentifier::from_der(e.extn_value.as_bytes())
                        .map(|found| found.0.as_bytes() == named.0.as_bytes())
                        .unwrap_or(false)
                })
        }
    }
}

/// Check the SM2 signature over the signed attributes against the
/// certificates it carries, and return the one it verified under.
///
/// The certificate the signer names is tried first; if it is not there or
/// does not fit, every other carried certificate is tried, because the point
/// is that *some* key that travelled with the signature made it — which one
/// is reported, and whether to believe it is [`super::trust`]'s question.
fn verify<'c>(
    signer: &cms::signed_data::SignerInfo,
    attributes: &cms::signed_data::SignedAttributes,
    certificates: &'c [x509_cert::Certificate],
) -> std::result::Result<&'c x509_cert::Certificate, Verdict> {
    use der::Encode;
    use sm2::dsa::signature::Verifier;

    if certificates.is_empty() {
        return Err(Verdict::Unreadable("it carries no certificate to check against".into()));
    }
    // Signed attributes are signed in their DER form as a SET — the
    // `[0] IMPLICIT` tag they wear inside the SignerInfo is not what was signed.
    let message = attributes
        .to_der()
        .map_err(|_| Verdict::Unreadable("its signed attributes cannot be re-encoded".into()))?;
    // The signature value is `SEQUENCE { r, s }`; one that is not is not a
    // signature this made, and does not verify under anything.
    let signature = super::sm::signature_from_der(signer.signature.as_bytes());

    let mut ordered: Vec<&x509_cert::Certificate> =
        certificates.iter().filter(|c| is_named(c, &signer.sid)).collect();
    ordered.extend(certificates.iter().filter(|c| !is_named(c, &signer.sid)));

    let mut tried = false;
    let mut unusable = None;
    for certificate in ordered {
        let key = match sm2_key_of(certificate) {
            Ok(key) => key,
            Err(why) => {
                unusable.get_or_insert(why);
                continue;
            }
        };
        tried = true;
        if let Some(signature) = &signature {
            if key.verify(&message, signature).is_ok() {
                return Ok(certificate);
            }
        }
    }
    match unusable {
        // Nothing could even be tried: say what stood in the way rather than
        // calling a signature invalid that was never checked.
        Some(why) if !tried => Err(Verdict::Unreadable(why)),
        _ => Err(Verdict::Invalid(
            "the signature is not the certificate's — the digest matches, but nothing that \
             travelled with the signature made it"
                .into(),
        )),
    }
}

/// The SM2 public key in a certificate, bound to the pinned distinguishing
/// ID, or why it is not one this can use.
///
/// The `sm2` crate reads the key info itself and refuses a key on any other
/// curve, so an ECDSA P-256 certificate is "not an SM2 key" here rather than
/// a signature that fails.
pub(super) fn sm2_key_of(
    certificate: &x509_cert::Certificate,
) -> std::result::Result<sm2::dsa::VerifyingKey, String> {
    use der::Encode;
    use sm2::pkcs8::DecodePublicKey;
    let spki = &certificate.tbs_certificate.subject_public_key_info;
    let der = spki.to_der().map_err(|_| "its certificate's key cannot be read".to_string())?;
    let key = sm2::PublicKey::from_public_key_der(&der).map_err(|_| {
        format!("its certificate's key is {}, not an SM2 key", words::key(certificate))
    })?;
    sm2::dsa::VerifyingKey::new(super::sm::DISTINGUISHING_ID, key)
        .map_err(|_| "its certificate's key cannot be read".to_string())
}

/// Algorithm identifiers as a person would name them.
///
/// A verdict that says `1.2.840.113549.1.1.11` tells nobody anything; one that
/// says `RSA-PKCS#1v1.5 / SHA-256` tells them what the document carries and
/// that it is not what Pagify signs with. The identifiers here are the ones
/// a document is likely to arrive with; anything else falls back to the
/// number, which is still a fact.
mod words {
    use const_oid::ObjectIdentifier;

    /// `RSA-PKCS#1v1.5 / SHA-256`, `ECDSA P-256 / SHA-256`, `Ed25519`,
    /// `1.2.3.4 / SHA-256`: the signature scheme and the digest it was made
    /// over, from the two identifiers a `SignerInfo` carries and, for a
    /// curve, the certificate.
    pub fn scheme(
        scheme: &ObjectIdentifier,
        digest_oid: &ObjectIdentifier,
        certificate: Option<&x509_cert::Certificate>,
    ) -> String {
        use const_oid::db::rfc5912::*;
        use const_oid::db::rfc8410::{ID_ED_25519, ID_ED_448};
        let digest_words = digest(digest_oid);
        match *scheme {
            RSA_ENCRYPTION => format!("RSA-PKCS#1v1.5 / {digest_words}"),
            SHA_1_WITH_RSA_ENCRYPTION => "RSA-PKCS#1v1.5 / SHA-1".into(),
            SHA_224_WITH_RSA_ENCRYPTION => "RSA-PKCS#1v1.5 / SHA-224".into(),
            SHA_256_WITH_RSA_ENCRYPTION => "RSA-PKCS#1v1.5 / SHA-256".into(),
            SHA_384_WITH_RSA_ENCRYPTION => "RSA-PKCS#1v1.5 / SHA-384".into(),
            SHA_512_WITH_RSA_ENCRYPTION => "RSA-PKCS#1v1.5 / SHA-512".into(),
            ID_RSASSA_PSS => format!("RSA-PSS / {digest_words}"),
            // ECDSA names its digest in the scheme and its curve in the key.
            ID_EC_PUBLIC_KEY => format!("ECDSA{} / {digest_words}", curve(certificate)),
            ECDSA_WITH_SHA_224 => format!("ECDSA{} / SHA-224", curve(certificate)),
            ECDSA_WITH_SHA_256 => format!("ECDSA{} / SHA-256", curve(certificate)),
            ECDSA_WITH_SHA_384 => format!("ECDSA{} / SHA-384", curve(certificate)),
            ECDSA_WITH_SHA_512 => format!("ECDSA{} / SHA-512", curve(certificate)),
            DSA_WITH_SHA_224 => "DSA / SHA-224".into(),
            DSA_WITH_SHA_256 => "DSA / SHA-256".into(),
            ID_ED_25519 => "Ed25519".into(),
            ID_ED_448 => "Ed448".into(),
            super::super::sm::ID_SM2_WITH_SM3 => format!("SM2 / {digest_words}"),
            other => format!("{other} / {digest_words}"),
        }
    }

    /// The key in a certificate: `RSA`, `EC P-256`, `EC SM2`, `Ed25519`, or
    /// the identifier.
    pub fn key(certificate: &x509_cert::Certificate) -> String {
        use const_oid::db::rfc5912::*;
        use const_oid::db::rfc8410::{ID_ED_25519, ID_ED_448};
        match certificate.tbs_certificate.subject_public_key_info.algorithm.oid {
            RSA_ENCRYPTION => "RSA".into(),
            ID_RSASSA_PSS => "RSA-PSS".into(),
            ID_EC_PUBLIC_KEY => format!("EC{}", curve(Some(certificate))),
            ID_ED_25519 => "Ed25519".into(),
            ID_ED_448 => "Ed448".into(),
            ID_DSA => "DSA".into(),
            other => other.to_string(),
        }
    }

    fn digest(oid: &ObjectIdentifier) -> String {
        use const_oid::db::rfc5912::{ID_SHA_224, ID_SHA_256, ID_SHA_384, ID_SHA_512};
        const MD5: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.2.5");
        const SHA_1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
        match *oid {
            ID_SHA_224 => "SHA-224".into(),
            ID_SHA_256 => "SHA-256".into(),
            ID_SHA_384 => "SHA-384".into(),
            ID_SHA_512 => "SHA-512".into(),
            SHA_1 => "SHA-1".into(),
            MD5 => "MD5".into(),
            super::super::sm::ID_SM3 => "SM3".into(),
            other => other.to_string(),
        }
    }

    /// ` P-256`, ` P-384`, ` P-521`, ` SM2`, ` <oid>`, or nothing when the
    /// key names no curve — leading space included, so it slots after `EC`
    /// or `ECDSA`.
    fn curve(certificate: Option<&x509_cert::Certificate>) -> String {
        use const_oid::db::rfc5912::{SECP_256_R_1, SECP_384_R_1, SECP_521_R_1};
        let Some(certificate) = certificate else { return String::new() };
        let spki = &certificate.tbs_certificate.subject_public_key_info;
        let Some(curve) = spki
            .algorithm
            .parameters
            .as_ref()
            .and_then(|p| p.decode_as::<ObjectIdentifier>().ok())
        else {
            return String::new();
        };
        match curve {
            SECP_256_R_1 => " P-256".into(),
            SECP_384_R_1 => " P-384".into(),
            SECP_521_R_1 => " P-521".into(),
            c if c == sm2::Sm2::OID => " SM2".into(),
            other => format!(" {other}"),
        }
    }

    // `sm2::Sm2::OID` needs the trait in scope.
    use sm2::pkcs8::AssociatedOid as _;
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
    // it says it does, and the padding is not part of it. **Where it says it
    // does** — not where the zeros start. Stripping trailing zero bytes cut a
    // signature whose last byte happened to be zero, one in 256, and the
    // document then read as "not a CMS structure". So the outer length is
    // read from the DER header and the blob cut there; the strip is only for a
    // blob whose header cannot be read.
    match der_length(&bytes) {
        Some(length) if length <= bytes.len() => bytes.truncate(length),
        _ => {
            while bytes.last() == Some(&0) {
                bytes.pop();
            }
        }
    }
    Some(bytes)
}

/// How many bytes the DER structure at the start of `bytes` occupies, header
/// included, read from its own length field.
fn der_length(bytes: &[u8]) -> Option<usize> {
    let first = *bytes.get(1)?;
    if first < 0x80 {
        return Some(2 + first as usize);
    }
    let count = (first & 0x7f) as usize;
    if count == 0 || count > 4 || bytes.len() < 2 + count {
        return None;
    }
    let mut length = 0usize;
    for byte in &bytes[2..2 + count] {
        length = length.checked_mul(256)?.checked_add(*byte as usize)?;
    }
    (2 + count).checked_add(length)
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

    /// **A blob ends where its DER header says, not where the padding's
    /// zeros begin.** One signature in 256 ends in a zero byte, and stripping
    /// zeros cut it short.
    #[test]
    fn a_blob_ending_in_a_zero_byte_is_not_cut_short() {
        // A SEQUENCE of three bytes, the last of them zero, then padding.
        let hex = b"3003010200000000".to_vec();
        let got = hex_of(Some(&Object::HexString(hex))).expect("hex");
        assert_eq!(got, vec![0x30, 0x03, 0x01, 0x02, 0x00]);
        // Long-form length: 0x82 then two bytes.
        let mut long = vec![0x30, 0x82, 0x01, 0x00];
        long.extend(vec![0xab; 0x100]);
        long[3 + 0x100] = 0x00;
        let mut padded = long.clone();
        padded.extend([0u8; 16]);
        let hex: Vec<u8> = padded.iter().map(|b| format!("{b:02X}")).collect::<String>().into_bytes();
        assert_eq!(hex_of(Some(&Object::HexString(hex))).expect("hex"), long);
    }

    /// **A byte range that does not add up is unreadable, not a panic.**
    /// Found by audit: `[0 0 1e308 n]` wrapped past the guard and panicked on
    /// the slice.
    #[test]
    fn a_byte_range_that_overflows_is_reported_not_sliced() {
        let bytes = vec![b'x'; 100];
        for range in [
            ["0", "0", "1e308", "101"],
            ["0", "18446744073709551615", "1", "1"],
            ["0", "60", "50", "50"],
        ] {
            let mut dict = super::super::Dict(Vec::new());
            dict.set(b"Type", Object::Name(b"Sig".to_vec()));
            dict.set(
                b"ByteRange",
                Object::Array(range.iter().map(|n| Object::Number(n.as_bytes().to_vec())).collect()),
            );
            dict.set(b"Contents", Object::HexString(b"00".to_vec()));
            let checked = verdict_for(&dict, &bytes, &Anchors::none());
            assert!(
                matches!(checked.verdict, Verdict::Unreadable(_) | Verdict::Incomplete { .. }),
                "{range:?} gave {:?}",
                checked.verdict
            );
        }
    }

    /// **A SignerInfo another implementation built verifies here.** The
    /// blob is OpenSSL's, made with `openssl cms -sign -md sm3 -keyopt
    /// distid:1234567812345678` over the thirteen bytes `hello sm2 cms`,
    /// detached. It proves the check reads the SM suite as it is written by
    /// somebody else — the algorithm identifiers, the `SEQUENCE { r, s }`
    /// signature value, and the distinguishing ID folded into `ZA` — rather
    /// than only what `sign.rs` writes.
    ///
    /// The mirror check, OpenSSL reading ours, is `cargo run --example
    /// sign_probe` plus `openssl pkeyutl -verify` over the extracted signed
    /// attributes; `openssl cms -verify` itself cannot be used, because it
    /// hashes with an empty distinguishing ID and fails its own output.
    #[test]
    fn a_signature_openssl_made_with_the_pinned_identity_verifies_here() {
        let blob = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/openssl-sm2.cms"),
        )
        .expect("the OpenSSL blob is committed");
        let bytes = b"hello sm2 cms".to_vec();
        let hex: Vec<u8> = blob.iter().map(|b| format!("{b:02X}")).collect::<String>().into_bytes();

        let mut dict = super::super::Dict(Vec::new());
        dict.set(b"Type", Object::Name(b"Sig".to_vec()));
        let numbers = ["0", "13", "13", "0"];
        dict.set(
            b"ByteRange",
            Object::Array(numbers.iter().map(|n| Object::Number(n.as_bytes().to_vec())).collect()),
        );
        dict.set(b"Contents", Object::HexString(hex));

        let checked = verdict_for(&dict, &bytes, &Anchors::none());
        assert_eq!(checked.verdict, Verdict::Unaltered, "{}", checked.verdict.describe());
        assert_eq!(checked.signer.as_deref(), Some("CN=Pagify SM2 Test Signer,O=Pagify"));
        assert_eq!(checked.trust, Some(Trust::Unrecognised), "nobody pinned that signer");

        // And not over other bytes, which is the digest check under SM3.
        let mut other = bytes.clone();
        other[0] = b'H';
        assert_eq!(verdict_for(&dict, &other, &Anchors::none()).verdict, Verdict::Altered);
    }

    /// **Identifiers become words**, so that "not verified" names what the
    /// document carries; an identifier this has no word for is shown as the
    /// number, which is still a fact.
    #[test]
    fn schemes_are_named_in_words_with_the_number_as_the_fallback() {
        use const_oid::db::rfc5912::*;
        use const_oid::db::rfc8410::ID_ED_25519;
        let oid = |text: &str| const_oid::ObjectIdentifier::new(text).unwrap();
        let name = |scheme: &const_oid::ObjectIdentifier, digest: &const_oid::ObjectIdentifier| {
            words::scheme(scheme, digest, None)
        };
        // What every RSA-signed PDF in the wild carries, in both spellings.
        assert_eq!(name(&RSA_ENCRYPTION, &ID_SHA_256), "RSA-PKCS#1v1.5 / SHA-256");
        assert_eq!(name(&SHA_256_WITH_RSA_ENCRYPTION, &ID_SHA_256), "RSA-PKCS#1v1.5 / SHA-256");
        assert_eq!(name(&ID_RSASSA_PSS, &ID_SHA_384), "RSA-PSS / SHA-384");
        assert_eq!(name(&ECDSA_WITH_SHA_256, &ID_SHA_256), "ECDSA / SHA-256");
        assert_eq!(name(&ID_ED_25519, &ID_SHA_512), "Ed25519");
        assert_eq!(name(&super::super::sm::ID_SM2_WITH_SM3, &super::super::sm::ID_SM3), "SM2 / SM3");
        assert_eq!(name(&RSA_ENCRYPTION, &oid("1.3.14.3.2.26")), "RSA-PKCS#1v1.5 / SHA-1");
        // Unknown on either side: the number.
        assert_eq!(name(&oid("1.2.3.4"), &ID_SHA_256), "1.2.3.4 / SHA-256");
        assert_eq!(name(&RSA_ENCRYPTION, &oid("1.2.3.5")), "RSA-PKCS#1v1.5 / 1.2.3.5");

        // With a certificate, ECDSA names its curve — and the key of a
        // certificate is named the same way.
        use der::Decode;
        let blob = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/openssl-sm2.cms"),
        )
        .expect("fixture");
        let info = cms::content_info::ContentInfo::from_der(&blob).expect("CMS");
        let data: cms::signed_data::SignedData = info.content.decode_as().expect("SignedData");
        let certificate = certificates_in(&data).remove(0);
        assert_eq!(
            words::scheme(&ECDSA_WITH_SHA_256, &ID_SHA_256, Some(&certificate)),
            "ECDSA SM2 / SHA-256"
        );
        assert_eq!(words::key(&certificate), "EC SM2");
    }

    #[test]
    fn every_verdict_says_what_it_means() {
        assert!(Verdict::Unaltered.describe().contains("said separately"));
        assert!(Verdict::Altered.describe().contains("CHANGED"));
        assert!(Verdict::Invalid("why".into()).describe().contains("NOT VALID"));
        assert!(Verdict::Incomplete { covered: 10, total: 20 }
            .describe()
            .contains("never signed"));
        assert!(Verdict::Incomplete { covered: 10, total: 20 }.describe().contains("first 10 of 20"));
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

    /// And "unchanged" never claims more than it knows: the signature is the
    /// certificate's, and whether the certificate is worth anything is said to
    /// be an open question.
    #[test]
    fn unaltered_does_not_claim_the_certificate_is_trusted() {
        let said = Verdict::Unaltered.describe();
        assert!(said.contains("whether to trust that certificate is said separately"), "{said}");
    }
}
