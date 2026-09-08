//! Reading a STEP file and turning it into a picture.
//!
//! # Where the work is
//!
//! Parsing STEP is a solved problem with a library; turning a boundary
//! representation into triangles is not, and is what this module is. The
//! division is deliberate:
//!
//! ```text
//! step-io          our adapter        our tessellator     our rasteriser
//!   parse    →    typed B-Rep    →      triangles     →    RGBA pixels
//! ```
//!
//! The last arrow ends at the same bitmap handover the PDF renderer already
//! uses, so showing a 3D model needs no new platform surface on either phone:
//! it is another producer of pixels for a `Bitmap` the core writes into.
//!
//! # No new dependency reaches past the adapter
//!
//! `step-io` describes itself as experimental and warns of breaking changes at
//! any time. It is pinned exactly and confined to the adapter, so a breaking
//! release costs one file rather than the geometry.

pub mod adapt;
pub mod audit;
pub mod camera;
pub mod curve;
pub mod model;
pub mod orient;
pub mod project;
pub mod raster;
pub mod session;
pub mod tessellate;
