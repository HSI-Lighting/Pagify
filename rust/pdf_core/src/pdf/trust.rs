//! Whether the certificate a signature verified under is one to believe.
//!
//! `validate.rs` answers two questions — is the file as it was signed, and did
//! the key in the carried certificate make the signature — and stops there,
//! because anyone with the `sm2` crate can make a certificate. This module
//! answers the third: **was that certificate issued by a root Pagify is built
//! to trust?** One root, pinned; not a trust store.
//!
//! ```text
//! HSI root (SM2, self-signed, long-lived)         pinned — compiled into the app
//!    └── signing leaf (SM2, issued by the root)   held by whoever signs, in a .p12
//!           └── document signature
//! ```
//!
//! # Why one pinned root and not a trust store
//!
//! A general trust store is the thing whoever can add a certificate to can
//! forge signatures with — and it brings revocation freshness, clock trust and
//! cross-platform store management along with it, none of which is needed
//! when every signer is HSI or a distributor HSI issued a leaf to. Pinning the
//! leaf directly would make every leaf rotation an app release. The root sits
//! between: issuing a new leaf ships nothing; a compromised leaf goes on the
//! denylist below; a root rotation is an app release that pins both roots for
//! the overlap.
//!
//! # Where the root lives, and what makes that safe
//!
//! The roots are compiled into the binary with `include_bytes!` from
//! `trust/roots.der` — no loose file in the bundle to swap. That only means
//! anything because the bundle is signed with a Developer ID
//! (`packaging/macos/bundle.sh` refuses to ship one that is not): a patched
//! executable no longer matches its signature, and macOS refuses to run it.
//! **The file is empty until HSI's root exists**, and until then nothing is
//! pinned and every signature is [`Trust::Unrecognised`] — which is the
//! truth, not a placeholder.
//!
//! # Revocation, proportionately
//!
//! Not CRLs, not OCSP: a list compiled in from `trust/denylist.txt`, keyed by
//! **issuer and serial** — never serial alone — and updated by release. HSI
//! issues the leaves, so HSI knows which ones are bad. A revoked leaf's
//! earlier signatures read as revoked too: there is no trusted time to prove
//! they came first (document timestamps went with the network), and that is
//! the safe direction.
//!
//! # What is not checked, and why
//!
//! Validity dates. The only time a signature carries is the signer's own
//! clock, and a check against *this* machine's clock would make a document's
//! verdict change on the day the leaf expires — years after it was signed,
//! for no reason to do with the document. Revocation is the denylist; expiry
//! would be a second, worse revocation with no list behind it. Intermediates
//! are not walked either: a leaf is issued by a pinned root directly, which is
//! what the issuing tool makes, or it is unrecognised.

use crate::error::{PdfError, Result};

use der::Encode;
use x509_cert::Certificate;

/// What the pinned roots say about the certificate a signature verified
/// under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// Issued by a pinned root, and not on the denylist. The green tick that
    /// means something — together with [`super::validate::Verdict::Unaltered`],
    /// and only then.
    Pinned,
    /// An SM2 signature whose certificate is not issued by a pinned root. The
    /// one thing this means: the signature is sound, and nobody Pagify is
    /// built to believe vouches for the signer. The subject is shown; a
    /// person decides.
    Unrecognised,
    /// Issued by a pinned root, and on the denylist by issuer and serial.
    Revoked,
}

impl Trust {
    pub fn describe(&self) -> &'static str {
        match self {
            Trust::Pinned => "issued by a root Pagify trusts",
            Trust::Unrecognised => "not issued by a root Pagify trusts — who the signer is, \
                                    only the certificate's subject says",
            Trust::Revoked => "REVOKED — issued by a root Pagify trusts, and since withdrawn",
        }
    }
}

/// The roots to believe and the leaves not to: what a document's signer is
/// judged against.
#[derive(Debug, Clone, Default)]
pub struct Anchors {
    roots: Vec<Certificate>,
    denied: Vec<Denied>,
}

