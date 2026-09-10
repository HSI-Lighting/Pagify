//! Pagify Desktop's core — everything that is *about* the app but not about
//! egui.
//!
//! The open-document session, the markup layer, the command registry and
//! dispatch, and the coordinate conversions all live here, with no UI
//! dependency of any kind. That split is not ceremony: it is what makes the
//! command box testable without a window, which matters enormously for an app
//! whose entire interface is a command box.
//!
//! It is also the standing answer to the gravity that took SIMLUX's own shell
//! to 50,081 lines in one file. **If logic can be tested without a window, it
//! must live here.**

pub mod passphrase;
pub mod automate;
pub mod command;
pub mod commit;
pub mod markup;
pub mod measure;
pub mod organize;
pub mod page_space;
pub mod predefined;
pub mod pdfium;
pub mod reader;
pub mod recent;
pub mod session;
pub mod signatures;
pub mod tools;
pub mod models;
pub mod outlined_fonts;
pub mod verbs;

pub use command::{CommandBox, Dispatch};
pub use page_space::{AppPoint, PageSpace};
pub use session::{PageRaster, Session};
pub use verbs::Verb;

/// Crate version, for the about screen.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
