//! SM4, and GCM composed over it.
//!
//! # Why the composition rather than a written-out GCM
//!
//! GCM is a Galois-field MAC bolted to counter mode, and every published attack
//! on it is an implementation mistake rather than a design one: a repeated
//! nonce, a tag compared with `==`, a length field counted wrong. A
//! hand-written GCM produces ciphertext that decrypts correctly under its own
//! decryptor and is not secure, and nothing downstream will tell you.
//!
//! So the construction comes from RustCrypto's generic AEAD machinery, which is
//! parameterised over the block cipher. `AesGcm<Sm4, U12>` is GCM over SM4 —
//! the name says AES because the crate was written for it, but the type is the
//! generic one and SM4 has the block size and key size it wants.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{AesGcm, Nonce};
use sm4::Sm4;
use zeroize::Zeroize;

use crate::error::{PdfError, Result};

/// GCM over SM4 with a 96-bit nonce.
///
/// 96 bits because that is the only nonce length GCM does not have to hash
/// first — every other length goes through GHASH to be reduced to twelve bytes,
/// which is more code doing the same job and a wider gap between what is
/// written and what is standard.
pub type Sm4Gcm = AesGcm<Sm4, aes_gcm::aes::cipher::consts::U12>;

/// Bytes of key material that wipe themselves.
///
/// A key left in a freed allocation is recoverable from a core dump, and a core
/// dump is a thing that happens to shipped software on other people's machines.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct Secret(pub [u8; 16]);

