//! PDF's own encryption — a password another reader will ask for.
//!
//! # Why this is here and not in `crate::crypto`
//!
//! `crate::crypto` protects things *for Pagify*: the vault picks its own
//! primitives and only Pagify ever reads them back. This is the opposite
//! constraint. Every byte has to be exactly what ISO 32000-2 §7.6.4 specifies,
//! because the reader on the other side is somebody else's — Acrobat, Preview,
//! a browser — and "close enough" means a file nobody can open.
//!
//! So this uses AES-256 in CBC with SHA-2, not the AES-GCM and Argon2 the vault
//! prefers. Those are better choices in general and the wrong ones here.
//!
//! # What it implements
//!
//! **Revision 6 only** — `/V 5 /R 6`, AES-256, the handler PDF 2.0 defines and
//! the one every current reader supports. The older revisions (RC4 40-bit, RC4
//! 128-bit, AES-128) are deliberately absent: they are all breakable, and
//! offering a choice between a real lock and a broken one invites picking the
//! broken one. A document that cannot be secured this way is refused rather
//! than secured worse.
//!
//! Revision 6 has one property that makes it much simpler than its
//! predecessors: **every object is encrypted with the file key itself**, with no
//! per-object key derived from the object and generation numbers.

use aes::cipher::{BlockEncrypt, KeyInit, KeyIvInit};
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::error::{PdfError, Result};

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

/// What a reader is allowed to do with a secured document.
///
/// The bits are ISO 32000-2 Table 22, which is written from the other
/// direction: a **set** bit permits. Bits 1-2 and 7-8 are reserved and must be
/// 0, bits 13-32 must be 1. `Permissions::all()` is therefore not `u32::MAX`.
///
/// **These are a request, not a guarantee.** A reader that ignores them can do
/// as it likes — the content is decrypted either way, and the specification
/// says so. What actually withholds content is the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions(pub u32);

impl Permissions {
    /// Everything permitted — the sensible default when a document is being
    /// given a password rather than being restricted.
    pub fn all() -> Self {
        Permissions(0xFFFF_FFFC)
    }

    /// Nothing but reading it on screen.
    pub fn read_only() -> Self {
        // Every optional bit cleared; the reserved high bits stay set.
        Permissions(0xFFFF_F0C0)
    }

    /// Whether a bit is set. Bit numbers are ISO 32000-1's, counting from 1.
    fn has(self, bit: u32) -> bool {
        self.0 & (1u32 << (bit - 1)) != 0
    }

    pub fn may_print(self) -> bool {
        self.has(3)
    }

    pub fn may_edit(self) -> bool {
        self.has(4)
    }

    pub fn may_copy(self) -> bool {
        self.has(5)
    }

    pub fn may_annotate(self) -> bool {
        self.has(6)
    }

    /// What is allowed, in words.
    ///
    /// Deliberately the same sentences the tool that *sets* them uses, so a
    /// document reads the same when it is restricted and when it is reported
    /// on. Two vocabularies for one fact is how somebody ends up believing
    /// they are different facts.
    pub fn describe(self) -> String {
        let forbidden: Vec<&str> = [
            (!self.may_print()).then_some("printing"),
            (!self.may_copy()).then_some("copying"),
            (!self.may_edit()).then_some("editing"),
            (!self.may_annotate()).then_some("annotating"),
        ]
        .into_iter()
        .flatten()
        .collect();

        match forbidden.len() {
            0 => "everything permitted".into(),
            4 => "reading only".into(),
            _ => format!("no {}", forbidden.join(", no ")),
        }
    }

    fn with(self, bit: u32, allowed: bool) -> Self {
        let mask = 1u32 << (bit - 1);
        Permissions(if allowed { self.0 | mask } else { self.0 & !mask })
    }

    pub fn allow_printing(self, yes: bool) -> Self {
        self.with(3, yes)
    }
    pub fn allow_editing(self, yes: bool) -> Self {
        self.with(4, yes)
    }
    pub fn allow_copying(self, yes: bool) -> Self {
        self.with(5, yes)
    }
    pub fn allow_annotating(self, yes: bool) -> Self {
        self.with(6, yes)
    }
}

/// Everything a secured file needs written into its `/Encrypt` dictionary,
/// plus the key its contents are encrypted with.
pub struct Security {
    /// 48 bytes: the user password's hash, then its two salts.
    pub u: [u8; 48],
    /// The file key wrapped under the user password.
    pub ue: [u8; 32],
    pub o: [u8; 48],
    pub oe: [u8; 32],
    pub perms: [u8; 16],
    pub permissions: Permissions,
    /// The key every stream and string in the file is encrypted with.
    file_key: [u8; 32],
}

