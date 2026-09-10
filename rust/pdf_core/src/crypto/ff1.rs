//! FF1 format-preserving encryption, NIST SP 800-38G.
//!
//! Encrypts a string into another string of **the same length over the same
//! alphabet**. A ten-digit part number becomes a different ten-digit part
//! number, so a partner's ingestion pipeline still parses it.
//!
//! # What this is for, and what it is not
//!
//! FPE's real job is preserving downstream parseability, **not
//! confidentiality**. It has no integrity: there is no tag, nothing detects
//! tampering, and a recipient cannot tell an altered value from a real one.
//! Anything that must be safe gets Lock or Redact instead.
//!
//! # Built against AES, then given SM4
//!
//! The published test vectors are AES-based, and validating the construction
//! before changing the cipher is the only way to know the FF1 is right — FF1
//! over SM4 has no known-answer tests of its own. So [`Ff1`] is generic over the
//! block cipher, [`ff1_aes128`] exists to be checked against SP 800-38G, and
//! [`Ff1Sm4`] is the same code with a different round function.
//!
//! The swap is sound — FF1's security argument reduces to the PRF security of
//! its round function, and SM4 matches AES-128's block and key size — but the
//! result is **no longer NIST FF1**. It is a non-standard instantiation, and
//! any certification naming FF1 does not cover it.

use aes_gcm::aes::cipher::generic_array::GenericArray;
use aes_gcm::aes::cipher::{BlockEncrypt, KeyInit};

use crate::error::{PdfError, Result};

/// The smallest domain FF1 may be used on: `radix^len ≥ 1,000,000`.
///
/// SP 800-38G's own floor. Six characters for digits, five for A–Z, four for
/// alphanumerics. **A shorter field is refused, never silently weakened** — a
/// four-digit part number has ten thousand possible values and encrypting it
/// only rearranges which one you see.
pub const MIN_DOMAIN: u128 = 1_000_000;

/// The largest domain this implementation will take.
///
/// `radix^ceil(n/2)` has to stay inside 64 bits for the modular arithmetic
/// below to be provably free of overflow. That allows a 38-character numeric
/// field or a 24-character alphanumeric one, which is far past any field worth
/// obfuscating — and refusing beyond it is better than a silent wrap.
const MAX_HALF_DOMAIN: u128 = u64::MAX as u128;

/// The alphabet a field is written in.
///
/// **Declared per field, never inferred per character.** Inferring it means the
/// decryptor has to guess the same way the encryptor did, and the first value
/// that happens to contain only digits inside an alphanumeric field decrypts to
/// something else entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alphabet {
    symbols: Vec<char>,
}

impl Alphabet {
    pub const DIGITS: &'static str = "0123456789";
    pub const UPPER: &'static str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    pub const ALPHANUMERIC: &'static str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

    pub fn new(symbols: &str) -> Result<Self> {
        let symbols: Vec<char> = symbols.chars().collect();
        if symbols.len() < 2 {
            return Err(PdfError::InvalidArgument(
                "an alphabet needs at least two symbols".into(),
            ));
        }
        let mut seen = symbols.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() != symbols.len() {
            // A repeated symbol makes two different plaintexts encrypt to the
            // same ciphertext and decryption ambiguous.
            return Err(PdfError::InvalidArgument(
                "an alphabet cannot repeat a symbol".into(),
            ));
        }
        Ok(Alphabet { symbols })
    }

    pub fn radix(&self) -> u32 {
        self.symbols.len() as u32
    }

    fn to_numerals(&self, text: &str) -> Result<Vec<u32>> {
        text.chars()
            .map(|c| {
                self.symbols
                    .iter()
                    .position(|s| *s == c)
                    .map(|i| i as u32)
                    .ok_or_else(|| {
                        PdfError::InvalidArgument(format!(
                            "{c:?} is not in this field's alphabet"
                        ))
                    })
            })
            .collect()
    }

    fn to_text(&self, numerals: &[u32]) -> String {
        numerals.iter().map(|n| self.symbols[*n as usize]).collect()
    }
}

/// FF1 over a block cipher.
pub struct Ff1<C: BlockEncrypt> {
    cipher: C,
    alphabet: Alphabet,
}