/// One denylist entry: a certificate, as X.509 identifies one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Denied {
    /// The issuer's name as `x509_cert` prints it (`CN=…,O=…`).
    issuer: String,
    /// The serial in upper-case hex without leading zeros.
    serial: String,
}

/// The roots compiled into this binary, and the denylist beside them.
const PINNED_ROOTS: &[u8] = include_bytes!("../../trust/roots.der");
const DENYLIST: &str = include_str!("../../trust/denylist.txt");

impl Anchors {
    /// What the shipped binary trusts: the compiled-in roots and denylist.
    ///
    /// Parsed once. A file that does not parse is a build that should not
    /// have shipped, and is reported as trusting nothing rather than
    /// something else.
    pub fn pinned() -> &'static Anchors {
        static PINNED: std::sync::OnceLock<Anchors> = std::sync::OnceLock::new();
        PINNED.get_or_init(|| Anchors::new(PINNED_ROOTS, DENYLIST).unwrap_or_default())
    }

    /// Anchors that trust nobody.
    pub fn none() -> Anchors {
        Anchors::default()
    }

    /// Anchors from a run of DER certificates — zero or more, one after the
    /// other — and a denylist in the format `trust/denylist.txt` describes.
    pub fn new(roots_der: &[u8], denylist: &str) -> Result<Anchors> {
        use der::Reader;
        let mut roots = Vec::new();
        let mut reader = der::SliceReader::new(roots_der)
            .map_err(|_| PdfError::InvalidArgument("the pinned roots cannot be read".into()))?;
        while !reader.is_finished() {
            let root: Certificate = reader.decode().map_err(|_| {
                PdfError::InvalidArgument("the pinned roots are not a run of DER certificates".into())
            })?;
            roots.push(root);
        }
        let mut denied = Vec::new();
        for (number, line) in denylist.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((serial, issuer)) = line.split_once(char::is_whitespace) else {
                return Err(PdfError::InvalidArgument(format!(
                    "denylist line {}: expected `<serial hex> <issuer>`",
                    number + 1
                )));
            };
            if serial.is_empty() || !serial.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(PdfError::InvalidArgument(format!(
                    "denylist line {}: the serial is not hex",
                    number + 1
                )));
            }
            denied.push(Denied { issuer: issuer.trim().to_string(), serial: normalised(serial) });
        }
        Ok(Anchors { roots, denied })
    }

    /// How many roots are pinned.
    pub fn root_count(&self) -> usize {
        self.roots.len()
    }

    /// How many leaves are denied.
    pub fn denied_count(&self) -> usize {
        self.denied.len()
    }
}

/// A serial as the denylist keys it: upper-case hex, no leading zeros.
fn normalised(serial_hex: &str) -> String {
    let upper = serial_hex.to_ascii_uppercase();
    let trimmed = upper.trim_start_matches('0');
    if trimmed.is_empty() { "0".into() } else { trimmed.to_string() }
}

/// What the anchors say about a certificate.
///
/// Issued by a pinned root — the issuer name matches the root's subject
/// **and** the certificate's own SM2 signature verifies under the root's key;
/// a root that merely carries HSI's name is not HSI's root — and then, only
/// then, checked against the denylist.
pub fn trust_in(certificate: &Certificate, anchors: &Anchors) -> Trust {
    if !anchors.roots.iter().any(|root| issued_by(certificate, root)) {
        return Trust::Unrecognised;
    }
    let issuer = certificate.tbs_certificate.issuer.to_string();
    let serial = normalised(&hex(certificate.tbs_certificate.serial_number.as_bytes()));
    let denied = anchors.denied.iter().any(|d| d.issuer == issuer && d.serial == serial);
    if denied { Trust::Revoked } else { Trust::Pinned }
}

