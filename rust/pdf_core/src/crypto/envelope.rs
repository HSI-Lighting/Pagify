//! The header stored beside locked content.
//!
//! Everything needed to get back to the data key, and nothing secret: the salt,
//! the KDF that used it, the parameters it used, and the data key wrapped under
//! the key that derivation produces.
//!
//! # Defined here, in Rust, once
//!
//! Read by all three platforms **through the core**, never parsed on the Swift
//! or Kotlin side. That is the `restore`-blob lesson applied before the third
//! platform exists rather than after it: a format with two parsers has two
//! interpretations, and the day they disagree is the day a document locked on a
//! phone will not open on a Mac.
//!
//! # Versioned, and the KDF named
//!
//! `v` is checked before anything else is read. `kdf.id` is written explicitly
//! and never implied — without it, a document locked under Argon2id could never
//! be opened by a build that also had PBKDF2-HMAC-SM3, because nothing would
//! say which one to try.

use serde::{Deserialize, Serialize};

use crate::crypto::cipher::{self, Secret};
use crate::crypto::kdf::{self, Argon2id, KdfParams, KeyDerivation};
use crate::error::{PdfError, Result};

/// The format this build writes.
///
/// Bumped only for a change a previous build could not read. Adding a field
/// with a `serde` default is not such a change.
pub const FORMAT_VERSION: u32 = 1;

/// Which derivation made the key, and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfRecord {
    pub id: String,
    #[serde(flatten)]
    pub params: KdfParams,
}

/// The public header of a locked thing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub v: u32,
    pub kdf: KdfRecord,
    /// Stored in the clear. A salt is not a secret — its job is to make one
    /// precomputed table useless against a second document, and it does that in
    /// the open.
    #[serde(with = "hex_bytes")]
    pub salt: Vec<u8>,
    /// The data key, sealed under the key-encryption key.
    #[serde(with = "hex_bytes")]
    pub wrapped_dek: Vec<u8>,
}

impl Envelope {
    /// Make a new data key and wrap it under `passcode`.
    ///
    /// The data key is **random, not derived**. Deriving it would mean a
    /// passcode change re-encrypts every blob in the document rather than
    /// re-wrapping sixteen bytes, and would make two passcodes over the same
    /// content impossible.
    pub fn create(passcode: &[u8], params: KdfParams) -> Result<(Envelope, Secret)> {
        let salt = cipher::random::<16>()?;
        let kek = Argon2id.derive(passcode, &salt, &params)?;
        let dek = Secret(cipher::random::<16>()?);

        // The header authenticates the wrapping, so a salt or a cost cannot be
        // edited without the unwrap failing.
        let header = Self::binding(FORMAT_VERSION, Argon2id.id(), &params, &salt);
        let wrapped = cipher::seal(&kek, &dek.0, &header)?;

        Ok((
            Envelope {
                v: FORMAT_VERSION,
                kdf: KdfRecord { id: Argon2id.id().to_string(), params },
                salt: salt.to_vec(),
                wrapped_dek: wrapped,
            },
            dek,
        ))
    }

    /// Recover the data key.
    ///
    /// A wrong passcode and a tampered envelope both fail here, with the same
    /// words — telling them apart tells an attacker which half to work on.
    pub fn unwrap_dek(&self, passcode: &[u8]) -> Result<Secret> {
        if self.v > FORMAT_VERSION {
            return Err(PdfError::Unsupported(
                "this document was locked by a newer version of Pagify",
            ));
        }
        let derivation = kdf::by_id(&self.kdf.id)?;
        let kek = derivation.derive(passcode, &self.salt, &self.kdf.params)?;

        let header = Self::binding(self.v, &self.kdf.id, &self.kdf.params, &self.salt);
        let opened = cipher::open(&kek, &self.wrapped_dek, &header)?;

        let bytes: [u8; 16] = opened
            .try_into()
            .map_err(|_| PdfError::InvalidArgument("the wrapped key is the wrong size".into()))?;
        Ok(Secret(bytes))
    }

    /// Change the passcode without touching a single blob.
    ///
    /// **The reason there are two levels of key.** Re-wrapping sixteen bytes is
    /// instant; re-encrypting a locked document would be as slow as locking it
    /// again and would need every blob in hand.
    pub fn rewrap(&self, old: &[u8], new: &[u8], params: KdfParams) -> Result<Envelope> {
        let dek = self.unwrap_dek(old)?;

        let salt = cipher::random::<16>()?;
        let kek = Argon2id.derive(new, &salt, &params)?;
        let header = Self::binding(FORMAT_VERSION, Argon2id.id(), &params, &salt);
        let wrapped = cipher::seal(&kek, &dek.0, &header)?;

        Ok(Envelope {
            v: FORMAT_VERSION,
            kdf: KdfRecord { id: Argon2id.id().to_string(), params },
            salt: salt.to_vec(),
            wrapped_dek: wrapped,
        })
    }

    /// The header bytes the wrapping is bound to.
    ///
    /// Built by hand rather than by serialising the struct, because the binding
    /// must not change when the serialisation does — a reordered field or a
    /// prettier encoder would otherwise stop every existing document opening.
    fn binding(v: u32, id: &str, params: &KdfParams, salt: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"pagify-envelope\0");
        out.extend_from_slice(&v.to_le_bytes());
        out.extend_from_slice(id.as_bytes());
        out.push(0);
        out.extend_from_slice(&params.memory_kib.to_le_bytes());
        out.extend_from_slice(&params.time.to_le_bytes());
        out.extend_from_slice(&params.lanes.to_le_bytes());
        out.extend_from_slice(salt);
        out
    }
}

