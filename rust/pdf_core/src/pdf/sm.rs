//! The SM suite on the wire: what SM2 and SM3 are called in a signature, and
//! how an SM2 signature is written into one.
//!
//! One place for the three facts that signing and checking must agree on, so
//! that they cannot drift apart. `sign.rs` writes with these; `validate.rs`
//! reads with the same ones.
//!
//! # The distinguishing ID
//!
//! SM2 does not sign a message. It signs `SM3(ZA ‖ M)`, where `ZA` folds in
//! the curve, the signer's public key and an **identity string** — and signer
//! and verifier must use the same string, or a perfectly good signature fails
//! to verify with no hint as to why. The standard's default is
//! `1234567812345678` (GB/T 32918.5-2017 uses it in its own worked example),
//! and it is pinned here as a constant rather than taken from the `sm2` crate,
//! which could move its default between versions.
//!
//! # The identifiers
//!
//! From the Chinese commercial-cryptography arc, `1.2.156.10197`:
//!
//! | | OID |
//! |---|---|
//! | the SM2 curve, in a certificate's key | `1.2.156.10197.1.301` |
//! | SM3, as a digest algorithm | `1.2.156.10197.1.401` |
//! | SM2 over SM3, as a signature algorithm | `1.2.156.10197.1.501` |
//!
//! OpenSSL writes exactly these into a CMS `SignerInfo`, and the first is
//! what the `sm2` crate checks when it reads a key.
//!
//! # The signature's shape
//!
//! The `sm2` crate hands over `r ‖ s`, two 32-byte numbers. A CMS signature
//! value carries `SEQUENCE { INTEGER r, INTEGER s }` — the same shape ECDSA
//! uses (RFC 5480 §2.2), and what GM/T 0009 prescribes for SM2. The two
//! functions below go between them, and nothing else in the crate touches the
//! encoding.

use const_oid::ObjectIdentifier;

/// The signer identity folded into every SM2 signature made or checked here.
///
/// **Both sides read this constant.** See the module note for why it is a
/// constant at all.
pub const DISTINGUISHING_ID: &str = "1234567812345678";

/// SM3 as a digest algorithm.
pub const ID_SM3: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.156.10197.1.401");

/// SM2 over SM3 as a signature algorithm.
pub const ID_SM2_WITH_SM3: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.156.10197.1.501");

/// An SM2 signature as a CMS signature value: `SEQUENCE { INTEGER r, INTEGER s }`.
pub fn signature_to_der(signature: &sm2::dsa::Signature) -> der::Result<Vec<u8>> {
    use der::Encode;
    let bytes = signature.to_bytes();
    let (r, s) = bytes.split_at(32);
    // `UintRef` strips leading zeros and adds the sign byte back on encoding,
    // so a number whose top bit is set is written as DER wants it.
    let mut body = der::asn1::UintRef::new(r)?.to_der()?;
    body.extend(der::asn1::UintRef::new(s)?.to_der()?);
    let mut out = der::Header::new(der::Tag::Sequence, body.len())?.to_der()?;
    out.extend(body);
    Ok(out)
}