/// Whether `root`'s key signed `leaf`.
///
/// The signature over a certificate is SM2 over SM3 of the `tbsCertificate`
/// in DER, under the same distinguishing ID as a document signature: the
/// standard's default, which GM/T 0015 prescribes for certificates and which
/// the issuing tool uses. **OpenSSL does not**: left to itself it signs a
/// certificate with an *empty* ID, and verifies with one, so a certificate
/// it issued chains here only if it was made with
/// `-sigopt distid:1234567812345678` — as the test fixtures were.
fn issued_by(leaf: &Certificate, root: &Certificate) -> bool {
    use sm2::dsa::signature::Verifier;
    if leaf.tbs_certificate.issuer != root.tbs_certificate.subject {
        return false;
    }
    if leaf.signature_algorithm.oid != super::sm::ID_SM2_WITH_SM3 {
        return false;
    }
    let Ok(key) = super::validate::sm2_key_of(root) else { return false };
    let Ok(tbs) = leaf.tbs_certificate.to_der() else { return false };
    let Some(signature) = super::sm::signature_from_der(leaf.signature.raw_bytes()) else {
        return false;
    };
    key.verify(&tbs, &signature).is_ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name))
            .expect("the fixture is committed")
    }

    /// **The shipped binary pins nothing yet**, and says so: HSI's root does
    /// not exist until the issuing tool makes it, and an empty pin is the
    /// truth rather than a test root standing in for it.
    #[test]
    fn nothing_is_pinned_until_the_real_root_exists() {
        let pinned = Anchors::pinned();
        assert_eq!(pinned.root_count(), 0, "a root is pinned — is it HSI's, and is this on purpose?");
        assert_eq!(pinned.denied_count(), 0);
    }

    /// A run of DER certificates parses to as many roots as it holds — none,
    /// one, two — and anything that is not one is refused.
    #[test]
    fn roots_are_a_run_of_der_certificates() {
        assert_eq!(Anchors::new(b"", "").expect("empty").root_count(), 0);
        let root = fixture("test-root-sm2.der");
        assert_eq!(Anchors::new(&root, "").expect("one").root_count(), 1);
        let mut two = root.clone();
        two.extend(fixture("test-lookalike-root-sm2.der"));
        assert_eq!(Anchors::new(&two, "").expect("two").root_count(), 2);
        assert!(Anchors::new(b"%PDF-1.7", "").is_err());
        let mut cut = root.clone();
        cut.truncate(100);
        assert!(Anchors::new(&cut, "").is_err());
    }

    /// The denylist reads as the file says: serial then issuer, comments and
    /// blank lines ignored, the serial in any case with or without leading
    /// zeros, and a line that is not that refused.
    #[test]
    fn the_denylist_is_read_as_documented() {
        let text = "# comment\n\n  1002  CN=Pagify Test Root,O=Pagify \n00ab CN=X\n";
        let anchors = Anchors::new(b"", text).expect("parses");
        assert_eq!(anchors.denied_count(), 2);
        assert_eq!(
            anchors.denied[0],
            Denied { issuer: "CN=Pagify Test Root,O=Pagify".into(), serial: "1002".into() }
        );
        assert_eq!(anchors.denied[1], Denied { issuer: "CN=X".into(), serial: "AB".into() });
        assert!(Anchors::new(b"", "1002").is_err(), "a serial with no issuer");
        assert!(Anchors::new(b"", "xyz CN=X").is_err(), "a serial that is not hex");
        // And the shipped denylist itself parses.
        Anchors::new(b"", DENYLIST).expect("trust/denylist.txt parses");
    }

    #[test]
    fn every_trust_says_what_it_means() {
        assert!(Trust::Pinned.describe().contains("trusts"));
        assert!(Trust::Unrecognised.describe().contains("not issued"));
        assert!(Trust::Revoked.describe().contains("REVOKED"));
        assert!(!Trust::Unrecognised.describe().contains("REVOKED"));
    }
}
