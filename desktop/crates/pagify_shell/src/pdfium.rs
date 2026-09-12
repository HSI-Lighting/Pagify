//! Finding PDFium.
//!
//! `pdf_core` binds PDFium once, lazily, on the first document open, and lets
//! the host say where from. Android finds it by soname; iOS passes a bundle
//! path it only learns at runtime. **A desktop host has neither**, so the choice
//! has to be made explicitly — and this module is that choice, made once.
//!
//! Resolution order, most deliberate first:
//!
//! 1. `PAGIFY_PDFIUM_LIB` — left to `pdf_core`, which already reads it. Set it
//!    to test against a different PDFium without touching this file.
//! 2. Next to the executable. This is what packaging produces (build plan
//!    phase 12) and what a shipped build will actually use.
//! 3. The vendored slice in the Pagify tree. Development only, resolved at
//!    compile time from this crate's own location.

use std::path::PathBuf;
use std::sync::Once;

// The slice directory and the library's path within it, for the target this was
// built for. Deliberately a compile-time mapping rather than a runtime search:
// a slice that was never fetched should fail by *name*, at the point someone
// adds the target, rather than as a "library not found" months later.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SLICE: (&str, &str) = ("pdfium-mac-arm64", "lib/libpdfium.dylib");
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SLICE: (&str, &str) = ("pdfium-mac-x64", "lib/libpdfium.dylib");
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const SLICE: (&str, &str) = ("pdfium-win-x64", "bin/pdfium.dll");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SLICE: (&str, &str) = ("pdfium-linux-x64", "lib/libpdfium.so");

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
)))]
compile_error!(
    "No PDFium slice is mapped for this target. Add one to SLICE in \
     crates/pagify_shell/src/pdfium.rs and fetch the matching binary into \
     third_party/pdfium/ — see tools/fetch_pdfium.ps1, which currently fetches \
     the Apple slices only."
);

/// This workspace's own PDFium tree, resolved from the crate rather than the
/// working directory so a test runner and the app agree about where it is.
/// Populated by `tools/fetch_pdfium.sh`.
const VENDORED_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../third_party/pdfium");

/// The Pagify repo's tree, which carries the Apple slices the phone builds use.
/// Kept as a fallback so a checkout that has not run the fetch script still
/// builds on a Mac.
const PAGIFY_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../third_party/pdfium"
);

/// The library file this build expects, wherever it is found.
pub fn library_file_name() -> &'static str {
    SLICE.1.rsplit('/').next().unwrap_or(SLICE.1)
}

/// Where PDFium will be loaded from, or `None` to leave the decision to
/// `pdf_core` (an explicit `PAGIFY_PDFIUM_LIB`, or the system loader).
pub fn locate() -> Option<PathBuf> {
    if std::env::var_os("PAGIFY_PDFIUM_LIB").is_some() {
        return None;
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join(library_file_name());
            if beside.is_file() {
                return Some(beside);
            }
        }
    }

    for root in [VENDORED_ROOT, PAGIFY_ROOT] {
        let vendored = PathBuf::from(root).join(SLICE.0).join(SLICE.1);
        if vendored.is_file() {
            return Some(vendored);
        }
    }
    None
}

/// Point `pdf_core` at a PDFium build. Idempotent, and safe to call from every
/// entry point that might be the first one — which is why both the app and each
/// integration test call it rather than assuming the other did.
pub fn ensure_bound() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if let Some(path) = locate() {
            pdf_core::document::pdfium_doc::set_library_path(path.to_string_lossy().into_owned());
        }
    });
}

/// A one-line description of what this build will load, for diagnostics and the
/// about screen. A wrong PDFium is the kind of fault that presents as "some
/// pages render oddly", so it is worth being able to read the answer.
pub fn describe() -> String {
    match locate() {
        Some(path) => path.display().to_string(),
        None => match std::env::var("PAGIFY_PDFIUM_LIB") {
            Ok(path) => format!("{path} (PAGIFY_PDFIUM_LIB)"),
            Err(_) => "system loader".to_string(),
        },
    }
}