/// Bytes as hex in the JSON. Base64 would be shorter; hex is readable in a file
/// somebody is trying to understand, and an envelope is measured in bytes.
pub(crate) mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push_str(&format!("{b:02x}"));
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() % 2 != 0 {
            return Err(serde::de::Error::custom("odd number of hex digits"));
        }
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(serde::de::Error::custom))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quick() -> KdfParams {
        KdfParams { memory_kib: 64, time: 1, lanes: 1 }
    }

    #[test]
    fn a_data_key_comes_back_with_the_right_passcode() {
        let (envelope, dek) = Envelope::create(b"correct horse", quick()).expect("create");
        let back = envelope.unwrap_dek(b"correct horse").expect("unwrap");
        assert_eq!(back.0, dek.0);
    }

    #[test]
    fn the_wrong_passcode_gets_nothing() {
        let (envelope, _) = Envelope::create(b"correct horse", quick()).expect("create");
        assert!(envelope.unwrap_dek(b"correct hoarse").is_err());
    }

    /// Random, not derived — which is what makes a passcode change cheap and two
    /// passcodes over one document possible.
    #[test]
    fn two_documents_with_the_same_passcode_have_different_keys() {
        let (_, a) = Envelope::create(b"same passcode", quick()).expect("create");
        let (_, b) = Envelope::create(b"same passcode", quick()).expect("create");
        assert_ne!(a.0, b.0, "the data key is being derived from the passcode");
    }

    /// **Why the KDF is named in the envelope.** Without this, a build that
    /// gained a second KDF could not tell which one made an existing document.
    #[test]
    fn the_envelope_says_which_derivation_made_it() {
        let (envelope, _) = Envelope::create(b"x", quick()).expect("create");
        assert_eq!(envelope.kdf.id, "argon2id");

        let json = serde_json::to_string(&envelope).expect("serialise");
        assert!(json.contains("\"id\":\"argon2id\""), "the KDF is not in the JSON: {json}");
    }

    #[test]
    fn an_envelope_survives_a_round_trip_through_json() {
        let (envelope, dek) = Envelope::create(b"passcode", quick()).expect("create");
        let json = serde_json::to_string(&envelope).expect("serialise");
        let back: Envelope = serde_json::from_str(&json).expect("parse");

        assert_eq!(back, envelope);
        assert_eq!(back.unwrap_dek(b"passcode").expect("unwrap").0, dek.0);
    }

    /// The header is authenticated, so its fields cannot be edited to weaken a
    /// document — lowering the recorded cost and re-deriving must fail.
    #[test]
    fn weakening_the_recorded_parameters_breaks_the_unwrap() {
        let (mut envelope, _) = Envelope::create(b"passcode", quick()).expect("create");
        envelope.kdf.params.time += 1;
        assert!(envelope.unwrap_dek(b"passcode").is_err(), "the parameters were not bound");
    }

    #[test]
    fn altering_the_salt_breaks_the_unwrap() {
        let (mut envelope, _) = Envelope::create(b"passcode", quick()).expect("create");
        envelope.salt[0] ^= 1;
        assert!(envelope.unwrap_dek(b"passcode").is_err());
    }

    /// **The reason there are two levels of key.**
    #[test]
    fn a_passcode_change_keeps_the_same_data_key() {
        let (envelope, dek) = Envelope::create(b"old passcode", quick()).expect("create");
        let moved = envelope.rewrap(b"old passcode", b"new passcode", quick()).expect("rewrap");

        assert_eq!(
            moved.unwrap_dek(b"new passcode").expect("unwrap").0,
            dek.0,
            "the data key changed, so every blob in the document would need re-encrypting"
        );
        assert!(moved.unwrap_dek(b"old passcode").is_err(), "the old passcode still works");
    }

    #[test]
    fn rewrapping_with_the_wrong_passcode_is_refused() {
        let (envelope, _) = Envelope::create(b"old", quick()).expect("create");
        assert!(envelope.rewrap(b"wrong", b"new", quick()).is_err());
    }

    /// A document from a newer build is refused by name rather than mis-read.
    #[test]
    fn a_newer_format_is_refused_rather_than_guessed_at() {
        let (mut envelope, _) = Envelope::create(b"x", quick()).expect("create");
        envelope.v = FORMAT_VERSION + 1;
        assert!(envelope.unwrap_dek(b"x").is_err());
    }

    #[test]
    fn an_unknown_kdf_is_refused() {
        let (mut envelope, _) = Envelope::create(b"x", quick()).expect("create");
        envelope.kdf.id = "pbkdf2-hmac-sm3".into();
        assert!(envelope.unwrap_dek(b"x").is_err());
    }

    /// Nothing secret is in the header. The salt and the parameters are public
    /// by design; the data key is only there wrapped.
    #[test]
    fn the_envelope_carries_no_secret_in_the_clear() {
        let (envelope, dek) = Envelope::create(b"passcode", quick()).expect("create");
        let json = serde_json::to_string(&envelope).expect("serialise");

        let hex: String = dek.0.iter().map(|b| format!("{b:02x}")).collect();
        assert!(!json.contains(&hex), "the data key is in the envelope in the clear");
        assert!(!json.contains("passcode"), "the passcode is in the envelope");
    }
}
