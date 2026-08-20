//! Ownership of open documents across the JNI boundary.
//!
//! Kotlin only ever holds an opaque `jlong`. Handles are monotonically increasing
//! and never reused, so a stale handle from a double-close or a leaked reference
//! reliably reports `InvalidHandle` instead of silently addressing whatever
//! document happens to occupy that slot now — which reuse would allow.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::command::CommandHistory;
use crate::document::Document;
use crate::error::{PdfError, Result};
use crate::render::{PageCache, DEFAULT_CACHE_BUDGET_BYTES};

/// A document plus everything owned alongside it for its lifetime.
pub struct DocumentSession {
    pub document: Box<dyn Document>,
    pub cache: PageCache,
    /// Undo/redo for this document. Per-session rather than global: two open
    /// documents must not share a history, or undo in one would try to revert a
    /// command against the other.
    pub history: CommandHistory,
}

impl DocumentSession {
    pub fn new(document: Box<dyn Document>) -> Self {
        DocumentSession {
            document,
            cache: PageCache::new(DEFAULT_CACHE_BUDGET_BYTES),
            history: CommandHistory::default(),
        }
    }
}

static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

fn sessions() -> &'static Mutex<HashMap<i64, DocumentSession>> {
    static SESSIONS: std::sync::OnceLock<Mutex<HashMap<i64, DocumentSession>>> =
        std::sync::OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A panic while a document was borrowed poisons the registry. Recovering the
/// guard is the right call here: the alternative is that one malformed page
/// bricks every document in the app until it is restarted.
fn lock() -> MutexGuard<'static, HashMap<i64, DocumentSession>> {
    sessions().lock().unwrap_or_else(PoisonError::into_inner)
}

/// Register an already-constructed document.
///
/// Prefer [`insert_with`] for anything backed by PDFium: constructing the document
/// *outside* the lock is the bug that one documents. This entry point remains for
/// tests, whose fake documents have no engine state to race over.
pub fn insert(document: Box<dyn Document>) -> i64 {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    lock().insert(handle, DocumentSession::new(document));
    handle
}

/// Open a document **with the registry lock already held**, then register it.
///
/// The lock has to cover the open itself, not just the insertion. PDFium hands out
/// document and page addresses that it recycles freely, and `pdfium-render` keys a
/// process-global page-index cache on those raw addresses with no purge when a
/// document closes. While one document is being built or torn down, another
/// arriving at the same address shares its key space — and a page lookup can be
/// answered with the wrong document's page.
///
/// This is not theoretical, and it is not only a write-path concern. Two threads
/// cycling open → read → close over two different files, measured with
/// `examples/handle_race_probe.rs`:
///
/// ```text
///                    opens failed    wrong page geometry
///   unserialised     771 of 800      3 of the 29 reads that got through
///   serialised         0 of 800      0 of 800
/// ```
///
/// One of the corrupt reads reproduced `[200, 612, 612, 612]` exactly — the
/// failure that first appeared in the round-trip suite, one correct page followed
/// by three reporting US Letter, which is what PDFium falls back to when it cannot
/// find a page's geometry.
///
/// The app reaches this shape without editing anything: documents open on
/// `Dispatchers.IO` while renders run on `Dispatchers.Default`, so opening a second
/// file while the first is still drawing is exactly the two-thread case above.
///
/// Note what this costs: opening a large document now blocks renders of any other
/// open document until it completes. That is the right trade — PDFium's open is
/// lazy, and the alternative is a page cache that occasionally serves pages from
/// the wrong file.
pub fn insert_with(open: impl FnOnce() -> Result<Box<dyn Document>>) -> Result<i64> {
    let mut guard = lock();
    // Failing here drops the guard on the way out, so a failed open never leaves
    // the registry locked.
    let document = open()?;
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    guard.insert(handle, DocumentSession::new(document));
    Ok(handle)
}