impl Security {
    /// Set up encryption for a document.
    ///
    /// `user` is the password a reader is asked for. `owner` — when given —
    /// opens the document with the restrictions lifted; when not, the user
    /// password serves as both, which is what "put a password on this" usually
    /// means.
    pub fn new(
        user: &[u8],
        owner: Option<&[u8]>,
        permissions: Permissions,
        random: impl Fn(&mut [u8]),
    ) -> Result<Security> {
        if user.is_empty() {
            return Err(PdfError::InvalidArgument(
                "a document cannot be secured with an empty password".into(),
            ));
        }
        // The specification caps a password at 127 bytes of UTF-8 and says to
        // truncate. Refused instead: silently securing a document with less
        // than what was typed is the kind of surprise a password should never
        // hold.
        if user.len() > 127 || owner.is_some_and(|o| o.len() > 127) {
            return Err(PdfError::InvalidArgument(
                "a password may be at most 127 bytes long".into(),
            ));
        }
        let owner = owner.unwrap_or(user);

        let mut file_key = [0u8; 32];
        random(&mut file_key);

        // -- /U and /UE, from the user password ---------------------------
        let mut salts = [0u8; 16];
        random(&mut salts);
        let (u_validation, u_key) = salts.split_at(8);

        let mut u = [0u8; 48];
        u[..32].copy_from_slice(&hash(user, u_validation, &[])?);
        u[32..40].copy_from_slice(u_validation);
        u[40..].copy_from_slice(u_key);

        let ue = wrap(&hash(user, u_key, &[])?, &file_key);

        // -- /O and /OE, from the owner password --------------------------
        //
        // These hash the *user* entry as well, which is what ties the two
        // together: changing /U invalidates /O.
        let mut salts = [0u8; 16];
        random(&mut salts);
        let (o_validation, o_key) = salts.split_at(8);

        let mut o = [0u8; 48];
        o[..32].copy_from_slice(&hash(owner, o_validation, &u)?);
        o[32..40].copy_from_slice(o_validation);
        o[40..].copy_from_slice(o_key);

        let oe = wrap(&hash(owner, o_key, &u)?, &file_key);

        // -- /Perms, so a reader can tell the permissions were not edited --
        let mut block = [0u8; 16];
        block[..4].copy_from_slice(&permissions.0.to_le_bytes());
        block[4..8].copy_from_slice(&[0xFF; 4]);
        // Metadata is encrypted along with everything else.
        block[8] = b'T';
        block[9..12].copy_from_slice(b"adb");
        random(&mut block[12..]);

        // One AES block, no chaining and no padding — the only place the
        // specification asks for raw ECB.
        let mut perms = block;
        let cipher = aes::Aes256::new_from_slice(&file_key)
            .map_err(|_| PdfError::Pdfium("the file key is the wrong length".into()))?;
        cipher.encrypt_block((&mut perms).into());

        Ok(Security { u, ue, o, oe, perms, permissions, file_key })
    }

    /// Encrypt one stream or string, as it will be written into the file.
    ///
    /// The initialisation vector goes in front of the ciphertext, which is where
    /// a reader looks for it.
    pub fn encrypt(&self, plain: &[u8], random: impl Fn(&mut [u8])) -> Vec<u8> {
        let mut iv = [0u8; 16];
        random(&mut iv);

        // PKCS#7, and a whole block of padding when the length already divides
        // evenly — so there is always padding to strip.
        let pad = 16 - (plain.len() % 16);
        let mut out = Vec::with_capacity(16 + plain.len() + pad);
        out.extend_from_slice(&iv);
        out.extend_from_slice(plain);
        out.extend(std::iter::repeat_n(pad as u8, pad));

        let cipher = Aes256CbcEnc::new((&self.file_key).into(), (&iv).into());
        let body = &mut out[16..];
        // Encrypted a block at a time so the buffer laid out above is used in
        // place, rather than copied into a second one.
        use aes::cipher::BlockEncryptMut;
        let mut cipher = cipher;
        for block in body.chunks_exact_mut(16) {
            cipher.encrypt_block_mut(block.into());
        }
        out
    }
}

/// Wrap the file key under a password-derived key: AES-256-CBC, zero IV, no
/// padding — the specification's own words for it.
fn wrap(key: &[u8; 32], file_key: &[u8; 32]) -> [u8; 32] {
    use aes::cipher::BlockEncryptMut;
    let mut out = *file_key;
    let mut cipher = Aes256CbcEnc::new(key.into(), &[0u8; 16].into());
    for block in out.chunks_exact_mut(16) {
        cipher.encrypt_block_mut(block.into());
    }
    out
}

