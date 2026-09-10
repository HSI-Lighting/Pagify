//! Pagify's own encryption — stronger, and readable by nothing else.
//!
//! # Why this exists beside [`super::encrypt`]
//!
//! That module writes what ISO 32000-2 specifies, because the reader on the
//! other side is somebody else's. This one is the opposite trade, taken
//! deliberately: **no other reader will open these files at all.**
//!
//! What that buys is freedom from the specification's choices. PDF's own
//! handler derives its key with a SHA-2 loop tuned in 2017 and encrypts with
//! AES-CBC, which is unauthenticated — a reader cannot tell a corrupted stream
//! from an altered one. Here the key comes from **Argon2id**, which is memory-
//! hard, and every stream and string is sealed with **SM4-GCM**, which is
//! authenticated: an altered byte fails to open rather than decrypting to
//! rubbish.
//!
//! # What a stranger sees
//!
//! `/Filter /Pagify` in the `/Encrypt` dictionary. A conforming reader that
//! meets a security handler it does not know is required to refuse the
//! document, and that is what happens — Preview, Acrobat and the rest report a
//! file they cannot open rather than asking for a password. **That is not a
//! side effect; it is the feature**, and anything offering it has to say so
//! before the person commits.
//!
//! # The format
//!
//! ```text
//! /Encrypt <<
//!   /Filter /Pagify          the handler, which is what makes others refuse
//!   /SubFilter /Pagify.SM4GCM.1   what the bytes actually are
//!   /Salt <16 bytes>         for Argon2id
//!   /M /T /P                 its cost, so a file outlives a change of default
//!   /Verify <sealed>         a known phrase, sealed under the key
//! >>
//! ```
//!
//! Every other stream and string is `nonce ‖ ciphertext ‖ tag`, which is what
//! [`crate::crypto::cipher::seal`] produces.

use crate::crypto::cipher::{self, Secret};
use crate::crypto::kdf::{self, KdfParams, KeyDerivation};
use crate::error::{PdfError, Result};

use super::object::Dict;
use super::{write_object, write_stream, File, Object};

/// What the `/SubFilter` says, and what this code knows how to read.
///
/// Versioned from the first commit for the same reason the vault's format is:
/// a document sealed today has to be openable by a build that has moved on, and
/// the only way to promise that is to write down what it was sealed with.
const SUBFILTER: &str = "Pagify.SM4GCM.1";

/// What `/Verify` holds once opened, so a wrong passcode is known immediately
/// rather than after a page fails to parse.
const PHRASE: &[u8] = b"pagify secure plus";

/// The key for a document, and the parameters that made it.
pub struct SecurePlus {
    key: Secret,
    salt: [u8; 16],
    params: KdfParams,
}

impl SecurePlus {
    /// Derive a key for a new document.
    pub fn new(password: &[u8], params: KdfParams, random: impl Fn(&mut [u8])) -> Result<Self> {
        if password.is_empty() {
            return Err(PdfError::InvalidArgument(
                "a document cannot be secured with an empty password".into(),
            ));
        }
        let mut salt = [0u8; 16];
        random(&mut salt);
        let key = kdf::Argon2id.derive(password, &salt, &params)?;
        Ok(SecurePlus { key, salt, params })
    }

    /// Derive the key for a document that already has one, and check it.
    ///
    /// The check is the `/Verify` entry: a known phrase sealed under the key.
    /// Opening it proves the password without touching a page — and because
    /// SM4-GCM is authenticated, it cannot be faked by anyone editing the file.
    pub fn open(password: &[u8], dict: &Dict) -> Result<Self> {
        let subfilter = dict
            .get(b"SubFilter")
            .and_then(Object::as_name)
            .map(|n| String::from_utf8_lossy(n).into_owned());
        if subfilter.as_deref() != Some(SUBFILTER) {
            return Err(PdfError::Unsupported(
                "this document is secured in a way this version cannot read",
            ));
        }

        let salt_bytes = hex_of(dict.get(b"Salt"))
            .ok_or_else(|| PdfError::InvalidArgument("that document has no salt".into()))?;
        if salt_bytes.len() != 16 {
            return Err(PdfError::InvalidArgument("that document's salt is the wrong size".into()));
        }
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&salt_bytes);

        let number = |key: &[u8], fallback: u32| -> u32 {
            dict.get(key)
                .and_then(Object::as_f64)
                .map(|n| n as u32)
                .unwrap_or(fallback)
        };
        let params = KdfParams {
            memory_kib: number(b"M", 49_152),
            time: number(b"T", 3),
            lanes: number(b"P", 1),
        };