/// Run `f` against the session for `handle`.
///
/// The registry lock is held for the duration. That serialises concurrent renders
/// of the *same* process, which is acceptable because PDFium is itself serialised
/// by pdfium-render's `thread_safe` feature — a finer-grained lock here would buy
/// no parallelism while adding a second way to deadlock.
pub fn with_session<T>(
    handle: i64,
    f: impl FnOnce(&mut DocumentSession) -> Result<T>,
) -> Result<T> {
    let mut guard = lock();
    let session = guard
        .get_mut(&handle)
        .ok_or(PdfError::InvalidHandle(handle))?;
    f(session)
}

/// Close a document, returning whether a live document was actually closed.
/// A `false` return means a double close, which the Kotlin side logs but tolerates.
pub fn remove(handle: i64) -> bool {
    lock().remove(&handle).is_some()
}

pub fn open_count() -> usize {
    lock().len()
}

/// Drop every open document. Wired to `onTrimMemory(TRIM_MEMORY_COMPLETE)`.
pub fn clear_all() {
    lock().clear();
}

/// Release cached rasters everywhere without closing anything. Wired to the
/// milder `onTrimMemory` levels, where the goal is to free memory but keep the
/// user's document open.
pub fn trim_caches() {
    for session in lock().values_mut() {
        session.cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::metadata::DocumentMetadata;
    use crate::document::Page;

    struct FakeDocument {
        pages: usize,
    }

    impl Document for FakeDocument {
        fn page_count(&self) -> usize {
            self.pages
        }
        fn metadata(&self) -> Result<DocumentMetadata> {
            Ok(DocumentMetadata {
                page_count: self.pages,
                ..Default::default()
            })
        }
        fn page(&self, index: usize) -> Result<Box<dyn Page + '_>> {
            self.validate_page_index(index)?;
            unreachable!("no test needs a real page object")
        }
    }

    fn insert_fake(pages: usize) -> i64 {
        insert(Box::new(FakeDocument { pages }))
    }

    #[test]
    fn a_handle_resolves_to_its_own_document() {
        let a = insert_fake(3);
        let b = insert_fake(7);

        let a_pages = with_session(a, |s| Ok(s.document.page_count())).unwrap();
        let b_pages = with_session(b, |s| Ok(s.document.page_count())).unwrap();

        assert_eq!(a_pages, 3);
        assert_eq!(b_pages, 7);

        remove(a);
        remove(b);
    }

    #[test]
    fn using_a_closed_handle_is_an_error_not_a_wrong_document() {
        let handle = insert_fake(1);
        assert!(remove(handle));

        let result = with_session(handle, |_| Ok(()));
        assert!(matches!(result, Err(PdfError::InvalidHandle(h)) if h == handle));
    }

    #[test]
    fn closing_twice_reports_that_nothing_was_closed() {
        let handle = insert_fake(1);
        assert!(remove(handle));
        assert!(!remove(handle), "the second close must be detectable");
    }

    #[test]
    fn handles_are_never_recycled() {
        // The danger this guards against: close a document, open another, and have
        // a stale Kotlin reference silently address the new one.
        let first = insert_fake(1);
        remove(first);
        let second = insert_fake(1);

        assert_ne!(first, second);
        assert!(with_session(first, |_| Ok(())).is_err());
        remove(second);
    }

    #[test]
    fn out_of_range_pages_are_rejected_with_the_real_page_count() {
        let handle = insert_fake(5);
        let result = with_session(handle, |s| s.document.page(9).map(|_| ()));
        assert!(matches!(
            result,
            Err(PdfError::PageOutOfRange { index: 9, count: 5 })
        ));
        remove(handle);
    }

    #[test]
    fn trimming_caches_frees_memory_without_closing_documents() {
        use crate::render::bitmap::{Bitmap, PixelOrder};
        use crate::render::CacheKey;

        let handle = insert_fake(2);
        with_session(handle, |s| {
            s.cache.put(
                CacheKey::new(0, 1.0, 0),
                Bitmap::new(16, 16, PixelOrder::Rgba).unwrap(),
            );
            Ok(())
        })
        .unwrap();

        trim_caches();

        let (entries, still_open) =
            with_session(handle, |s| Ok((s.cache.len(), s.document.page_count()))).unwrap();
        assert_eq!(entries, 0, "cached rasters were released");
        assert_eq!(still_open, 2, "the document itself stayed open");

        remove(handle);
    }
}
