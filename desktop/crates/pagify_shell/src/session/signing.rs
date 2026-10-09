//! Signing and form marks: stamping drawn lines and boxes, placing picture
//! and drawn signatures (and adjusting them), the marks each page carries,
//! applying the saved-signature pass, validating, and signing the document
//! itself — plus `permissions`, which signing reads.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// Rule a line, as a form is filled in by hand.
    pub fn stamp_line(
        &self,
        page: usize,
        from: pdf_core::document::Point,
        to: pdf_core::document::Point,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?
                .stamp_line(page, from, to)
        })
    }

    /// Draw a box around something, as a form is filled in by hand.
    pub fn stamp_box(&self, page: usize, area: pdf_core::document::Rect) -> Result<()> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?
                .stamp_box(page, area)
        })
    }

    /// What this document permits, when it is encrypted. `None` when it has no
    /// password — see [`pdf_core::document::Document::permissions`].
    pub fn permissions(&self) -> Option<pdf_core::pdf::encrypt::Permissions> {
        registry::with_session(self.handle, |s| Ok(s.document.permissions())).unwrap_or(None)
    }

    /// Place ink and record that it is a signature rather than a drawing.
    ///
    /// Both halves here rather than at the caller, because the index the mark
    /// goes on is PDFium's and is only knowable in between: the engine appends,
    /// so the new annotation is the last one on the page. Doing it in one place
    /// means there is no window in which a placed signature is not one.
    pub fn place_signature(
        &self,
        page: usize,
        strokes: Vec<Vec<pdf_core::document::Point>>,
        color: pdf_core::document::Color,
        width: f32,
        name: &str,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            let doc = s
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?;
            let index =
                doc.add_annotation(page, &pdf_core::document::Annotation::Ink { strokes, color, width })?;
            doc.mark_as_signature(page, index, name)
        })
    }

    /// The same, for a signature that is a picture — see
    /// [`pdf_core::document::Annotation::Image`].
    ///
    /// `rgba` may carry real alpha (see [`crate::signature_extract`]) or be
    /// uniformly opaque (any upload made before that existed). Two different
    /// things happen to it, for two different moments:
    ///
    /// - The annotation itself — what is actually placed, and what PDFium's
    ///   own picture object shows while it sits there unapplied — gets a
    ///   *flattened* copy, composited pixel for pixel against this page's
    ///   own rendered pixels at `rect` (see
    ///   [`crate::signatures::composite_onto_image`]), because the
    ///   mechanism a picture is placed through cannot carry alpha itself
    ///   and the page underneath is not known any earlier than this call.
    ///   Indistinguishable from real transparency for as long as the page
    ///   does not change after this moment.
    /// - The *original*, still carrying real alpha, is kept alongside it —
    ///   see [`pdf_core::document::DocumentMut::remember_image_alpha`] — so
    ///   that applying this signature later, in this same session, can burn
    ///   in a real soft mask instead of the flattened approximation, which
    ///   then holds even if the page changes afterward.
    pub fn place_image_signature(
        &self,
        page: usize,
        rect: pdf_core::document::Rect,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
        name: &str,
    ) -> Result<()> {
        // A scale of 1.0 is 72 dpi, one pixel per point — the same units
        // `rect` is already in, so placing it onto this raster is a round,
        // not a conversion. Rendering fails open to white: a signature that
        // cannot sample its destination looks exactly as it did before
        // backgrounds were sampled at all, not worse.
        let flattened = match self.render_page(page, 1.0) {
            Ok(raster) => {
                let (crop, cw, ch) = raster.crop(
                    rect.left.round() as u32,
                    rect.top.round() as u32,
                    rect.right.round() as u32,
                    rect.bottom.round() as u32,
                );
                crate::signatures::composite_onto_image(&rgba, width, height, &crop, cw, ch)
            }
            Err(_) => crate::signatures::composite_onto(&rgba, [255, 255, 255]),
        };

        registry::with_session(self.handle, |s| {
            let doc = s
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?;
            let index = doc.add_annotation(
                page,
                &pdf_core::document::Annotation::Image { rect, rgba: flattened, width, height },
            )?;
            doc.mark_as_signature(page, index, name)?;
            doc.remember_image_alpha(page, index, rgba)
        })
    }

    /// Move or resize a placed picture signature, before it is applied —
    /// see [`pdf_core::document::DocumentMut::set_image_signature_rect`].
    pub fn set_image_signature_rect(
        &self,
        page: usize,
        index: usize,
        rect: pdf_core::document::Rect,
    ) -> Result<()> {
        registry::with_session(self.handle, |s| {
            let doc = s
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?;
            doc.set_image_signature_rect(page, index, rect)
        })
    }

    /// Turn a placed picture signature, before it is applied — see
    /// [`pdf_core::document::DocumentMut::rotate_image_signature`].
    pub fn rotate_image_signature(&self, page: usize, index: usize, degrees: f32) -> Result<()> {
        registry::with_session(self.handle, |s| {
            let doc = s
                .document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?;
            doc.rotate_image_signature(page, index, degrees)
        })
    }

    /// The picture signatures placed on a page, as opposed to any other
    /// picture on it — see [`pdf_core::document::ImageSignatureMark`].
    pub fn image_signature_marks(
        &self,
        page: usize,
    ) -> Result<Vec<pdf_core::document::ImageSignatureMark>> {
        registry::with_session(self.handle, |s| s.document.image_signature_marks(page))
    }

    /// The plain pictures placed on a page — see [`pdf_core::document::
    /// PlacedImageMark`].
    pub fn placed_image_marks(
        &self,
        page: usize,
    ) -> Result<Vec<pdf_core::document::PlacedImageMark>> {
        registry::with_session(self.handle, |s| s.document.placed_image_marks(page))
    }

    /// The signatures placed on a page, as opposed to any other ink on it.
    pub fn signature_marks(
        &self,
        page: usize,
    ) -> Result<Vec<pdf_core::document::SignatureMark>> {
        registry::with_session(self.handle, |s| s.document.signature_marks(page))
    }

    /// Burn a page's placed signatures into the page. Returns how many.
    pub fn apply_signatures(&self, page: usize) -> Result<usize> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("editing this document"))?
                .apply_signatures(page)
        })
    }

    /// Check the signatures this document carries.
    pub fn validate_signatures(&self) -> Result<Vec<pdf_core::pdf::validate::Signature>> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("checking this document"))?
                .validate_signatures()
        })
    }

    /// Sign this document with a certificate.
    pub fn sign_document(
        &self,
        pkcs12: &[u8],
        password: &str,
        about: &pdf_core::pdf::sign::Reason,
    ) -> Result<String> {
        registry::with_session(self.handle, |s| {
            s.document
                .as_document_mut()
                .ok_or(pdf_core::PdfError::Unsupported("signing this document"))?
                .sign_document(pkcs12, password, about)
        })
    }
    /// How many signatures this document carries.
    pub fn signature_count(&self) -> usize {
        registry::with_session(self.handle, |s| {
            Ok(s.document.as_document_mut().map(|d| d.signature_count()).unwrap_or(0))
        })
        .unwrap_or(0)
    }
}
