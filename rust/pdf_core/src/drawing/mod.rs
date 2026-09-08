//! Drawings: DWG and DXF, read and looked at.
//!
//! A separate module from [`super::step`] on purpose. They share the scaffolding
//! — a session, a handle, a bitmap handed over the JNI boundary — and nothing
//! else: one holds a solid made of trimmed surfaces, the other a sheet of
//! strokes. Generalising over two examples would produce something that fits
//! neither.

pub mod model;
pub mod dwg;
pub mod dxf;
pub mod raster;
