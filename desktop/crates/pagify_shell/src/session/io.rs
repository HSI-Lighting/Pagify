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
        Ok(Session { handle, path, generation })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
