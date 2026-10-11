//! The document checks behind `preflight` — the Conversion tab's Preflight.
//!
//! **What this build can actually see.** Whether a font is embedded, and how
//! many pixels an image has over the box it was drawn in, are facts the
//! engine already answers; PDF/A conformance, colour spaces, transparency and
//! metadata validity are not, and a check that cannot see a thing must not
//! report it as passed. The scope sentence says so in the answer.
//!
//! A shell function over `Session`, so it needs no window and can be tested
//! against the committed fixtures.

use crate::Session;

/// Below this, an image is soft on paper: 150 dpi is the usual print floor.
const PRINT_DPI_FLOOR: f32 = 150.0;

/// One line of the preflight, and whether it is a problem.
pub struct Finding {
    /// Which page it is about, one-based in [`Finding::line`].
    pub page: usize,
    pub problem: bool,
    pub line: String,
}

/// Every finding the engine can make, page by page: faces that are not in
/// the file, and pictures drawn below the print floor.
pub fn findings(session: &Session) -> pdf_core::Result<Vec<Finding>> {
    let mut out = Vec::new();
    for page in 0..session.page_count()? {
        // Fonts: a face that is not embedded renders differently wherever it
        // is missing. One line per face per page — a page of runs in the
        // same missing face is one problem, not forty.
        let names = session.run_font_names(page)?;
        let mut flagged: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (object, name) in &names {
            if session.run_font_is_embedded(page, *object)? {
                continue;
            }
            if flagged.insert(name.clone()) {
                out.push(Finding {
                    page,
                    problem: true,
                    line: format!(
                        "page {}: {name} is not embedded — it will look different on a \
                         machine that does not have it.",
                        page + 1
                    ),
                });
            }
        }

        // Pictures: pixels over the box, against the print floor. A box with
        // no width says nothing and is skipped rather than divided by.
        for image in session.images_on(page)? {
            let width_pt = (image.rect.right - image.rect.left).abs();
            if width_pt <= 0.0 {
                continue;
            }
            let dpi = image.pixel_width as f32 * 72.0 / width_pt;
            if dpi < PRINT_DPI_FLOOR {
                out.push(Finding {
                    page,
                    problem: true,
                    line: format!(
                        "page {}: an image holds {:.0} dpi over the box it was drawn in — \
                         soft if printed, where 150 is the usual floor. It carries {}×{} \
                         pixels.",
                        page + 1,
                        dpi,
                        image.pixel_width,
                        image.pixel_height
                    ),
                });
            }
        }
    }
    Ok(out)
}
