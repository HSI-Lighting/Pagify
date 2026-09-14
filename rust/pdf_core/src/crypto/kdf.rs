//! Turning a passcode into a key-encryption key.
//!
//! # Why this is a trait when there is only one implementation
//!
//! Because the choice may change once, and the format cannot.
//!
//! If SM-algorithm compliance ever turns out to be a real requirement, Argon2id
//! has to go: it uses BLAKE2b internally, so an all-GM stack cannot include it,
//! and PBKDF2-HMAC-SM3 takes its place. That swap is a second `impl` and a
//! config flag — about a day.
//!
//! **What is not a day is the format.** Every locked document carries the key
//! that opened it, and if the envelope does not say *which KDF produced that
//! key*, a document locked under Argon2id can never be opened by a build that
//! also supports PBKDF2-HMAC-SM3, and vice versa. That is a format break, and
//! formats are the one thing here that cannot be fixed afterwards.
//!
//! So the discriminator is written from the first commit, and only Argon2id is
//! implemented. The stronger cryptography is the default; the compliance case
//! can prove itself later without stranding a single document.

use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::crypto::cipher::Secret;
use crate::error::{PdfError, Result};

/// What a derivation was given, stored beside the document so it can be
/// repeated exactly.
///
/// Recorded rather than assumed: raising the cost for new documents must not
/// lock anyone out of an old one, and it will not, because the old one says
/// what it was made with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory, in KiB.
    #[serde(rename = "m")]
    pub memory_kib: u32,
    /// Passes.
    #[serde(rename = "t")]
    pub time: u32,
    /// Lanes.
    #[serde(rename = "p")]
    pub lanes: u32,
}

impl KdfParams {
    /// The most memory a document may ask a derivation to use, in KiB: 1 GiB.
    pub const MAX_MEMORY_KIB: u32 = 1024 * 1024;
    /// The most passes.
    pub const MAX_TIME: u32 = 10;
    /// The most lanes.
    pub const MAX_LANES: u32 = 16;

    /// Refuse costs beyond what any document of ours would carry.
    ///
    /// **The parameters come from the file, and the file is untrusted.** A
    /// document that names `/M 4294967295` asks for four terabytes, and one
    /// that names `/T 4294967295` for four billion passes — before the
    /// password has been checked, because the check needs the key, so typing
    /// anything at all set it off. Found by audit. The ceilings are far above
    /// what this engine ever writes and well below what a machine can give.
    pub fn check(&self) -> Result<()> {
        if self.memory_kib > Self::MAX_MEMORY_KIB
            || self.time > Self::MAX_TIME
            || self.lanes > Self::MAX_LANES
        {
            return Err(PdfError::InvalidArgument(format!(
                "this document asks for a key derivation costing more than is allowed here \
                 ({} KiB, {} passes, {} lanes; the most is {} KiB, {} and {})",
                self.memory_kib,
                self.time,
                self.lanes,
                Self::MAX_MEMORY_KIB,
                Self::MAX_TIME,
                Self::MAX_LANES
            )));
        }
        Ok(())
    }
}

impl Default for KdfParams {
    /// ~48 MiB, three passes, one lane.
    ///
    /// **A starting point, not a measured one.** A memory-hard KDF is a real
    /// cost on a phone, and this engine is shared with Android and iOS; these
    /// numbers want measuring on a low-end device before they are called
    /// settled. They are recorded per document, so raising them later strands
    /// nobody.
    fn default() -> Self {
        KdfParams { memory_kib: 48 * 1024, time: 3, lanes: 1 }
    }
}

/// Something that turns a passcode into a key-encryption key.
pub trait KeyDerivation {
    /// The name written into the envelope. **Never inferred.**
    fn id(&self) -> &'static str;

    fn derive(&self, passcode: &[u8], salt: &[u8], params: &KdfParams) -> Result<Secret>;
}

/// The only implementation, and the default.
#[derive(Debug, Default, Clone, Copy)]
pub struct Argon2id;

impl KeyDerivation for Argon2id {
    fn id(&self) -> &'static str {
        "argon2id"
    }

    fn derive(&self, passcode: &[u8], salt: &[u8], params: &KdfParams) -> Result<Secret> {
        if salt.len() < 8 {
            return Err(PdfError::InvalidArgument("the salt is too short".into()));
        }
        // Here, so every path that reads a cost out of a file — the Secure
        // Plus dictionary, the lock's envelope, the ledger — is covered by
        // the one check.
        params.check()?;

        let settings = Params::new(params.memory_kib, params.time, params.lanes, Some(16))
            .map_err(|e| PdfError::InvalidArgument(format!("argon2: {e}")))?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, settings);

        let mut key = [0u8; 16];
        argon
            .hash_password_into(passcode, salt, &mut key)
            .map_err(|e| PdfError::Internal(format!("argon2: {e}")))?;

        let secret = Secret(key);
        // The copy on the stack, gone. `Secret` wipes its own.
        key.zeroize();
        Ok(secret)
    }
}

