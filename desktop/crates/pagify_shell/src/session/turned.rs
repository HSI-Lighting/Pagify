//! Pages that carry their own `/Rotate`: the one place `Session` relates the
//! frame the engine works in to the one the app draws in.
//!
//! See `pdf_core::document::frame`. The rule here is short: **everything that
//! comes out of the engine for a page is carried into the displayed frame, and
//! everything that goes in is carried back**, so the app, the paragraph
//! detector and every hover and marquee only ever see the page as it is shown.
//! A page that is not turned has `Frame::NONE` and goes through untouched.
use super::*;
use pdf_core::document::frame::Frame;

impl Session {
    /// A page's frame, read once and kept until something that could change it
    /// (a page turned, cut, resized, moved, added or removed, or an undo of any
    /// of those) runs.
    ///
    /// **Never call this inside a `registry::with_session` closure**: it takes
    /// the same lock. A frame that cannot be read is `Frame::NONE` and is not
    /// kept, so a later call tries again.
    pub(crate) fn frame(&self, page: usize) -> Frame {
        if let Some(frame) = self.known_frame(page) {
            return frame;
        }
        match registry::with_session(self.handle, |s| read_frame(s, page)) {
            Ok(frame) => {
                self.keep_frame(page, frame);
                frame
            }
            Err(_) => Frame::NONE,
        }
    }

    /// As [`Session::frame`] for the callers that must never wait for the engine:
    /// `None` when the frame is not known and the engine is busy.
    pub(crate) fn try_frame(&self, page: usize) -> Option<Frame> {
        if let Some(frame) = self.known_frame(page) {
            return Some(frame);
        }
        match registry::try_with_session(self.handle, |s| read_frame(s, page))? {
            Ok(frame) => {
                self.keep_frame(page, frame);
                Some(frame)
            }
            Err(_) => Some(Frame::NONE),
        }
    }

    pub(crate) fn forget_frames(&self) {
        if let Ok(mut frames) = self.frames.lock() {
            frames.clear();
        }
    }

    fn known_frame(&self, page: usize) -> Option<Frame> {
        self.frames.lock().ok().and_then(|f| f.get(&page).copied())
    }

    fn keep_frame(&self, page: usize, frame: Frame) {
        if let Ok(mut frames) = self.frames.lock() {
            frames.insert(page, frame);
        }
    }
}

fn read_frame(s: &mut pdf_core::registry::DocumentSession, page: usize) -> Result<Frame> {
    // Most pages are not turned, and `page_rotation` has to load the page: ask
    // for the size (which does not) only once there is a turn to relate it to.
    let turns = match s.document.as_document_mut() {
        Some(d) => d.page_rotation(page)?,
        None => 0,
    };
    if turns % 4 == 0 {
        return Ok(Frame::NONE);
    }
    let size = s.document.page_size(page)?;
    Ok(Frame::new(turns, size.width_pt, size.height_pt))
}

impl Session {
    /// A command from the app, as the engine must run it.
    ///
    /// Frames are read first and the command converted outside the registry
    /// lock; a command on a page that is not turned comes back unchanged.
    pub(crate) fn unturned(&self, command: pdf_core::command::Command) -> pdf_core::command::Command {
        let mut pages = Vec::new();
        collect_pages(&command, &mut pages);
        let frames: std::collections::HashMap<usize, Frame> =
            pages.into_iter().map(|p| (p, self.frame(p))).collect();
        if frames.values().all(Frame::is_identity) {
            return command;
        }
        command.unturned(&|page| frames.get(&page).copied().unwrap_or(Frame::NONE))
    }
}

fn collect_pages(command: &pdf_core::command::Command, out: &mut Vec<usize>) {
    use pdf_core::command::Command as C;
    match command {
        C::SetPageCrop { index, .. } => out.push(*index),
        C::SetTextRun { page_index, .. }
        | C::SetTextRuns { page_index, .. }
        | C::ReplaceTextLines { page_index, .. }
        | C::MoveObject { page_index, .. }
        | C::MoveObjects { page_index, .. }
        | C::MoveObjectsEach { page_index, .. }
        | C::ScaleObjects { page_index, .. }
        | C::ScaleObject { page_index, .. }
        | C::RotateObject { page_index, .. }
        | C::SetPlacedImageRect { page_index, .. }
        | C::PasteLifted { page_index, .. }
        | C::AddAnnotation { page_index, .. }
        | C::AddTextLayer { page_index, .. }
        | C::Redact { page_index, .. } => out.push(*page_index),
        C::Batch { commands } => commands.iter().for_each(|c| collect_pages(c, out)),
        _ => {}
    }
    out.sort_unstable();
    out.dedup();
}

impl Session {
    /// Recognised words, carried into the displayed frame.
    pub(crate) fn words_shown(
        &self,
        page: usize,
        words: Option<Vec<pdf_core::document::RecognisedWord>>,
    ) -> Option<Vec<pdf_core::document::RecognisedWord>> {
        let f = self.frame(page);
        words.map(|mut w| {
            w.iter_mut().for_each(|x| x.rect = f.rect(x.rect));
            w
        })
    }
}
