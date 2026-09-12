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
//! It does **not** answer whether the signer is who they claim to be. That
//! needs a chain of trust up to a root somebody has decided to believe, and no
//! amount of checking here supplies it. A document signed with a certificate
//! made five minutes ago verifies perfectly and means nothing at all — but it
//! verifies under *that* certificate, whose subject is reported so a person can
//! decide what it is worth.
//!
//! Saying "valid" without that distinction is how a green tick comes to mean
//! less than nothing, so every verdict here carries it.
//!
//! # The check that catches the other real attack
//!
//! A signature covers a **range**, not a file. The classic way to alter a
//! signed document is to append to it: everything the range names is untouched,
//! the digest still matches, and the new content was never covered. So the
//! range is checked against the length of the file before anything else, and a
//! signature that does not reach the end is reported as exactly that.
//!
//! # Fail closed
//!
//! Anything this cannot check — a hash it does not implement, a signature
//! scheme it does not know, a certificate it cannot read — is reported as
//! *unreadable*, never as unaltered. The green tick is only ever earned.

use crate::error::Result;

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
            Verdict::Unaltered => "unchanged since it was signed, and the signature is the \
                                   certificate's (whether to trust that certificate is not checked)"
                .into(),
            Verdict::Altered => "CHANGED since it was signed".into(),
            Verdict::Invalid(why) => format!("NOT VALID — {why}"),
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
    ///
    /// **A label, not a finding.** It is plain text in the file, written by
    /// whoever wrote the file. The name that means something is `signer`.
    pub name: String,
    /// When it says it was made.
    pub when: String,
    /// Whether it is a signature or a document timestamp.
    pub timestamp: bool,
    pub verdict: Verdict,
    /// The subject of the certificate the signature verified under — only
    /// when it did. Who *holds* that certificate is a question of trust this
    /// does not answer.
    pub signer: Option<String>,
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
        let Checked { verdict, signer } = verdict_for(dict, bytes);
        out.push(Signature {
            name: text_of(dict.get(b"Name")),
            when: text_of(dict.get(b"M")),
            timestamp: subfilter.contains("RFC3161"),
            verdict,
            signer,
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
}

impl Checked {
    fn failed(verdict: Verdict) -> Self {
        Checked { verdict, signer: None }
    }
}

/// `id-ct-TSTInfo`: the content type of a timestamp token, whose content is
/// the `TSTInfo` the authority signed. Not in the OID database this uses.
const ID_CT_TST_INFO: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");

/// What became of one signature dictionary.
fn verdict_for(dict: &super::Dict, bytes: &[u8]) -> Checked {
    // -- the range, before anything else ----------------------------------
    let Some(numbers) = range_numbers(dict) else {
        return Checked::failed(Verdict::Unreadable("it declares no byte range".into()));
    };
    let [_, first, second_at, second_len] = numbers;
    let covered = first + second_len;
    let reaches = second_at + second_len;
    if reaches != bytes.len() {
        // **The append.** Everything named is untouched and the digest will
        // match; what matters is that the file is longer than the signature
        // ever claimed.
        return Checked::failed(Verdict::Incomplete { covered, total: bytes.len() });
    }
    if second_at + second_len > bytes.len() || first > bytes.len() {
        return Checked::failed(Verdict::Unreadable("its byte range runs past the file".into()));
    }
    let signed_bytes = [&bytes[..first], &bytes[second_at..second_at + second_len]];

    // -- the blob ----------------------------------------------------------
    let Some(blob) = hex_of(dict.get(b"Contents")) else {
        return Checked::failed(Verdict::Unreadable("it holds no signature".into()));
    };

    use der::Decode;
    let Ok(info) = cms::content_info::ContentInfo::from_der(&blob) else {
        return Checked::failed(Verdict::Unreadable("its signature is not a CMS structure".into()));
    };
    let Ok(data) = info.content.decode_as::<cms::signed_data::SignedData>() else {
        return Checked::failed(Verdict::Unreadable("its signature is not SignedData".into()));
    };
    let Some(signer) = data.signer_infos.0.as_ref().first() else {
        return Checked::failed(Verdict::Unreadable("its signature names no signer".into()));
    };
    let Some(attributes) = &signer.signed_attrs else {
        return Checked::failed(Verdict::Unreadable(
            "its signer committed to no attributes".into(),
        ));
    };
    let Some(hash) = Hash::named(&signer.digest_alg.oid) else {
        return Checked::failed(Verdict::Unreadable(format!(
            "its digest uses {}, which this does not implement",
            signer.digest_alg.oid
        )));
    };
    let committed = attributes
        .iter()
        .find(|a| a.oid == const_oid::db::rfc5911::ID_MESSAGE_DIGEST)
        .and_then(|a| a.values.as_ref().first().map(|v| v.value().to_vec()));
    let Some(committed) = committed else {
        return Checked::failed(Verdict::Unreadable("its signer committed to no digest".into()));
    };

    // -- what the signer committed to, against the file ----------------------
    //
    // A signature commits to the file's digest directly. A timestamp token
    // commits to a `TSTInfo`, and it is the imprint *inside* that which names
    // the file — the token's own digest is over the `TSTInfo`, and comparing
    // it with the file reads every genuine timestamp as an alteration.
    let content = &data.encap_content_info;
    if content.econtent_type == ID_CT_TST_INFO {
        let tst_info = content
            .econtent
            .as_ref()
            .and_then(|any| any.decode_as::<der::asn1::OctetStringRef>().ok())
            .map(|octets| octets.as_bytes().to_vec());
        let Some(tst_info) = tst_info else {
            return Checked::failed(Verdict::Unreadable("its token carries no TSTInfo".into()));
        };
        let Some((imprint_alg, imprint)) = imprint_in(&tst_info) else {
            return Checked::failed(Verdict::Unreadable(
                "its token's message imprint cannot be read".into(),
            ));
        };
        let Some(imprint_hash) = Hash::named(&imprint_alg) else {
            return Checked::failed(Verdict::Unreadable(format!(
                "its token's imprint uses {imprint_alg}, which this does not implement"
            )));
        };
        if imprint_hash.over(&signed_bytes) != imprint {
            return Checked::failed(Verdict::Altered);
        }
        if hash.over(&[&tst_info]) != committed {
            return Checked::failed(Verdict::Invalid(
                "the token's digest is not the digest of its own contents".into(),
            ));
        }
    } else if hash.over(&signed_bytes) != committed {
        return Checked::failed(Verdict::Altered);
    }

    // -- and whether anything vouches for what was committed to --------------
    let certificates = certificates_in(&data);
    match verify(signer, attributes, &certificates, hash) {
        Ok(subject) => Checked { verdict: Verdict::Unaltered, signer: Some(subject) },
        Err(verdict) => Checked::failed(verdict),
    }
}

/// The hashes this checks with. Anything else is reported as unreadable,
/// never guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hash {
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn named(oid: &const_oid::ObjectIdentifier) -> Option<Hash> {
        use const_oid::db::rfc5912::{ID_SHA_256, ID_SHA_384, ID_SHA_512};
        match *oid {
            ID_SHA_256 => Some(Hash::Sha256),
            ID_SHA_384 => Some(Hash::Sha384),
            ID_SHA_512 => Some(Hash::Sha512),
            _ => None,
        }
    }

    /// The digest over several stretches of bytes taken as one.
    fn over(self, parts: &[&[u8]]) -> Vec<u8> {
        use sha2::Digest;
        fn run<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut hasher = D::new();
            for part in parts {
                hasher.update(part);
            }
            hasher.finalize().to_vec()
        }
        match self {
            Hash::Sha256 => run::<sha2::Sha256>(parts),
            Hash::Sha384 => run::<sha2::Sha384>(parts),
            Hash::Sha512 => run::<sha2::Sha512>(parts),
        }
    }
}