impl<C: BlockEncrypt + KeyInit> Ff1<C> {
    pub fn new(key: &[u8], alphabet: Alphabet) -> Result<Self> {
        let cipher = C::new_from_slice(key)
            .map_err(|_| PdfError::InvalidArgument("ff1: wrong key length".into()))?;
        Ok(Ff1 { cipher, alphabet })
    }
}

impl<C: BlockEncrypt> Ff1<C> {
    /// Encrypt, preserving length and alphabet.
    ///
    /// `tweak` must differ per field. FF1 takes one by design, and using it is
    /// what stops the same value encrypting identically everywhere in a
    /// document — without it, a reader learns which parts of a catalogue share
    /// a part number.
    pub fn encrypt(&self, text: &str, tweak: &[u8]) -> Result<String> {
        let x = self.checked_numerals(text)?;
        Ok(self.alphabet.to_text(&self.rounds(&x, tweak, true)?))
    }

    pub fn decrypt(&self, text: &str, tweak: &[u8]) -> Result<String> {
        let x = self.checked_numerals(text)?;
        Ok(self.alphabet.to_text(&self.rounds(&x, tweak, false)?))
    }

    /// The guardrails from the plan, **as errors rather than warnings**. A
    /// caller must not be able to ask for a weak encryption and get one.
    fn checked_numerals(&self, text: &str) -> Result<Vec<u32>> {
        let numerals = self.alphabet.to_numerals(text)?;
        let n = numerals.len() as u32;
        let radix = self.alphabet.radix() as u128;

        if n < 2 {
            return Err(PdfError::InvalidArgument(
                "ff1: a field of fewer than two characters cannot be encrypted".into(),
            ));
        }

        // radix^n, saturating: only its comparison with the floor matters.
        let domain = (0..n).try_fold(1u128, |acc, _| acc.checked_mul(radix)).unwrap_or(u128::MAX);
        if domain < MIN_DOMAIN {
            return Err(PdfError::InvalidArgument(format!(
                "ff1: {n} characters over {radix} symbols is only {domain} possible values — \
                 too few to encrypt safely. At least {MIN_DOMAIN} are needed"
            )));
        }

        let half = n.div_ceil(2);
        let half_domain =
            (0..half).try_fold(1u128, |acc, _| acc.checked_mul(radix)).unwrap_or(u128::MAX);
        if half_domain > MAX_HALF_DOMAIN {
            return Err(PdfError::InvalidArgument(format!(
                "ff1: {n} characters over {radix} symbols is longer than this implementation \
                 takes"
            )));
        }
        Ok(numerals)
    }

    /// The ten Feistel rounds of SP 800-38G algorithms 7 and 8.
    fn rounds(&self, x: &[u32], tweak: &[u8], forward: bool) -> Result<Vec<u32>> {
        let n = x.len() as u32;
        let radix = self.alphabet.radix();
        let u = n / 2;
        let v = n - u;

        let mut a = x[..u as usize].to_vec();
        let mut b = x[u as usize..].to_vec();

        // b: bytes needed for a v-digit value. d: bytes taken from the round
        // function's output.
        let bytes_per_half = ((v as f64 * (radix as f64).log2()).ceil() / 8.0).ceil() as usize;
        let d = 4 * bytes_per_half.div_ceil(4) + 4;

        let mod_u = pow(radix as u128, u);
        let mod_v = pow(radix as u128, v);

        // The fixed half of the round input.
        let mut p = Vec::with_capacity(16);
        p.extend_from_slice(&[1, 2, 1]);
        p.extend_from_slice(&radix.to_be_bytes()[1..]); // three bytes
        p.push(10);
        p.push((u % 256) as u8);
        p.extend_from_slice(&n.to_be_bytes());
        p.extend_from_slice(&(tweak.len() as u32).to_be_bytes());
        debug_assert_eq!(p.len(), 16);

        let pad = (16 - ((tweak.len() + bytes_per_half + 1) % 16)) % 16;

        for round in 0..10u8 {
            let i = if forward { round } else { 9 - round };
            let (source, modulus, m) = if forward {
                (&b, if i % 2 == 0 { mod_u } else { mod_v }, if i % 2 == 0 { u } else { v })
            } else {
                (&a, if i % 2 == 0 { mod_u } else { mod_v }, if i % 2 == 0 { u } else { v })
            };

            let mut q = Vec::with_capacity(tweak.len() + pad + 1 + bytes_per_half);
            q.extend_from_slice(tweak);
            q.extend(std::iter::repeat_n(0u8, pad));
            q.push(i);
            q.extend_from_slice(&be_bytes(num_radix(source, radix), bytes_per_half));

            let r = self.prf(&p, &q);
            let s = self.expand(&r, d);
            let y = num_bytes_mod(&s, modulus);

            if forward {
                let c = (num_radix_mod(&a, radix, modulus) + y) % modulus;
                let c = digits(c, radix, m);
                a = std::mem::replace(&mut b, c);
            } else {
                // Subtraction, and the modulus keeps it non-negative.
                let c = (num_radix_mod(&b, radix, modulus) + modulus - y % modulus) % modulus;
                let c = digits(c, radix, m);
                b = std::mem::replace(&mut a, c);
            }
        }

        let mut out = a;
        out.extend_from_slice(&b);
        Ok(out)
    }

