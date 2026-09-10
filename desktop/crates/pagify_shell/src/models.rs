//! Where the recognition models live, and whether they are the ones we expect.
//!
//! # Why a module rather than a path
//!
//! A model is **code in every sense that matters**. It is data fed straight into
//! a numeric runtime that will do whatever the weights tell it to; a corrupted
//! file is a crash and a substituted one is arbitrary behaviour inside the
//! process that is reading the user's documents. "Point at a directory and hope"
//! is fine for a developer and is not a way to ship.
//!
//! So: a known name, a known size, and a **known SHA-256**, checked before the
//! bytes reach the runtime. Nothing loads unverified.
//!
//! # Where it looks
//!
//! `PAGIFY_OCR_MODELS`, if it is set, is **the only** place looked at. An
//! explicit instruction that cannot be honoured is an error, not an invitation
//! to substitute something else: pointing at a directory and being given the
//! models from somewhere else is how you spend an afternoon testing a change
//! that was never loaded.
//!
//! Otherwise, first hit wins:
//!
//! 1. Beside the executable — `…/Contents/Resources/ocr` in a Mac bundle, or an
//!    `ocr` directory next to the binary elsewhere. This is what an installed
//!    app uses.
//! 2. The checkout's `third_party/ocr`, so a `cargo run` works with nothing set
//!    up.
//!
//! A directory is only accepted if the files in it are **the right files**. A
//! stale copy earlier in the list would otherwise shadow a good one later, and
//! the failure would be a recogniser that quietly reads worse.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// A model file, and what it must be.
pub struct Model {
    pub name: &'static str,
    /// Lowercase hex SHA-256 of the exact file this program was built against.
    pub sha256: &'static str,
    /// Bytes. Checked first because it is free, and a wrong length is the
    /// commonest corruption — a truncated download.
    pub bytes: u64,
}

/// The two files recognition needs.
///
/// From `ocrs` 0.10.4 / `rten` 0.21, the pair the M0 gate measured. Changing a
/// model means changing its hash here in the same commit, which is the point:
/// the constant is the record of which model this program was tested against.
pub const REQUIRED: &[Model] = &[
    Model {
        name: "text-detection.rten",
        sha256: "f15cfb56bd02c4bf478a20343986504a1f01e1665c2b3a0ad66340f054b1b5ca",
        bytes: 2_510_284,
    },
    Model {
        name: "text-recognition.rten",
        sha256: "e484866d4cce403175bd8d00b128feb08ab42e208de30e42cd9889d8f1735a6e",
        bytes: 9_716_568,
    },
];

/// Why a directory was not usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejected {
    Missing { name: String },
    WrongSize { name: String, expected: u64, found: u64 },
    WrongContents { name: String },
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rejected::Missing { name } => write!(f, "{name} is not there"),
            Rejected::WrongSize { name, expected, found } => {
                write!(f, "{name} is {found} bytes, expected {expected} — a truncated download?")
            }
            Rejected::WrongContents { name } => {
                write!(f, "{name} is not the model this build was tested against")
            }
        }
    }
}

/// The SHA-256 of a file, as lowercase hex.
///
/// Read in chunks rather than into a `Vec`: the recognition model is ten
/// megabytes, and a verifier that doubles the peak memory of the thing it is
/// guarding is a poor guard.
fn digest(path: &Path) -> std::io::Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Check one directory against [`REQUIRED`].
pub fn check(dir: &Path) -> Result<(), Rejected> {
    for model in REQUIRED {
        let path = dir.join(model.name);
        let found = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(_) => return Err(Rejected::Missing { name: model.name.into() }),
        };
        if found != model.bytes {
            return Err(Rejected::WrongSize {
                name: model.name.into(),
                expected: model.bytes,
                found,
            });
        }
        match digest(&path) {
            Ok(hex) if hex == model.sha256 => {}
            _ => return Err(Rejected::WrongContents { name: model.name.into() }),
        }
    }
    Ok(())
}

