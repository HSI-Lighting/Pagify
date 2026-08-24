//! Making a document out of nothing.
//!
//! Distinct from `DocumentMut::insert_blank_page`, which adds a sheet to a
//! document somebody already has open. This is the other case: there is no
//! document yet, and the first thing that exists is the paper.
//!
//! Returns bytes rather than an open document. The caller has to write the file
//! somewhere the reader chose before it is a document at all, and handing back
//! bytes means the ordinary open path runs on the ordinary saved file — one path
//! through opening, not two.

use crate::document::pdfium_doc::pdfium;
use crate::document::{Color, PageSize};
use crate::error::{PdfError, Result};
use pdfium_render::prelude::*;
use std::os::raw::c_uint;

/// What is printed on the paper before anything is written on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ruling {
    /// Plain paper.
    None,
    /// Horizontal lines, as in a notebook.
    Lined,
    /// Squared paper.
    Grid,
    /// A dot at each grid intersection: the grid without the lines shouting.
    Dots,
}

impl Ruling {
    /// From the wire, where it arrives as a small integer.
    pub fn from_code(code: i32) -> Ruling {
        match code {
            1 => Ruling::Lined,
            2 => Ruling::Grid,
            3 => Ruling::Dots,
            _ => Ruling::None,
        }
    }
}

/// The gap between ruled lines, in points. 24pt is about 8.5mm, ordinary lined
/// paper.
const LINE_SPACING: f32 = 24.0;

/// The gap in squared and dotted paper: finer, because a square is read as a
/// unit of area and a line as a place to write.
const GRID_SPACING: f32 = 14.0;

/// How far the ruling stays clear of the paper's edge.
const RULING_MARGIN: f32 = 28.0;

/// Half a dot's width. Small enough to read as a point rather than a square.
const DOT_RADIUS: f32 = 0.6;

/// A new document of `pages` identical sheets, as PDF bytes.
pub fn blank_document(
    pages: usize,
    size: PageSize,
    fill: Option<Color>,
    ruling: Ruling,
) -> Result<Vec<u8>> {
    if pages == 0 {
        return Err(PdfError::InvalidArgument("a document needs a page".into()));
    }
    if size.width_pt <= 0.0 || size.height_pt <= 0.0 {
        return Err(PdfError::InvalidArgument("the sheet has no size".into()));
    }

    let pdfium = pdfium()?;
    let bindings = pdfium.bindings();
    let document = pdfium
        .create_new_pdf()
        .map_err(|e| PdfError::Pdfium(e.to_string()))?;
    let handle = document.handle();

    // The paper, and the ink it is ruled with. White paper is left as no
    // rectangle at all: an empty page already looks like white paper, and a
    // sheet-sized white rectangle is one more thing to go wrong for no gain.
    let paper = fill.unwrap_or(Color { r: 255, g: 255, b: 255, a: 255 });
    let ink = ruling_ink(paper);

    for index in 0..pages {
        // Safety: the page is created here, used here, and closed before the
        // iteration ends. Every handle passed to PDFium is checked for null.
        unsafe {
            let page = bindings.FPDFPage_New(
                handle,
                index as i32,
                size.width_pt as f64,
                size.height_pt as f64,
            );
            if page.is_null() {
                return Err(PdfError::Pdfium(format!("sheet {index} was not made")));
            }

            if fill.is_some() {
                let rect =
                    bindings.FPDFPageObj_CreateNewRect(0.0, 0.0, size.width_pt, size.height_pt);
                if rect.is_null() {
                    bindings.FPDF_ClosePage(page);
                    return Err(PdfError::Pdfium("the sheet had no colour".into()));
                }
                set_fill(bindings, rect, paper);
                // Filled, not stroked: a stroke would draw a border round the
                // edge of every sheet.
                bindings.FPDFPath_SetDrawMode(rect, 1, 0);
                bindings.FPDFPage_InsertObject(page, rect);
            }

            if let Err(e) = rule_page(bindings, page, size, ruling, ink) {
                bindings.FPDF_ClosePage(page);
                return Err(e);
            }

            if bindings.FPDFPage_GenerateContent(page) == 0 {
                bindings.FPDF_ClosePage(page);
                return Err(PdfError::Pdfium(format!(
                    "sheet {index} was made but its content was not written"
                )));
            }
            bindings.FPDF_ClosePage(page);
        }
    }

    let mut bytes = Vec::new();
    document
        .save_to_writer(&mut bytes)
        .map_err(|e| PdfError::Pdfium(e.to_string()))?;
    Ok(bytes)
}