/// The signature back out of a CMS signature value, or `None` if that is not
/// what the bytes hold.
///
/// Strict: DER only, both numbers at most 32 bytes, nothing after them. A
/// signature that needs a looser reading is not one this made.
pub fn signature_from_der(der: &[u8]) -> Option<sm2::dsa::Signature> {
    use der::{Decode, Reader, Tagged};

    let whole = der::asn1::AnyRef::from_der(der).ok()?;
    if whole.tag() != der::Tag::Sequence {
        return None;
    }
    let mut fields = der::SliceReader::new(whole.value()).ok()?;
    let r: der::asn1::UintRef = fields.decode().ok()?;
    let s: der::asn1::UintRef = fields.decode().ok()?;
    if !fields.is_finished() {
        return None;
    }
    let mut bytes = [0u8; 64];
    let (r_slot, s_slot) = bytes.split_at_mut(32);
    for (number, slot) in [(r.as_bytes(), r_slot), (s.as_bytes(), s_slot)] {
        if number.len() > 32 {
            return None;
        }
        slot[32 - number.len()..].copy_from_slice(number);
    }
    sm2::dsa::Signature::from_bytes(&bytes.into()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sm2::dsa::signature::{Signer, Verifier};

    fn hex(text: &str) -> Vec<u8> {
        let digits: Vec<u8> = text.bytes().filter(u8::is_ascii_hexdigit).collect();
        digits
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    // -- known answers from the standards ---------------------------------
    //
    // The `sm2` crate is younger than the RSA implementations it replaces,
    // so its answers are checked against the standards' own worked examples.
    // A wrong implementation fails these; it cannot pass them by accident.

    /// GB/T 32905-2016 (SM3), Appendix A: the two examples.
    #[test]
    fn sm3_gives_the_standards_answers() {
        use sm3::{Digest, Sm3};
        assert_eq!(
            Sm3::digest(b"abc").to_vec(),
            hex("66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0")
        );
        assert_eq!(
            Sm3::digest(b"abcd".repeat(16)).to_vec(),
            hex("debe9ff92275b8a138604889c18e5a4d6fdb70e5387e5765293dcba39c0c5732")
        );
    }

    /// GB/T 32918.5-2017 (SM2 parameters), Appendix A.2: the digital signature
    /// example on the recommended curve. The message is `message digest`, the
    /// identity is the standard default — the one pinned here — and the
    /// signature is the one the standard prints.
    ///
    /// Verify-side only, by necessity: the crate draws its nonce by RFC 6979
    /// and the standard's example uses a fixed `k`, so the signature this
    /// makes over the same message is a different, equally valid one.
    #[test]
    fn the_standards_sm2_signature_verifies_under_the_pinned_identity() {
        let public = hex(
            "04 09F9DF311E5421A150DD7D161E4BC5C672179FAD1833FC076BB08FF356F35020 \
                CCEA490CE26775A52DC6EA718CC1AA600AED05FBF35E084A6632F6072DA9AD13",
        );
        let signature = hex(
            "F5A03B0648D2C4630EEAC513E1BB81A15944DA3827D5B74143AC7EACEEE720B3 \
             B1B6AA29DF212FD8763182BC0D421CA1BB9038FD1F7F42D4840B69C485BBC1AA",
        );
        let key = sm2::dsa::VerifyingKey::from_sec1_bytes(DISTINGUISHING_ID, &public).unwrap();
        let signature = sm2::dsa::Signature::from_slice(&signature).unwrap();
        assert!(key.verify(b"message digest", &signature).is_ok());

        // And not under another identity, which is the whole reason the
        // identity is pinned: the same key, the same bytes, a different
        // string — and nothing verifies.
        let other = sm2::dsa::VerifyingKey::from_sec1_bytes("ALICE123@YAHOO.COM", &public).unwrap();
        assert!(other.verify(b"message digest", &signature).is_err());
        // Nor over a different message.
        assert!(key.verify(b"message digest.", &signature).is_err());
    }

    /// The identity is what the standard's example uses, so that anything
    /// signed here verifies in another SM2 implementation left at its default.
    #[test]
    fn the_pinned_identity_is_the_standard_default() {
        assert_eq!(DISTINGUISHING_ID, "1234567812345678");
        assert_eq!(DISTINGUISHING_ID.len() * 8, 128, "ENTL is 0x0080 in the standard's example");
    }

    // -- the wire shape -----------------------------------------------------

    /// What is written is `SEQUENCE { INTEGER, INTEGER }`, and reading it back
    /// gives the signature that was written — including one whose numbers
    /// have their top bit set, which DER pads with a sign byte.
    #[test]
    fn a_signature_goes_to_der_and_back() {
        let key = sm2::dsa::SigningKey::from_slice(
            DISTINGUISHING_ID,
            &hex("3945208F7B2144B13F36E38AC6D39F9588939369 2860B51A42FB81EF4DF7C5B8"),
        )
        .unwrap();
        for message in [&b"one"[..], b"two", b"three", b"four", b"five", b"six", b"seven", b"eight"] {
            let signature = key.sign(message);
            let der = signature_to_der(&signature).expect("encodes");
            assert_eq!(der[0], 0x30, "not a SEQUENCE");
            assert_eq!(der[2], 0x02, "the first element is not an INTEGER");
            assert!(der.len() >= 68 && der.len() <= 72, "{} bytes", der.len());
            let back = signature_from_der(&der).expect("reads back");
            assert_eq!(back.to_bytes(), signature.to_bytes());
            assert!(key.verifying_key().verify(message, &back).is_ok());
        }
    }

    /// The standard's own signature, written the way OpenSSL writes it: both
    /// numbers here have their top bit set, so both carry a sign byte.
    #[test]
    fn the_standards_signature_encodes_with_sign_bytes() {
        let signature = sm2::dsa::Signature::from_slice(&hex(
            "F5A03B0648D2C4630EEAC513E1BB81A15944DA3827D5B74143AC7EACEEE720B3 \
             B1B6AA29DF212FD8763182BC0D421CA1BB9038FD1F7F42D4840B69C485BBC1AA",
        ))
        .unwrap();
        let der = signature_to_der(&signature).expect("encodes");
        assert_eq!(
            der,
            hex("3046 022100 F5A03B0648D2C4630EEAC513E1BB81A15944DA3827D5B74143AC7EACEEE720B3 \
                      022100 B1B6AA29DF212FD8763182BC0D421CA1BB9038FD1F7F42D4840B69C485BBC1AA")
        );
        assert_eq!(signature_from_der(&der).unwrap().to_bytes(), signature.to_bytes());
    }

    /// Bytes that are not a signature are refused rather than read loosely.
    #[test]
    fn what_is_not_a_signature_is_not_read_as_one() {
        assert!(signature_from_der(b"").is_none());
        assert!(signature_from_der(&[0x30, 0x00]).is_none());
        // An INTEGER pair with a third element after them.
        let mut trailing = hex("3009 020101 020102 020103");
        assert!(signature_from_der(&trailing).is_none());
        // A number wider than the curve: 33 bytes, and a positive one.
        trailing = hex("3026 0221 010000000000000000000000000000000000000000000000000000000000000000 020101");
        assert!(signature_from_der(&trailing).is_none());
        // Not DER: a leading zero that is not a sign byte.
        assert!(signature_from_der(&hex("3008 02020001 020101")).is_none());
    }
}