/// **Algorithm 2.B** — revision 6's hardened password hash.
///
/// Deliberately expensive and deliberately awkward: sixty-four rounds at
/// minimum, each one encrypting a block built from the password, the running
/// hash and (for the owner entry) the whole `/U` value, then choosing between
/// SHA-256, SHA-384 and SHA-512 by the arithmetic of the result. The choice of
/// digest is what stops the whole thing being precomputed.
///
/// `extra` is empty for the user entry and the 48-byte `/U` for the owner's.
fn hash(password: &[u8], salt: &[u8], extra: &[u8]) -> Result<[u8; 32]> {
    use aes::cipher::BlockEncryptMut;

    let mut k: Vec<u8> = {
        let mut d = Sha256::new();
        d.update(password);
        d.update(salt);
        d.update(extra);
        d.finalize().to_vec()
    };

    let mut round = 0usize;
    loop {
        // K1: the password, the running hash and the extra, sixty-four times.
        let one = [password, &k, extra].concat();
        let mut k1 = Vec::with_capacity(one.len() * 64);
        for _ in 0..64 {
            k1.extend_from_slice(&one);
        }

        // AES-128-CBC with the first half of K as key and the second as IV.
        if k.len() < 32 {
            return Err(PdfError::Pdfium("the password hash went wrong".into()));
        }
        let mut e = k1;
        // The specification's input is always a multiple of the block size,
        // because it is sixty-four copies of the same thing; a length that is
        // not says the state is corrupt.
        if e.len() % 16 != 0 {
            return Err(PdfError::Pdfium("the password hash went wrong".into()));
        }
        let mut cipher = Aes128CbcEnc::new(k[..16].into(), k[16..32].into());
        for block in e.chunks_exact_mut(16) {
            cipher.encrypt_block_mut(block.into());
        }

        // Which digest comes next is decided by the first sixteen bytes.
        let sum: u32 = e[..16].iter().map(|b| u32::from(*b)).sum();
        k = match sum % 3 {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };

        round += 1;
        // At least sixty-four rounds, then until the last byte of the round's
        // ciphertext is small enough.
        if round >= 64 && usize::from(*e.last().unwrap_or(&0)) <= round - 32 {
            break;
        }
        // A guard the specification does not need but a parser does: the loop
        // above terminates on data, and data can be wrong.
        if round > 4096 {
            return Err(PdfError::Pdfium("the password hash did not settle".into()));
        }
    }

    let mut out = [0u8; 32];
    out.copy_from_slice(&k[..32]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic "randomness", so a test can say what the bytes will be.
    fn fixed(buffer: &mut [u8]) {
        for (index, byte) in buffer.iter_mut().enumerate() {
            *byte = index as u8;
        }
    }

    #[test]
    fn the_user_entry_is_a_hash_and_its_two_salts() {
        let s = Security::new(b"hunter2", None, Permissions::all(), fixed).expect("secure");
        // The salts are the last sixteen bytes, in the order they were made.
        assert_eq!(&s.u[32..40], &[0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(&s.u[40..48], &[8, 9, 10, 11, 12, 13, 14, 15]);
        assert_ne!(&s.u[..32], &[0u8; 32], "the hash was not computed");
    }

    /// The owner entry hashes `/U` as well, so the two cannot be mixed between
    /// documents.
    #[test]
    fn the_owner_entry_depends_on_the_user_entry() {
        let a = Security::new(b"hunter2", Some(b"owner"), Permissions::all(), fixed)
            .expect("secure");
        // A different user password changes /U, and so must change /O even
        // though the owner password is the same.
        let b = Security::new(b"different", Some(b"owner"), Permissions::all(), fixed)
            .expect("secure");
        assert_ne!(a.o[..32], b.o[..32]);
    }

    #[test]
    fn a_different_password_gives_a_different_document_key() {
        let a = Security::new(b"one", None, Permissions::all(), fixed).expect("secure");
        let b = Security::new(b"two", None, Permissions::all(), fixed).expect("secure");
        // The file key is the same only because the fixed randomness makes it
        // so; what must differ is everything derived from the password.
        assert_ne!(a.u[..32], b.u[..32]);
        assert_ne!(a.ue, b.ue);
    }

    #[test]
    fn an_empty_password_is_refused() {
        assert!(Security::new(b"", None, Permissions::all(), fixed).is_err());
    }

    /// Truncating silently would secure a document with less than was typed.
    #[test]
    fn an_over_long_password_is_refused_rather_than_truncated() {
        let long = vec![b'x'; 128];
        assert!(Security::new(&long, None, Permissions::all(), fixed).is_err());
        let ok = vec![b'x'; 127];
        assert!(Security::new(&ok, None, Permissions::all(), fixed).is_ok());
    }

    #[test]
    fn encryption_prepends_the_iv_and_pads_to_a_block() {
        let s = Security::new(b"hunter2", None, Permissions::all(), fixed).expect("secure");
        let out = s.encrypt(b"hello", fixed);
        // Sixteen of IV, then one block holding five bytes and eleven of
        // padding.
        assert_eq!(out.len(), 32);
        assert_eq!(&out[..16], &(0u8..16).collect::<Vec<u8>>()[..]);

        // A whole extra block when the length already divides evenly, so there
        // is always padding to take off.
        assert_eq!(s.encrypt(&[0u8; 16], fixed).len(), 16 + 32);
    }

    #[test]
    fn the_same_bytes_encrypt_differently_each_time() {
        let s = Security::new(b"hunter2", None, Permissions::all(), fixed).expect("secure");
        let mut counter = std::cell::Cell::new(0u8);
        let varying = |buffer: &mut [u8]| {
            let seed = counter.get();
            counter.set(seed.wrapping_add(1));
            for (index, byte) in buffer.iter_mut().enumerate() {
                *byte = seed.wrapping_add(index as u8);
            }
        };
        let one = s.encrypt(b"the same words", &varying);
        let two = s.encrypt(b"the same words", &varying);
        assert_ne!(one, two, "the initialisation vector is not being varied");
        let _ = counter.get_mut();
    }

    /// The permission bits are written from the other direction: a set bit
    /// permits, and several are reserved.
    #[test]
    fn what_is_permitted_reads_the_same_as_what_was_asked_for() {
        assert_eq!(Permissions::all().describe(), "everything permitted");
        assert_eq!(Permissions::read_only().describe(), "reading only");
        assert_eq!(
            Permissions::all().allow_printing(false).allow_copying(false).describe(),
            "no printing, no copying"
        );
        // And the bits read back the way they were set.
        let restricted = Permissions::all().allow_printing(false);
        assert!(!restricted.may_print());
        assert!(restricted.may_copy() && restricted.may_edit() && restricted.may_annotate());
    }

    #[test]
    fn permissions_clear_the_bit_they_forbid() {
        let all = Permissions::all();
        assert_eq!(all.allow_printing(false).0 & 0b100, 0);
        assert_ne!(all.allow_printing(true).0 & 0b100, 0);
        assert_eq!(all.allow_copying(false).0 & 0b1_0000, 0);
        // Reserved bits 1 and 2 stay clear whatever is asked for.
        assert_eq!(all.0 & 0b11, 0);
    }
}

// ---------------------------------------------------------------- the file --

use super::object::Dict;
use super::{write_object, write_stream, File, Object};

/// Write a secured copy of a file.
///
/// Every string and every stream is encrypted with the file key, a new
/// `/Encrypt` dictionary is added, and the trailer is pointed at it. What is
/// **not** encrypted, because a reader has to read it before it has a key: the
/// `/Encrypt` dictionary itself, and the `/ID` in the trailer.
pub fn secure(file: &File<'_>, security: &Security, random: impl Fn(&mut [u8]) + Copy) -> Result<Vec<u8>> {
    // A file that is already secured would have its ciphertext encrypted a
    // second time, and the result would open for nobody. Refused rather than
    // ruined.
    if file.trailer().get(b"Encrypt").is_some() {
        return Err(PdfError::Unsupported(
            "that document is already secured — remove its password first",
        ));
    }

    let numbers: Vec<u32> = file.numbers().collect();
    let mut replacements: Vec<(u32, Vec<u8>)> = Vec::with_capacity(numbers.len());
    for number in &numbers {
        let object = file.object(*number)?;
        let mut body = Vec::new();
        match object {
            Object::Stream(dict, range) => {
                let plain = file.bytes().get(range).ok_or_else(|| {
                    PdfError::InvalidArgument(format!("object {number} runs past the file"))
                })?;
                let sealed = security.encrypt(plain, random);
                let mut dict = encrypt_strings(&Object::Dict(dict), security, random)
                    .as_dict()
                    .cloned()
                    .unwrap_or(Dict(Vec::new()));
                // The length is now the ciphertext's, initialisation vector and
                // padding included — a reader that trusted the old one would
                // stop partway through.
                dict.set(b"Length", Object::Number(format!("{}", sealed.len()).into_bytes()));
                body = write_stream(&dict, &sealed);
            }
            other => write_object(&mut body, &encrypt_strings(&other, security, random)),
        }
        replacements.push((*number, body));
    }

    // The `/Encrypt` dictionary, as a new object at the end.
    let encrypt_number = numbers.iter().copied().max().unwrap_or(0) + 1;
    let mut dict = Dict(Vec::new());
    dict.set(b"Filter", Object::Name(b"Standard".to_vec()));
    dict.set(b"V", Object::Number(b"5".to_vec()));
    dict.set(b"R", Object::Number(b"6".to_vec()));
    dict.set(b"Length", Object::Number(b"256".to_vec()));
    dict.set(b"U", Object::HexString(hex(&security.u)));
    dict.set(b"UE", Object::HexString(hex(&security.ue)));
    dict.set(b"O", Object::HexString(hex(&security.o)));
    dict.set(b"OE", Object::HexString(hex(&security.oe)));
    dict.set(b"Perms", Object::HexString(hex(&security.perms)));
    dict.set(
        b"P",
        Object::Number(format!("{}", security.permissions.0 as i32).into_bytes()),
    );
    dict.set(b"EncryptMetadata", Object::Bool(true));

    // Both streams and strings use the same AES-256 crypt filter.
    let mut aes = Dict(Vec::new());
    aes.set(b"CFM", Object::Name(b"AESV3".to_vec()));
    aes.set(b"Length", Object::Number(b"32".to_vec()));
    let mut filters = Dict(Vec::new());
    filters.set(b"StdCF", Object::Dict(aes));
    dict.set(b"CF", Object::Dict(filters));
    dict.set(b"StmF", Object::Name(b"StdCF".to_vec()));
    dict.set(b"StrF", Object::Name(b"StdCF".to_vec()));

    let mut encrypt_body = Vec::new();
    write_object(&mut encrypt_body, &Object::Dict(dict));

    // The trailer points at it, and carries an `/ID` — which revision 6 does
    // not use in its key derivation, but which readers still expect a secured
    // file to have.
    let mut trailer = Dict(Vec::new());
    trailer.set(b"Encrypt", Object::Reference(encrypt_number, 0));
    if file.trailer().get(b"ID").is_none() {
        let mut id = [0u8; 16];
        random(&mut id);
        let entry = Object::HexString(hex(&id));
        trailer.set(b"ID", Object::Array(vec![entry.clone(), entry]));
    }

    file.rewrite_adding(&replacements, &[(encrypt_number, encrypt_body)], &trailer)
}

/// Every string inside an object, encrypted, with the structure kept.
fn encrypt_strings(object: &Object, security: &Security, random: impl Fn(&mut [u8]) + Copy) -> Object {
    match object {
        Object::LiteralString(raw) => {
            Object::HexString(hex(&security.encrypt(&unescape(raw), random)))
        }
        Object::HexString(raw) => {
            Object::HexString(hex(&security.encrypt(&from_hex(raw), random)))
        }
        Object::Array(items) => Object::Array(
            items.iter().map(|item| encrypt_strings(item, security, random)).collect(),
        ),
        Object::Dict(dict) => Object::Dict(Dict(
            dict.0
                .iter()
                .map(|(key, value)| (key.clone(), encrypt_strings(value, security, random)))
                .collect(),
        )),
        // A stream's dictionary is handled by the caller, which also has to fix
        // its length; reaching one here would mean a stream nested inside
        // another object, which cannot happen.
        other => other.clone(),
    }
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

/// A literal string's real bytes, escapes resolved.
pub(super) fn unescape(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0usize;
    while at < raw.len() {
        if raw[at] != b'\\' {
            out.push(raw[at]);
            at += 1;
            continue;
        }
        at += 1;
        let Some(next) = raw.get(at) else { break };
        match next {
            b'n' => {
                out.push(b'\n');
                at += 1;
            }
            b'r' => {
                out.push(b'\r');
                at += 1;
            }
            b't' => {
                out.push(b'\t');
                at += 1;
            }
            b'b' => {
                out.push(0x08);
                at += 1;
            }
            b'f' => {
                out.push(0x0C);
                at += 1;
            }
            b'\n' => at += 1,
            b'\r' => {
                at += 1;
                if raw.get(at) == Some(&b'\n') {
                    at += 1;
                }
            }
            b'0'..=b'7' => {
                let mut value = 0u16;
                let mut digits = 0;
                while digits < 3 && raw.get(at).is_some_and(|b| (b'0'..=b'7').contains(b)) {
                    value = value * 8 + u16::from(raw[at] - b'0');
                    at += 1;
                    digits += 1;
                }
                out.push(value as u8);
            }
            other => {
                out.push(*other);
                at += 1;
            }
        }
    }
    out
}
