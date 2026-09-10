//! Encryption for Lock and Obfuscate — Protect phase 4.
//!
//! Platform-neutral and host-testable. **Nothing here touches a PDF**, which is
//! deliberate: the cryptography is the part that has to be right before
//! anything is stored, and mixing it with PDFium would make it untestable
//! without a document.
//!
//! # One cipher, where we own both ends
//!
//! SM4 is the cipher wherever Pagify writes the ciphertext and Pagify is the
//! only thing that will ever read it — the Lock blob, the Obfuscate field
//! values. One primitive keeps the audit surface small.
//!
//! It is **not** the cipher for the PDF's own `/Encrypt` dictionary. The
//! specification enumerates what may appear there — RC4, AES-128, AES-256, and
//! AES-GCM in PDF 2.0 — and SM4 is not among them. A PDF with SM4-encrypted
//! streams opens in Pagify and nowhere else, which is a proprietary container
//! wearing a `.pdf` extension rather than a protected document. The rule:
//!
//! > SM4 where Pagify owns both ends. AES where somebody else's reader is the
//! > other end.
//!
//! # Two levels of key
//!
//! ```text
//! passcode ──Argon2id+salt──▶ KEK ──unwraps──▶ DEK ──▶ every blob and field
//! ```
//!
//! Content keys are never derived from the passcode. Two levels cost nothing
//! now and are expensive to retrofit: changing a passcode re-wraps one key
//! rather than re-encrypting a document, two passcodes over the same content is
//! the DEK wrapped twice, and the memory-hard derivation runs once per open
//! rather than once per item.

pub mod cipher;
pub mod envelope;
pub mod ff1;
pub mod kdf;
pub mod ledger;
pub mod tweak;
pub mod vault;

pub use cipher::Secret;
pub use envelope::Envelope;
pub use ff1::{Alphabet, Ff1Sm4};
pub use kdf::{Argon2id, KdfParams, KeyDerivation};
pub use ledger::{Ledger, ObfuscatedField};
pub use tweak::{FieldKind, Source, check_coverage};
pub use vault::Vault;
