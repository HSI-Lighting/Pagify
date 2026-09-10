//! Just enough of a PDF file to change one object and leave the rest alone.
//!
//! # Why this exists
//!
//! Every edit to a page object goes into the file through PDFium's
//! `FPDFPage_GenerateContent`, which re-emits the **whole** content stream from
//! its own object model rather than editing the part that changed. Measured on
//! a real catalogue page, blanking one image that way rewrote text nobody had
//! touched — `sky‑light` came back as `sky -\r\nlight`, and the paragraph
//! around it drew differently. There is no flag to turn that off; it is how the
//! writer works.
//!
//! So Lock reaches past it. The file PDFium saved is read here, one object is
//! replaced, and everything else is copied through **byte for byte** — the
//! content streams that draw the text included, because they are never parsed,
//! never re-emitted, and never given the chance to differ.
//!
//! # Deliberately small
//!
//! This is not a PDF library and must not become one. It reads what PDFium
//! writes, which `examples/save_shape_probe.rs` measured on a 149-page
//! catalogue:
//!
//! ```text
//!   classic `xref` sections:            2
//!   /XRef  (cross-reference streams):   0
//!   /ObjStm (objects packed in streams): 0
//! ```
//!
//! Classic tables, plain objects. So cross-reference streams and object streams
//! are **refused rather than half-read** — a file this cannot account for is
//! handed back untouched, and Lock falls back to what it did before. The moment
//! that measurement stops holding, `a_file_this_cannot_read_is_refused_whole`
//! fails rather than a document being quietly corrupted.

pub mod cmap;
pub mod content;
pub mod embed;
pub mod encrypt;
pub mod hidden;
pub mod secure_plus;
pub mod sign;
pub mod timestamp;
pub mod validate;
mod object;
mod reader;

pub use object::{Dict, Object};
pub use reader::{write_object, write_stream, File};