/// The `messageImprint` of a `TSTInfo`: which hash, and the value.
///
/// ```text
/// TSTInfo ::= SEQUENCE {
///   version        INTEGER,
///   policy         OBJECT IDENTIFIER,
///   messageImprint SEQUENCE { hashAlgorithm AlgorithmIdentifier,
///                             hashedMessage OCTET STRING },
///   ... }
/// ```
///
/// Read field by field rather than through a full `TSTInfo` type: the three
/// fields wanted come first, and the ones after them are the ones that vary.
fn imprint_in(tst_info: &[u8]) -> Option<(const_oid::ObjectIdentifier, Vec<u8>)> {
    use der::{Decode, Reader, Tag, Tagged};

    let whole = der::asn1::AnyRef::from_der(tst_info).ok()?;
    if whole.tag() != Tag::Sequence {
        return None;
    }
    let mut fields = der::SliceReader::new(whole.value()).ok()?;
    let _version: der::asn1::AnyRef = fields.decode().ok()?;
    let _policy: der::asn1::AnyRef = fields.decode().ok()?;
    let imprint: der::asn1::AnyRef = fields.decode().ok()?;
    if imprint.tag() != Tag::Sequence {
        return None;
    }
    let mut inner = der::SliceReader::new(imprint.value()).ok()?;
    let algorithm: spki::AlgorithmIdentifierOwned = inner.decode().ok()?;
    let hashed: der::asn1::OctetStringRef = inner.decode().ok()?;
    Some((algorithm.oid, hashed.as_bytes().to_vec()))
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

/// Check the signature over the signed attributes against the certificates
/// it carries, and name the one it verified under.
///
/// The certificate the signer names is tried first; if it is not there or
/// does not fit, every other carried certificate is tried, because the point
/// is that *some* key that travelled with the signature made it — which one
/// is reported, and trust is for somebody else to decide.
fn verify(
    signer: &cms::signed_data::SignerInfo,
    attributes: &cms::signed_data::SignedAttributes,
    certificates: &[x509_cert::Certificate],
    hash: Hash,
) -> std::result::Result<String, Verdict> {
    use const_oid::db::rfc5912::{
        ID_RSASSA_PSS, RSA_ENCRYPTION, SHA_256_WITH_RSA_ENCRYPTION, SHA_384_WITH_RSA_ENCRYPTION,
        SHA_512_WITH_RSA_ENCRYPTION,
    };
    use der::Encode;

    if certificates.is_empty() {
        return Err(Verdict::Unreadable("it carries no certificate to check against".into()));
    }
    // Signed attributes are signed in their DER form as a SET — the
    // `[0] IMPLICIT` tag they wear inside the SignerInfo is not what was signed.
    let message = attributes
        .to_der()
        .map_err(|_| Verdict::Unreadable("its signed attributes cannot be re-encoded".into()))?;
    let signature = signer.signature.as_bytes();

    let scheme_oid = signer.signature_algorithm.oid;
    let scheme = match scheme_oid {
        RSA_ENCRYPTION => Scheme::Pkcs1v15(hash),
        SHA_256_WITH_RSA_ENCRYPTION => Scheme::Pkcs1v15(Hash::Sha256),
        SHA_384_WITH_RSA_ENCRYPTION => Scheme::Pkcs1v15(Hash::Sha384),
        SHA_512_WITH_RSA_ENCRYPTION => Scheme::Pkcs1v15(Hash::Sha512),
        ID_RSASSA_PSS => Scheme::Pss(hash),
        other => {
            return Err(Verdict::Unreadable(format!(
                "its signature scheme is {other}, which this does not check"
            )))
        }
    };

    let mut ordered: Vec<&x509_cert::Certificate> =
        certificates.iter().filter(|c| is_named(c, &signer.sid)).collect();
    ordered.extend(certificates.iter().filter(|c| !is_named(c, &signer.sid)));

    let mut unusable = None;
    for certificate in ordered {
        let key = match rsa_key_of(certificate) {
            Ok(key) => key,
            Err(why) => {
                unusable.get_or_insert(why);
                continue;
            }
        };
        if scheme.verifies(key, &message, signature) {
            return Ok(certificate.tbs_certificate.subject.to_string());
        }
    }
    match unusable {
        // Nothing could even be tried: say what stood in the way rather than
        // calling a signature invalid that was never checked.
        Some(why) if certificates.iter().all(|c| rsa_key_of(c).is_err()) => {
            Err(Verdict::Unreadable(why))
        }
        _ => Err(Verdict::Invalid(
            "the signature is not the certificate's — the digest matches, but nothing that \
             travelled with the signature made it"
                .into(),
        )),
    }
}

/// The RSA public key in a certificate, or why it is not one this can use.
fn rsa_key_of(certificate: &x509_cert::Certificate) -> std::result::Result<rsa::RsaPublicKey, String> {
    use der::Encode;
    use rsa::pkcs8::DecodePublicKey;
    let spki = &certificate.tbs_certificate.subject_public_key_info;
    if spki.algorithm.oid != const_oid::db::rfc5912::RSA_ENCRYPTION {
        return Err(format!(
            "its certificate's key is {}, which this does not check",
            spki.algorithm.oid
        ));
    }
    let der = spki.to_der().map_err(|_| "its certificate's key cannot be read".to_string())?;
    rsa::RsaPublicKey::from_public_key_der(&der)
        .map_err(|_| "its certificate's key cannot be read".to_string())
}

/// The signature schemes this checks.
#[derive(Debug, Clone, Copy)]
enum Scheme {
    Pkcs1v15(Hash),
    Pss(Hash),
}

impl Scheme {
    fn verifies(self, key: rsa::RsaPublicKey, message: &[u8], signature: &[u8]) -> bool {
        use rsa::signature::Verifier;
        fn pkcs1v15<D>(key: rsa::RsaPublicKey, message: &[u8], signature: &[u8]) -> bool
        where
            D: sha2::Digest + const_oid::AssociatedOid,
        {
            let Ok(signature) = rsa::pkcs1v15::Signature::try_from(signature) else {
                return false;
            };
            rsa::pkcs1v15::VerifyingKey::<D>::new(key).verify(message, &signature).is_ok()
        }
        fn pss<D>(key: rsa::RsaPublicKey, message: &[u8], signature: &[u8]) -> bool
        where
            D: sha2::Digest + rsa::signature::digest::FixedOutputReset,
        {
            let Ok(signature) = rsa::pss::Signature::try_from(signature) else {
                return false;
            };
            rsa::pss::VerifyingKey::<D>::new(key).verify(message, &signature).is_ok()
        }
        match self {
            Scheme::Pkcs1v15(Hash::Sha256) => pkcs1v15::<sha2::Sha256>(key, message, signature),
            Scheme::Pkcs1v15(Hash::Sha384) => pkcs1v15::<sha2::Sha384>(key, message, signature),
            Scheme::Pkcs1v15(Hash::Sha512) => pkcs1v15::<sha2::Sha512>(key, message, signature),
            Scheme::Pss(Hash::Sha256) => pss::<sha2::Sha256>(key, message, signature),
            Scheme::Pss(Hash::Sha384) => pss::<sha2::Sha384>(key, message, signature),
            Scheme::Pss(Hash::Sha512) => pss::<sha2::Sha512>(key, message, signature),
        }
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
        assert!(Verdict::Unaltered.describe().contains("not checked"));
        assert!(Verdict::Altered.describe().contains("CHANGED"));
        assert!(Verdict::Invalid("why".into()).describe().contains("NOT VALID"));
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

    /// And "unchanged" never claims more than it knows: the signature is the
    /// certificate's, and whether the certificate is worth anything is said to
    /// be an open question.
    #[test]
    fn unaltered_does_not_claim_the_certificate_is_trusted() {
        let said = Verdict::Unaltered.describe();
        assert!(said.contains("whether to trust that certificate is not checked"), "{said}");
    }
}
