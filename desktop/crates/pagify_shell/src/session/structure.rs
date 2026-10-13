//! A page's tags: the structure tree PDFium reads back.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.

use super::*;

impl Session {
    /// A page's structure tree, as the Tags panel lists it. Empty for an
    /// untagged page.
    pub fn structure_outline(&self, page: usize) -> Result<Vec<pdf_core::document::TagNode>> {
        registry::with_session(self.handle, |s| s.document.structure_outline(page))
    }

    /// The page's tags in reading order — every element's kind, depth
    /// first — or `None` for an untagged page.
    pub fn structure_sequence(&self, page: usize) -> Result<Option<Vec<String>>> {
        let outline = self.structure_outline(page)?;
        if outline.is_empty() {
            return Ok(None);
        }
        let mut sequence = Vec::new();
        for node in &outline {
            walk_kinds(node, &mut sequence);
        }
        Ok(Some(sequence))
    }

    /// The alternate-text reading: how many `Figure` elements carry `/Alt`,
    /// and which ones do not, as `(page, kind)`.
    ///
    /// A figure is what a screen reader *says something else for*; text
    /// elements it reads directly. The reading is deliberately about
    /// figures only — reporting "no alt text" on a heading would be noise.
    pub fn alt_text_report(&self, pages: &[usize]) -> Result<(usize, Vec<(usize, String)>)> {
        let mut with_alt = 0usize;
        let mut missing = Vec::new();
        for &page in pages {
            for node in self.structure_outline(page)? {
                walk_alt_text(&node, page, &mut with_alt, &mut missing);
            }
        }
        Ok((with_alt, missing))
    }
}

fn walk_kinds(node: &pdf_core::document::TagNode, out: &mut Vec<String>) {
    out.push(node.kind.clone());
    for child in &node.children {
        walk_kinds(child, out);
    }
}

fn walk_alt_text(
    node: &pdf_core::document::TagNode,
    page: usize,
    with_alt: &mut usize,
    missing: &mut Vec<(usize, String)>,
) {
    if node.kind == "Figure" {
        if node.alt.as_deref().is_some_and(|alt| !alt.trim().is_empty()) {
            *with_alt += 1;
        } else {
            missing.push((page, node.kind.clone()));
        }
    }
    for child in &node.children {
        walk_alt_text(child, page, with_alt, missing);
    }
}