        let key = kdf::Argon2id.derive(password, &salt, &params)?;
        let verify = hex_of(dict.get(b"Verify"))
            .ok_or_else(|| PdfError::InvalidArgument("that document has no check value".into()))?;
        let opened = cipher::open(&key, &verify, SUBFILTER.as_bytes())?;
        if opened != PHRASE {
            return Err(PdfError::InvalidArgument("incorrect password".into()));
        }
        Ok(SecurePlus { key, salt, params })
    }

    /// The `/Encrypt` dictionary for this document.
    fn dictionary(&self) -> Result<Dict> {
        let mut dict = Dict(Vec::new());
        // The handler's name is what makes every other reader refuse the file.
        dict.set(b"Filter", Object::Name(b"Pagify".to_vec()));
        dict.set(b"SubFilter", Object::Name(SUBFILTER.as_bytes().to_vec()));
        dict.set(b"Salt", Object::HexString(hex(&self.salt)));
        dict.set(b"M", Object::Number(self.params.memory_kib.to_string().into_bytes()));
        dict.set(b"T", Object::Number(self.params.time.to_string().into_bytes()));
        dict.set(b"P", Object::Number(self.params.lanes.to_string().into_bytes()));
        dict.set(
            b"Verify",
            Object::HexString(hex(&cipher::seal(&self.key, PHRASE, SUBFILTER.as_bytes())?)),
        );
        Ok(dict)
    }
}

/// Write a copy sealed with SM4-GCM under a password.
pub fn secure(file: &File<'_>, plus: &SecurePlus) -> Result<Vec<u8>> {
    if file.trailer().get(b"Encrypt").is_some() {
        return Err(PdfError::Unsupported(
            "that document is already secured — remove its password first",
        ));
    }

    let numbers: Vec<u32> = file.numbers().collect();
    let mut replacements: Vec<(u32, Vec<u8>)> = Vec::with_capacity(numbers.len());
    for number in &numbers {
        // Each object is sealed against its own number, so a stream cannot be
        // lifted out of one place in the file and pasted into another.
        let binding = number.to_be_bytes();
        let mut body = Vec::new();
        match file.object(*number)? {
            Object::Stream(dict, range) => {
                let plain = file.bytes().get(range).ok_or_else(|| {
                    PdfError::InvalidArgument(format!("object {number} runs past the file"))
                })?;
                let sealed = cipher::seal(&plus.key, plain, &binding)?;
                let mut dict = seal_strings(&Object::Dict(dict), plus, &binding)?
                    .as_dict()
                    .cloned()
                    .unwrap_or(Dict(Vec::new()));
                dict.set(b"Length", Object::Number(sealed.len().to_string().into_bytes()));
                body = write_stream(&dict, &sealed);
            }
            other => write_object(&mut body, &seal_strings(&other, plus, &binding)?),
        }
        replacements.push((*number, body));
    }

    let encrypt_number = numbers.iter().copied().max().unwrap_or(0) + 1;
    let mut encrypt_body = Vec::new();
    write_object(&mut encrypt_body, &Object::Dict(plus.dictionary()?));

    let mut trailer = Dict(Vec::new());
    trailer.set(b"Encrypt", Object::Reference(encrypt_number, 0));
    file.rewrite_adding(&replacements, &[(encrypt_number, encrypt_body)], &trailer)
}

/// Read a sealed document back to plaintext.
///
/// PDFium cannot open one of these — no reader can — so the whole file is
/// turned back into an ordinary PDF here and handed on.
pub fn unseal(file: &File<'_>, plus: &SecurePlus) -> Result<Vec<u8>> {
    let encrypt = match file.trailer().get(b"Encrypt") {
        Some(Object::Reference(number, _)) => *number,
        _ => return Err(PdfError::InvalidArgument("that document is not sealed".into())),
    };

    let mut replacements: Vec<(u32, Vec<u8>)> = Vec::new();
    for number in file.numbers().collect::<Vec<_>>() {
        // The `/Encrypt` dictionary itself was never sealed — it is what a
        // reader needs before it has a key — and it goes away entirely.
        if number == encrypt {
            continue;
        }
        let binding = number.to_be_bytes();
        let mut body = Vec::new();
        match file.object(number)? {
            Object::Stream(dict, range) => {
                let sealed = file.bytes().get(range).ok_or_else(|| {
                    PdfError::InvalidArgument(format!("object {number} runs past the file"))
                })?;
                let plain = cipher::open(&plus.key, sealed, &binding)?;
                let mut dict = open_strings(&Object::Dict(dict), plus, &binding)?
                    .as_dict()
                    .cloned()
                    .unwrap_or(Dict(Vec::new()));
                dict.set(b"Length", Object::Number(plain.len().to_string().into_bytes()));
                body = write_stream(&dict, &plain);
            }
            other => write_object(&mut body, &open_strings(&other, plus, &binding)?),
        }
        replacements.push((number, body));
    }

    let mut trailer = Dict(Vec::new());
    // `Null` means "take this key out" — see `File::rewrite_dropping`.
    trailer.set(b"Encrypt", Object::Null);
    file.rewrite_dropping(&replacements, &[], &trailer, &[encrypt])
}

/// Whether a file is one of ours, and its `/Encrypt` dictionary if so.
pub fn dictionary_of(file: &File<'_>) -> Option<Dict> {
    let encrypt = file.trailer().get(b"Encrypt")?;
    let dict = file.resolve(encrypt).ok()?.as_dict().cloned()?;
    let filter = dict.get(b"Filter").and_then(Object::as_name)?;
    (filter == b"Pagify").then_some(dict)
}

