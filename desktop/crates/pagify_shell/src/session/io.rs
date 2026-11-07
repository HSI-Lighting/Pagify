//! Opening a document and where it lives: the two constructors and `path`.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.
use super::*;

impl Session {
    /// Open a PDF from disk, binding PDFium first if nothing has yet.
    ///
    /// The open happens *inside* the registry lock. That is the whole point:
    /// the lock has to cover the open itself, not merely the bookkeeping after
    /// it, because a document being built shares PDFium's address space with
    /// one being read.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_password(path, None)
    }

    /// Open a document that is encrypted.
    ///
    /// Separate from [`Session::open`] rather than folded into it, because a
    /// password is not part of a path and must not be handled like one: it is
    /// never stored on the session, never written to the recent list, and never
    /// recorded. It is used once, here, and dropped.
    pub fn open_with_password(path: impl AsRef<Path>, password: Option<&str>) -> Result<Self> {
        pdfium::ensure_bound();

        let path = path.as_ref().to_path_buf();
        let for_open = path.clone();
        // A plain `String` copy, made because the `move` closure below needs
        // one it owns — wiped on drop rather than left for whatever reuses
        // that memory next. Found by audit.
        let password = password.map(|p| zeroize::Zeroizing::new(p.to_owned()));
        let handle = registry::insert_with(move || {
            let document = PdfiumDocument::open_path(
                &for_open.to_string_lossy(),
                password.as_deref().map(String::as_str),
            )?;
            Ok(Box::new(document) as Box<dyn Document>)
        })?;

        let generation = registry::with_session(handle, |s| Ok(s.history.generation_counter()))?;
        Ok(Session { handle, path: std::sync::Mutex::new(path), generation, frames: Default::default() })
    }

    pub fn path(&self) -> PathBuf {
        self.path.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Make `path` the file this document is saved to from now on — Save As,
    /// after the file has been written. PDFium keeps reading what it has not
    /// loaded yet from the file it was opened from; only the name changes.
    pub fn set_path(&self, path: PathBuf) {
        *self.path.lock().unwrap_or_else(|e| e.into_inner()) = path;
    }

    /// Write a new document of one A4 sheet of plain paper to `path`. Only
    /// writes: the caller opens it by the ordinary open path, as Android does.
    ///
    /// Made under the registry lock, because building a document shares
    /// PDFium with every session reading one.
    pub fn write_blank(path: &Path) -> Result<()> {
        pdfium::ensure_bound();
        let a4 = pdf_core::document::PageSize { width_pt: 595.0, height_pt: 842.0 };
        let bytes = registry::exclusive(|| {
            pdf_core::document::blank_document(1, a4, None, pdf_core::document::Ruling::None)
        })?;
        std::fs::write(path, bytes)?;
        Ok(())
    }
}