    /// CBC-MAC over `P || Q` with a zero IV, as SP 800-38G's PRF.
    fn prf(&self, p: &[u8], q: &[u8]) -> [u8; 16] {
        let mut chain = [0u8; 16];
        for block in p.iter().chain(q.iter()).collect::<Vec<_>>().chunks(16) {
            for (c, b) in chain.iter_mut().zip(block.iter()) {
                *c ^= **b;
            }
            let mut g = GenericArray::clone_from_slice(&chain);
            self.cipher.encrypt_block(&mut g);
            chain.copy_from_slice(g.as_slice());
        }
        chain
    }

    /// `R` extended to `d` bytes by encrypting `R xor j`.
    fn expand(&self, r: &[u8; 16], d: usize) -> Vec<u8> {
        let mut out = r.to_vec();
        let mut j = 1u32;
        while out.len() < d {
            let mut block = *r;
            for (k, byte) in j.to_be_bytes().iter().enumerate() {
                block[12 + k] ^= byte;
            }
            let mut g = GenericArray::clone_from_slice(&block);
            self.cipher.encrypt_block(&mut g);
            out.extend_from_slice(g.as_slice());
            j += 1;
        }
        out.truncate(d);
        out
    }
}

fn pow(base: u128, exp: u32) -> u128 {
    (0..exp).fold(1u128, |acc, _| acc * base)
}

/// A numeral string as an integer. Only called where the value is known to fit.
fn num_radix(digits: &[u32], radix: u32) -> u128 {
    digits.iter().fold(0u128, |acc, d| acc * radix as u128 + *d as u128)
}

fn num_radix_mod(digits: &[u32], radix: u32, modulus: u128) -> u128 {
    digits.iter().fold(0u128, |acc, d| (acc * radix as u128 + *d as u128) % modulus)
}

/// A byte string as an integer, reduced as it goes.
///
/// Reduced *during* the fold rather than after: the round function's output is
/// up to twenty bytes, which no fixed-width integer holds, and the modulus is
/// bounded so that `acc * 256` cannot overflow.
fn num_bytes_mod(bytes: &[u8], modulus: u128) -> u128 {
    bytes.iter().fold(0u128, |acc, b| (acc * 256 + *b as u128) % modulus)
}

fn be_bytes(mut value: u128, len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    for slot in out.iter_mut().rev() {
        *slot = (value & 0xFF) as u8;
        value >>= 8;
    }
    out
}

fn digits(mut value: u128, radix: u32, len: u32) -> Vec<u32> {
    let mut out = vec![0u32; len as usize];
    for slot in out.iter_mut().rev() {
        *slot = (value % radix as u128) as u32;
        value /= radix as u128;
    }
    out
}

/// FF1 over AES-128 — **for validating the construction**, not for use.
///
/// The published vectors are AES-based; this exists so the Feistel structure,
/// the PRF and the byte lengths can be checked against them before the cipher
/// is changed.
pub fn ff1_aes128(key: &[u8], alphabet: Alphabet) -> Result<Ff1<aes_gcm::aes::Aes128>> {
    Ff1::new(key, alphabet)
}