fn seal_strings(object: &Object, plus: &SecurePlus, binding: &[u8]) -> Result<Object> {
    Ok(match object {
        Object::LiteralString(raw) => {
            Object::HexString(hex(&cipher::seal(&plus.key, &super::encrypt::unescape(raw), binding)?))
        }
        Object::HexString(raw) => {
            Object::HexString(hex(&cipher::seal(&plus.key, &from_hex(raw), binding)?))
        }
        Object::Array(items) => Object::Array(
            items
                .iter()
                .map(|item| seal_strings(item, plus, binding))
                .collect::<Result<Vec<_>>>()?,
        ),
        Object::Dict(dict) => Object::Dict(Dict(
            dict.0
                .iter()
                .map(|(key, value)| Ok((key.clone(), seal_strings(value, plus, binding)?)))
                .collect::<Result<Vec<_>>>()?,
        )),
        other => other.clone(),
    })
}

fn open_strings(object: &Object, plus: &SecurePlus, binding: &[u8]) -> Result<Object> {
    Ok(match object {
        Object::HexString(raw) => {
            let opened = cipher::open(&plus.key, &from_hex(raw), binding)?;
            Object::HexString(hex(&opened))
        }
        Object::Array(items) => Object::Array(
            items
                .iter()
                .map(|item| open_strings(item, plus, binding))
                .collect::<Result<Vec<_>>>()?,
        ),
        Object::Dict(dict) => Object::Dict(Dict(
            dict.0
                .iter()
                .map(|(key, value)| Ok((key.clone(), open_strings(value, plus, binding)?)))
                .collect::<Result<Vec<_>>>()?,
        )),
        other => other.clone(),
    })
}

fn hex(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.extend_from_slice(format!("{byte:02X}").as_bytes());
    }
    out
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

fn hex_of(object: Option<&Object>) -> Option<Vec<u8>> {
    match object? {
        Object::HexString(raw) => Some(from_hex(raw)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(buffer: &mut [u8]) {
        for (index, byte) in buffer.iter_mut().enumerate() {
            *byte = index as u8;
        }
    }

    fn quick() -> KdfParams {
        KdfParams { memory_kib: 8, time: 1, lanes: 1 }
    }

    #[test]
    fn the_right_password_derives_a_key_and_the_wrong_one_does_not() {
        let plus = SecurePlus::new(b"open sesame", quick(), fixed).expect("new");
        let dict = plus.dictionary().expect("dictionary");

        SecurePlus::open(b"open sesame", &dict).expect("the right password was refused");
        assert!(
            SecurePlus::open(b"open sesamf", &dict).is_err(),
            "a near miss opened it"
        );
    }

    /// **The check is authenticated**, so editing the file cannot make a wrong
    /// password look right.
    #[test]
    fn an_altered_check_value_does_not_open() {
        let plus = SecurePlus::new(b"open sesame", quick(), fixed).expect("new");
        let mut dict = plus.dictionary().expect("dictionary");

        let Some(Object::HexString(raw)) = dict.get(b"Verify").cloned() else {
            panic!("no check value");
        };
        let mut bytes = from_hex(&raw);
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        dict.set(b"Verify", Object::HexString(hex(&bytes)));

        assert!(SecurePlus::open(b"open sesame", &dict).is_err());
    }

    /// A version this build does not know is refused rather than guessed at.
    #[test]
    fn an_unknown_subfilter_is_refused() {
        let plus = SecurePlus::new(b"open sesame", quick(), fixed).expect("new");
        let mut dict = plus.dictionary().expect("dictionary");
        dict.set(b"SubFilter", Object::Name(b"Pagify.SM4GCM.9".to_vec()));

        // Deliberately not `expect_err`: a key is not a thing to print, so
        // `SecurePlus` has no `Debug`.
        match SecurePlus::open(b"open sesame", &dict) {
            Ok(_) => panic!("it read a format from the future"),
            Err(problem) => assert!(problem.to_string().contains("cannot read"), "{problem}"),
        }
    }

    /// The cost is written into the file, so a document sealed today still
    /// opens after the default changes.
    #[test]
    fn the_cost_travels_with_the_document() {
        let plus = SecurePlus::new(b"open sesame", KdfParams { memory_kib: 16, time: 2, lanes: 1 }, fixed)
            .expect("new");
        let dict = plus.dictionary().expect("dictionary");
        assert_eq!(dict.get(b"M").and_then(Object::as_f64), Some(16.0));
        assert_eq!(dict.get(b"T").and_then(Object::as_f64), Some(2.0));
        SecurePlus::open(b"open sesame", &dict).expect("its own parameters did not open it");
    }

    #[test]
    fn an_empty_password_is_refused() {
        assert!(SecurePlus::new(b"", quick(), fixed).is_err());
    }
}