impl std::fmt::Debug for Secret {
    /// Never the bytes. A key that can be printed is a key that ends up in a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// Random bytes from the operating system.
pub fn random<const N: usize>() -> Result<[u8; N]> {
    use rand_core::RngCore;

    let mut out = [0u8; N];
    rand_core::OsRng
        .try_fill_bytes(&mut out)
        .map_err(|e| PdfError::Internal(format!("no randomness available: {e}")))?;
    Ok(out)
}

/// Seal `plaintext` under `key`, binding `associated` to the result.
///
/// Returns the nonce followed by the ciphertext and its tag. The nonce is
/// carried with the message rather than derived, because a derived nonce is a
/// repeated nonce the first time two documents share a key — and a repeated
/// nonce in GCM does not merely leak the plaintext, it leaks the authentication
/// key and forges every later message.
///
/// `associated` is authenticated but not encrypted: the envelope header goes
/// there, so a blob cannot be lifted out of one document and pasted into
/// another.
pub fn seal(key: &Secret, plaintext: &[u8], associated: &[u8]) -> Result<Vec<u8>> {
    let cipher = Sm4Gcm::new_from_slice(&key.0)
        .map_err(|_| PdfError::Internal("sm4-gcm: bad key length".into()))?;
    let nonce = random::<12>()?;

    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: associated })
        .map_err(|_| PdfError::Internal("sm4-gcm: could not seal".into()))?;

    let mut out = Vec::with_capacity(12 + sealed.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Open what `seal` produced.
///
/// **The tag is verified before a byte is returned**, which is the entire point
/// of an AEAD: a failure here means the ciphertext or the associated data was
/// changed, and the right response is to hand back nothing at all rather than
/// plaintext-shaped bytes for a caller to decide about.
pub fn open(key: &Secret, sealed: &[u8], associated: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < 12 + 16 {
        return Err(PdfError::InvalidArgument("sm4-gcm: too short to be sealed".into()));
    }
    let (nonce, body) = sealed.split_at(12);

    let cipher = Sm4Gcm::new_from_slice(&key.0)
        .map_err(|_| PdfError::Internal("sm4-gcm: bad key length".into()))?;
    cipher
        .decrypt(Nonce::from_slice(nonce), Payload { msg: body, aad: associated })
        // Deliberately the same message whichever part failed. Distinguishing
        // "wrong passcode" from "tampered" tells an attacker which half to work
        // on.
        .map_err(|_| PdfError::InvalidArgument("could not be opened — wrong passcode, or altered".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sm4::cipher::{BlockDecrypt, BlockEncrypt, KeyInit as _};
    use sm4::cipher::generic_array::GenericArray;

    /// **GB/T 32907-2016, appendix A.1 — the standard's own known-answer test.**
    ///
    /// A block cipher that is subtly wrong still produces convincing-looking
    /// ciphertext, and every layer above it will encrypt and decrypt happily
    /// against itself. Nothing downstream can catch a wrong S-box or a key
    /// schedule off by one round. This can.
    #[test]
    fn sm4_matches_the_standards_known_answer() {
        let key = hex("0123456789abcdeffedcba9876543210");
        let plain = hex("0123456789abcdeffedcba9876543210");
        let expected = hex("681edf34d206965e86b3e94f536e4246");

        let cipher = Sm4::new(GenericArray::from_slice(&key));
        let mut block = *GenericArray::from_slice(&plain);
        cipher.encrypt_block(&mut block);
        assert_eq!(block.as_slice(), &expected[..], "SM4 does not match GB/T 32907-2016");

        cipher.decrypt_block(&mut block);
        assert_eq!(block.as_slice(), &plain[..], "SM4 does not decrypt its own output");
    }

    /// The same appendix's **one-million-iteration** vector.
    ///
    /// The single-block test can pass with a key schedule that is wrong in a way
    /// that cancels out. A million chained encryptions cannot.
    #[test]
    #[ignore = "one million block encryptions; run with --ignored"]
    fn sm4_matches_the_standards_iterated_answer() {
        let key = hex("0123456789abcdeffedcba9876543210");
        let expected = hex("595298c7c6fd271f0402f804c33d3f66");

        let cipher = Sm4::new(GenericArray::from_slice(&key));
        let mut block = *GenericArray::from_slice(&hex("0123456789abcdeffedcba9876543210"));
        for _ in 0..1_000_000 {
            cipher.encrypt_block(&mut block);
        }
        assert_eq!(block.as_slice(), &expected[..], "SM4's key schedule is wrong");
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    fn key() -> Secret {
        Secret([7u8; 16])
    }

    #[test]
    fn what_is_sealed_can_be_opened() {
        let sealed = seal(&key(), b"part number 4821-A", b"header").expect("seal");
        assert_eq!(open(&key(), &sealed, b"header").expect("open"), b"part number 4821-A");
    }

    #[test]
    fn the_ciphertext_does_not_contain_the_plaintext() {
        let secret = b"THE-SECRET-VALUE";
        let sealed = seal(&key(), secret, b"").expect("seal");
        assert!(
            !sealed.windows(secret.len()).any(|w| w == secret),
            "the plaintext is sitting in the ciphertext"
        );
    }

    /// The tag is the whole reason Lock is tamper-evident. A single flipped bit
    /// anywhere must refuse, not decrypt to rubbish.
    #[test]
    fn a_single_altered_bit_refuses_to_open() {
        let sealed = seal(&key(), b"recoverable text", b"header").expect("seal");
        for at in [0, 12, sealed.len() - 1] {
            let mut altered = sealed.clone();
            altered[at] ^= 1;
            assert!(
                open(&key(), &altered, b"header").is_err(),
                "a flipped bit at {at} was accepted"
            );
        }
    }

    /// The associated data binds a blob to its document. Lifting one out and
    /// pasting it into another must fail even though the key is right.
    #[test]
    fn a_blob_moved_to_another_document_refuses_to_open() {
        let sealed = seal(&key(), b"page 4 of report A", b"document-A").expect("seal");
        assert!(open(&key(), &sealed, b"document-B").is_err(), "the blob was portable");
        assert!(open(&key(), &sealed, b"document-A").is_ok());
    }

    #[test]
    fn the_wrong_key_refuses_to_open() {
        let sealed = seal(&key(), b"hidden", b"").expect("seal");
        assert!(open(&Secret([8u8; 16]), &sealed, b"").is_err());
    }

    /// A repeated nonce in GCM does not merely leak the plaintext — it leaks the
    /// authentication key and forges every later message. The nonce is random
    /// per message and carried with it.
    #[test]
    fn sealing_the_same_thing_twice_gives_different_ciphertext() {
        let a = seal(&key(), b"same input", b"").expect("seal");
        let b = seal(&key(), b"same input", b"").expect("seal");
        assert_ne!(a, b, "the nonce is being reused");
        assert_ne!(a[..12], b[..12], "the nonce is not random");
    }

    #[test]
    fn something_too_short_to_be_sealed_is_refused_rather_than_indexed() {
        for len in [0usize, 11, 27] {
            assert!(open(&key(), &vec![0u8; len], b"").is_err(), "{len} bytes was accepted");
        }
    }
}