/// Print the ruling onto a page.
///
/// Safety: `page` must be a live page of the document being built.
pub(crate) unsafe fn rule_page(
    bindings: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    size: PageSize,
    ruling: Ruling,
    ink: Color,
) -> Result<()> {
    let left = RULING_MARGIN;
    let right = size.width_pt - RULING_MARGIN;
    let bottom = RULING_MARGIN;
    let top = size.height_pt - RULING_MARGIN;
    if right <= left || top <= bottom {
        // A sheet smaller than its own margins. Leave it plain rather than
        // draw ruling that runs backwards.
        return Ok(());
    }

    match ruling {
        Ruling::None => {}
        Ruling::Lined => {
            for y in steps(bottom, top, LINE_SPACING) {
                line(bindings, page, left, y, right, y, ink)?;
            }
        }
        Ruling::Grid => {
            for y in steps(bottom, top, GRID_SPACING) {
                line(bindings, page, left, y, right, y, ink)?;
            }
            for x in steps(left, right, GRID_SPACING) {
                line(bindings, page, x, bottom, x, top, ink)?;
            }
        }
        Ruling::Dots => {
            for y in steps(bottom, top, GRID_SPACING) {
                for x in steps(left, right, GRID_SPACING) {
                    let dot = bindings.FPDFPageObj_CreateNewRect(
                        x - DOT_RADIUS,
                        y - DOT_RADIUS,
                        DOT_RADIUS * 2.0,
                        DOT_RADIUS * 2.0,
                    );
                    if dot.is_null() {
                        return Err(PdfError::Pdfium("a dot would not be made".into()));
                    }
                    set_fill(bindings, dot, ink);
                    bindings.FPDFPath_SetDrawMode(dot, 1, 0);
                    bindings.FPDFPage_InsertObject(page, dot);
                }
            }
        }
    }
    Ok(())
}

/// Every ruled position from `from` to `to`, `by` apart.
///
/// Counted rather than accumulated so a long page cannot drift: adding 14.0 to
/// itself sixty times is not sixty times 14.0.
fn steps(from: f32, to: f32, by: f32) -> impl Iterator<Item = f32> {
    let count = ((to - from) / by).floor().max(0.0) as usize;
    (0..=count).map(move |step| from + step as f32 * by)
}

/// Safety: `page` must be live.
unsafe fn line(
    bindings: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    ink: Color,
) -> Result<()> {
    let path = bindings.FPDFPageObj_CreateNewPath(x1, y1);
    if path.is_null() {
        return Err(PdfError::Pdfium("a rule would not be made".into()));
    }
    if bindings.FPDFPath_LineTo(path, x2, y2) == 0 {
        return Err(PdfError::Pdfium("a rule had no length".into()));
    }
    bindings.FPDFPageObj_SetStrokeColor(
        path,
        ink.r as c_uint,
        ink.g as c_uint,
        ink.b as c_uint,
        ink.a as c_uint,
    );
    bindings.FPDFPageObj_SetStrokeWidth(path, 0.5);
    // Stroked, not filled: a line has no inside.
    bindings.FPDFPath_SetDrawMode(path, 0, 1);
    bindings.FPDFPage_InsertObject(page, path);
    Ok(())
}

/// Safety: `object` must be a live page object.
unsafe fn set_fill(bindings: &dyn PdfiumLibraryBindings, object: FPDF_PAGEOBJECT, c: Color) {
    bindings.FPDFPageObj_SetFillColor(
        object,
        c.r as c_uint,
        c.g as c_uint,
        c.b as c_uint,
        c.a as c_uint,
    );
}

/// A ruling colour that shows on this paper without competing with what is
/// written on it.
///
/// Mixed towards the paper rather than set to a fixed grey: a fixed grey is
/// invisible on the grey sheet and glaring on the black one, and the ruling is
/// supposed to be the quietest thing on the page whichever paper was chosen.
pub(crate) fn ruling_ink(paper: Color) -> Color {
    let luminance = 0.2126 * paper.r as f32 + 0.7152 * paper.g as f32 + 0.0722 * paper.b as f32;
    let towards: f32 = if luminance > 128.0 { 0.0 } else { 255.0 };
    let mix = 0.28;
    Color {
        r: blend(paper.r, towards, mix),
        g: blend(paper.g, towards, mix),
        b: blend(paper.b, towards, mix),
        a: 255,
    }
}

fn blend(from: u8, to: f32, amount: f32) -> u8 {
    (from as f32 + (to - from as f32) * amount)
        .round()
        .clamp(0.0, 255.0) as u8
}