/// Every place worth looking, in order.
pub fn candidates() -> Vec<PathBuf> {
    // Set means "use these". On its own, and it stands or falls on its own.
    if let Ok(dir) = std::env::var("PAGIFY_OCR_MODELS") {
        return vec![PathBuf::from(dir)];
    }

    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // …/Pagify.app/Contents/MacOS/Pagify → …/Contents/Resources/ocr
            out.push(dir.join("../Resources/ocr"));
            out.push(dir.join("ocr"));
        }
    }
    out.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/ocr"));
    out
}

/// The first directory holding the models this build expects.
///
/// The rejections are returned alongside so a caller can say *why* nothing was
/// usable. "No models found" when a file is sitting right there, one byte
/// short, is the kind of message that costs an afternoon.
pub fn find() -> Result<PathBuf, Vec<(PathBuf, Rejected)>> {
    let mut refused = Vec::new();
    for dir in candidates() {
        match check(&dir) {
            Ok(()) => return Ok(dir),
            Err(why) => refused.push((dir, why)),
        }
    }
    Err(refused)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/ocr")
    }

    /// The hashes in `REQUIRED` must be the hashes of the files in the tree.
    ///
    /// This is the whole guarantee. If a model is replaced without its constant
    /// being updated in the same commit, everything else here is theatre.
    #[test]
    fn the_committed_models_are_the_ones_this_build_expects() {
        let dir = committed();
        if !dir.join(REQUIRED[0].name).exists() {
            eprintln!("skipping: no models in the tree");
            return;
        }
        assert_eq!(check(&dir), Ok(()), "the models in third_party/ocr do not match REQUIRED");
    }

    #[test]
    fn a_missing_file_is_named() {
        let empty = std::env::temp_dir().join("pagify-models-empty");
        std::fs::create_dir_all(&empty).expect("dir");
        match check(&empty) {
            Err(Rejected::Missing { name }) => assert_eq!(name, REQUIRED[0].name),
            other => panic!("expected a missing file, got {other:?}"),
        }
    }

    /// A truncated download is the commonest corruption, and the cheapest to
    /// catch — no need to read ten megabytes to find out.
    #[test]
    fn a_file_of_the_wrong_length_is_refused_before_it_is_read() {
        let dir = std::env::temp_dir().join("pagify-models-short");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join(REQUIRED[0].name), b"not a model").expect("write");

        match check(&dir) {
            Err(Rejected::WrongSize { name, found, .. }) => {
                assert_eq!(name, REQUIRED[0].name);
                assert_eq!(found, 11);
            }
            other => panic!("expected a size refusal, got {other:?}"),
        }
    }

    /// Right length, wrong bytes — which is the case a size check cannot see
    /// and the only one that matters for a substituted file.
    #[test]
    fn a_file_of_the_right_length_but_wrong_contents_is_refused() {
        let dir = std::env::temp_dir().join("pagify-models-swapped");
        std::fs::create_dir_all(&dir).expect("dir");
        for model in REQUIRED {
            std::fs::write(dir.join(model.name), vec![0u8; model.bytes as usize]).expect("write");
        }

        match check(&dir) {
            Err(Rejected::WrongContents { name }) => assert_eq!(name, REQUIRED[0].name),
            other => panic!("expected a contents refusal, got {other:?}"),
        }
    }

    /// `PAGIFY_OCR_MODELS` is an instruction, not a hint. Falling through to
    /// another directory when it cannot be honoured means testing a change that
    /// was never loaded.
    #[test]
    fn the_environment_variable_replaces_the_search_rather_than_leading_it() {
        let before = std::env::var("PAGIFY_OCR_MODELS").ok();

        std::env::remove_var("PAGIFY_OCR_MODELS");
        assert!(
            candidates().iter().any(|p| p.ends_with("third_party/ocr")),
            "with nothing set, the checkout should still be searched"
        );

        std::env::set_var("PAGIFY_OCR_MODELS", "/nowhere/at/all");
        let only = candidates();
        assert_eq!(only.len(), 1, "the variable was set and other places were still tried");
        assert!(only[0].ends_with("nowhere/at/all"));

        match before {
            Some(v) => std::env::set_var("PAGIFY_OCR_MODELS", v),
            None => std::env::remove_var("PAGIFY_OCR_MODELS"),
        }
    }
}