/// The derivation named in an envelope.
///
/// An unknown name is an error rather than a fallback: guessing which KDF made
/// a key produces a wrong key, and a wrong key is indistinguishable from a
/// wrong passcode. Better to say the document was made by a newer build.
pub fn by_id(id: &str) -> Result<Box<dyn KeyDerivation>> {
    match id {
        "argon2id" => Ok(Box::new(Argon2id)),
        other => Err(PdfError::Unsupported(match other {
            // A name this build knows of but cannot do is worth its own words.
            "pbkdf2-hmac-sm3" => "this document needs PBKDF2-HMAC-SM3, which this build does not have",
            _ => "this document was locked by a newer version of Pagify",
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A cost the file names is refused before a byte is allocated.**
    /// Found by audit: `/M 4294967295` is four terabytes, asked for before
    /// the password is checked.
    #[test]
    fn costs_beyond_the_ceiling_are_refused_before_deriving() {
        let salt = [7u8; 16];
        for params in [
            KdfParams { memory_kib: u32::MAX, time: 1, lanes: 1 },
            KdfParams { memory_kib: KdfParams::MAX_MEMORY_KIB + 1, time: 1, lanes: 1 },
            KdfParams { memory_kib: 64, time: u32::MAX, lanes: 1 },
            KdfParams { memory_kib: 64, time: 1, lanes: KdfParams::MAX_LANES + 1 },
        ] {
            let started = std::time::Instant::now();
            let outcome = Argon2id.derive(b"anything", &salt, &params);
            assert!(outcome.is_err(), "{params:?} was accepted");
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "{params:?} took {:?} to refuse",
                started.elapsed()
            );
            assert!(
                outcome.unwrap_err().to_string().contains("more than is allowed"),
                "the refusal does not say why"
            );
        }
        // The defaults, which every document of ours carries, are within it.
        KdfParams::default().check().expect("the defaults are allowed");
    }

    /// Cheap parameters. The defaults are memory-hard on purpose and would make
    /// this suite crawl; what is under test here is the plumbing, and the cost
    /// is measured separately on real hardware.
    fn quick() -> KdfParams {
        KdfParams { memory_kib: 64, time: 1, lanes: 1 }
    }

    #[test]
    fn the_same_passcode_and_salt_give_the_same_key() {
        let a = Argon2id.derive(b"open sesame", b"0123456789abcdef", &quick()).expect("derive");
        let b = Argon2id.derive(b"open sesame", b"0123456789abcdef", &quick()).expect("derive");
        assert_eq!(a.0, b.0);
    }

    /// The salt is what stops one precomputed table opening every document.
    #[test]
    fn a_different_salt_gives_a_different_key() {
        let a = Argon2id.derive(b"open sesame", b"0123456789abcdef", &quick()).expect("derive");
        let b = Argon2id.derive(b"open sesame", b"fedcba9876543210", &quick()).expect("derive");
        assert_ne!(a.0, b.0, "the salt is not reaching the derivation");
    }

    #[test]
    fn a_different_passcode_gives_a_different_key() {
        let a = Argon2id.derive(b"open sesame", b"0123456789abcdef", &quick()).expect("derive");
        let b = Argon2id.derive(b"open sesamf", b"0123456789abcdef", &quick()).expect("derive");
        assert_ne!(a.0, b.0);
    }

    /// The parameters are part of the key. Changing the cost and expecting the
    /// old key back would strand every document written before the change —
    /// which is why they are recorded per document rather than compiled in.
    #[test]
    fn different_parameters_give_a_different_key() {
        let a = Argon2id.derive(b"same", b"0123456789abcdef", &quick()).expect("derive");
        let b = Argon2id
            .derive(b"same", b"0123456789abcdef", &KdfParams { time: 2, ..quick() })
            .expect("derive");
        assert_ne!(a.0, b.0, "the parameters are not reaching the derivation");
    }

    #[test]
    fn a_salt_too_short_to_be_useful_is_refused() {
        assert!(Argon2id.derive(b"x", b"1234", &quick()).is_err());
    }

    /// **The name in the envelope is the contract.** If it ever changes, every
    /// document written before the change stops opening.
    #[test]
    fn the_identity_written_into_documents_is_fixed() {
        assert_eq!(Argon2id.id(), "argon2id");
    }

    #[test]
    fn a_known_derivation_can_be_looked_up_by_name() {
        assert_eq!(by_id("argon2id").expect("known").id(), "argon2id");
    }

    /// Guessing which KDF made a key produces a wrong key, and a wrong key is
    /// indistinguishable from a wrong passcode. Say so instead.
    #[test]
    fn an_unknown_derivation_is_refused_rather_than_guessed() {
        assert!(by_id("scrypt").is_err());
        assert!(by_id("pbkdf2-hmac-sm3").is_err(), "a KDF this build lacks was accepted");
    }
}
