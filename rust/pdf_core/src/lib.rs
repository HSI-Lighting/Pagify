//! Pagify's PDF engine.
//!
//! Everything expensive lives here: parsing, rasterisation, caching, and — from
//! roadmap phase 3 — editing and signing. The Kotlin layer above is presentation
//! only, and reaches this crate through the single JNI surface in [`jni_bridge`].
//!
//! ## Layering
//!
//! ```text
//!   jni_bridge   typed Java exceptions, panic containment, Android bitmap locking
//!        |
//!     engine     cache-aware orchestration (host-testable; no JNI types)
//!        |
//!   registry     ownership of open documents behind opaque handles
//!        |
//!   document     the `Document`/`Page` traits + the PDFium implementation
//!     render     pixel buffers, format conversion, the LRU page cache
//! ```
//!
//! Only `jni_bridge` is Android-specific; everything below it compiles and tests
//! on the host, which is where the majority of the test suite runs.

pub mod command;
pub mod document;
pub mod engine;
pub mod error;
pub mod plugins;
pub mod registry;
pub mod render;

#[cfg(target_os = "android")]
pub mod jni_bridge;

pub use document::{Document, DocumentMetadata, Page, PageSize, RenderRequest, Rotation};
pub use error::{PdfError, Result};
pub use render::{Bitmap, PixelOrder, RenderTarget};

/// Crate version, surfaced to the UI's about screen via `NativeBridge.nativeVersion()`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