/// FF1 over SM4 — what the product uses.
pub type Ff1Sm4 = Ff1<sm4::Sm4>;

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    const NIST_KEY: &str = "2B7E151628AED2A6ABF7158809CF4F3C";

    fn sm4(alphabet: &str) -> Ff1Sm4 {
        Ff1Sm4::new(&[0x2bu8; 16], Alphabet::new(alphabet).unwrap()).unwrap()
    }

    // -- the swap ----------------------------------------------------------

    /// The same code with a different round function. There are no published
    /// vectors for this — which is precisely why the AES ones above matter.
    #[test]
    fn ff1_over_sm4_round_trips() {
        let ff1 = sm4(Alphabet::DIGITS);
        let sealed = ff1.encrypt("4821000734", b"tweak").unwrap();
        assert_ne!(sealed, "4821000734", "the value came back unchanged");
        assert_eq!(ff1.decrypt(&sealed, b"tweak").unwrap(), "4821000734");
    }

    /// Format preservation is the entire feature: same length, same alphabet.
    #[test]
    fn the_shape_of_the_value_survives() {
        for (alphabet, value) in [
            (Alphabet::DIGITS, "4821000734"),
            (Alphabet::UPPER, "LUMINAIRE"),
            (Alphabet::ALPHANUMERIC, "IP65A4821"),
        ] {
            let ff1 = sm4(alphabet);
            let out = ff1.encrypt(value, b"t").unwrap();
            assert_eq!(out.chars().count(), value.chars().count(), "{value}: length changed");
            assert!(
                out.chars().all(|c| alphabet.contains(c)),
                "{value} encrypted to {out}, which leaves the alphabet"
            );
        }
    }

    /// **Without a tweak, the same value encrypts identically everywhere** — and
    /// a reader learns which parts of a catalogue share a part number without
    /// decrypting anything.
    #[test]
    fn a_different_tweak_gives_a_different_answer() {
        let ff1 = sm4(Alphabet::DIGITS);
        let a = ff1.encrypt("4821000734", b"page1/object7").unwrap();
        let b = ff1.encrypt("4821000734", b"page9/object2").unwrap();
        assert_ne!(a, b, "the tweak is not reaching the encryption");
    }

    #[test]
    fn the_wrong_tweak_does_not_recover_the_value() {
        let ff1 = sm4(Alphabet::DIGITS);
        let sealed = ff1.encrypt("4821000734", b"right").unwrap();
        assert_ne!(ff1.decrypt(&sealed, b"wrong").unwrap(), "4821000734");
    }

    #[test]
    fn the_wrong_key_does_not_recover_the_value() {
        let sealed = sm4(Alphabet::DIGITS).encrypt("4821000734", b"t").unwrap();
        let other = Ff1Sm4::new(&[0x99u8; 16], Alphabet::new(Alphabet::DIGITS).unwrap()).unwrap();
        assert_ne!(other.decrypt(&sealed, b"t").unwrap(), "4821000734");
    }

    // -- the guardrails, as errors -----------------------------------------

    /// **`radix^len ≥ 1,000,000`, refused rather than weakened.** A four-digit
    /// part number has ten thousand possible values; encrypting it only
    /// rearranges which one you see.
    #[test]
    fn a_field_too_short_to_encrypt_safely_is_refused() {
        let ff1 = sm4(Alphabet::DIGITS);
        for short in ["4821", "48210", "482"] {
            let refused = ff1.encrypt(short, b"t");
            assert!(refused.is_err(), "{short:?} was encrypted anyway");
            assert!(
                format!("{}", refused.unwrap_err()).contains("too few"),
                "the refusal does not say why"
            );
        }
        // Six digits is exactly the floor, and allowed.
        assert!(ff1.encrypt("482100", b"t").is_ok(), "the floor itself was refused");
    }

    /// The floor depends on the alphabet: five letters clear a million, four do
    /// not.
    #[test]
    fn the_floor_follows_the_alphabet() {
        let upper = sm4(Alphabet::UPPER);
        assert!(upper.encrypt("LUMEN", b"t").is_ok(), "five letters should be enough");
        assert!(upper.encrypt("LUME", b"t").is_err(), "four letters is under the floor");

        let alnum = sm4(Alphabet::ALPHANUMERIC);
        assert!(alnum.encrypt("IP65", b"t").is_ok(), "four alphanumerics should be enough");
    }

    /// A character outside the declared alphabet is an error, not a passthrough.
    /// Silently leaving it alone would tell a reader exactly where the real
    /// characters are.
    #[test]
    fn a_character_outside_the_alphabet_is_refused() {
        let ff1 = sm4(Alphabet::DIGITS);
        let refused = ff1.encrypt("4821-0007", b"t");
        assert!(refused.is_err(), "a hyphen was accepted into a digits-only field");
    }

    #[test]
    fn an_alphabet_that_repeats_a_symbol_is_refused() {
        assert!(Alphabet::new("0123456780").is_err(), "a repeated symbol was accepted");
        assert!(Alphabet::new("0").is_err(), "a one-symbol alphabet was accepted");
    }

    /// Longer than the arithmetic can hold, refused rather than wrapped.
    #[test]
    fn a_field_longer_than_this_implementation_takes_is_refused() {
        let ff1 = sm4(Alphabet::DIGITS);
        let long = "1".repeat(120);
        assert!(ff1.encrypt(&long, b"t").is_err(), "an over-long field silently wrapped");
    }

    /// Every value in a domain must map to a distinct value, or decryption is
    /// ambiguous. Checked exhaustively over a small-but-legal domain.
    #[test]
    fn the_mapping_is_a_permutation() {
        let ff1 = sm4(Alphabet::DIGITS);
        let mut seen = std::collections::BTreeSet::new();
        for n in 0..2000u32 {
            let value = format!("{n:06}");
            let out = ff1.encrypt(&value, b"tweak").unwrap();
            assert!(seen.insert(out.clone()), "{value} and something else both gave {out}");
            assert_eq!(ff1.decrypt(&out, b"tweak").unwrap(), value);
        }
    }

    /// **NIST SP 800-38G, sample 1** — FF1-AES128, radix 10, no tweak.
    ///
    /// An FF1 that is subtly wrong still round-trips against itself: encrypt and
    /// decrypt agree, output looks random, and every value in the document is
    /// quietly encrypted under something weaker than intended. These vectors are
    /// the only thing that can tell.
    #[test]
    fn ff1_matches_nist_sample_1() {
        let ff1 = ff1_aes128(&hex(NIST_KEY), Alphabet::new(Alphabet::DIGITS).unwrap()).unwrap();
        assert_eq!(ff1.encrypt("0123456789", &[]).unwrap(), "2433477484");
        assert_eq!(ff1.decrypt("2433477484", &[]).unwrap(), "0123456789");
    }

    /// **Sample 2** — the same input with a tweak. A different answer is the
    /// whole point of the tweak.
    #[test]
    fn ff1_matches_nist_sample_2() {
        let ff1 = ff1_aes128(&hex(NIST_KEY), Alphabet::new(Alphabet::DIGITS).unwrap()).unwrap();
        let tweak = hex("39383736353433323130");
        assert_eq!(ff1.encrypt("0123456789", &tweak).unwrap(), "6124200773");
        assert_eq!(ff1.decrypt("6124200773", &tweak).unwrap(), "0123456789");
    }

    /// **Sample 3** — radix 36, an odd length, and a tweak. Exercises the
    /// uneven Feistel split that samples 1 and 2 do not.
    #[test]
    fn ff1_matches_nist_sample_3() {
        let alphabet = Alphabet::new("0123456789abcdefghijklmnopqrstuvwxyz").unwrap();
        let ff1 = ff1_aes128(&hex(NIST_KEY), alphabet).unwrap();
        let tweak = hex("3737373770717273373737");
        assert_eq!(ff1.encrypt("0123456789abcdefghi", &tweak).unwrap(), "a9tv40mll9kdu509eum");
        assert_eq!(ff1.decrypt("a9tv40mll9kdu509eum", &tweak).unwrap(), "0123456789abcdefghi");
    }
}
