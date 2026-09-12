//! Lock — recoverable redaction, proved by reading the file back.
//!
//! The claim is stronger than redaction's and so is what it takes to believe it:
//! the words must be **gone from the page** and **recoverable with the
//! passcode**, and either half failing makes the feature a lie in a different
//! direction. Half one way is a redaction that leaked; half the other is content
//! destroyed that somebody was told they could get back.
//!
//! So every test here saves, reopens from those bytes, and asks PDFium's own
//! extraction — the same discipline the redaction suite uses, for the same
//! reason.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test lock
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::command::history::CommandHistory;
use pdf_core::command::Command;
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Rect, Redaction};

const PASSCODE: &[u8] = b"correct horse battery staple";

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect { left, top, right, bottom }
}

/// The line "The quick brown fox" on `text-lines.pdf`.
fn the_fox() -> Rect {
    rect(35.0, 45.0, 170.0, 66.0)
}

fn open(name: &str) -> PdfiumDocument {
    let path = harness::fixture_path(name);
    PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open fixture")
}

fn text_of(doc: &dyn Document, page: usize) -> String {
    doc.page(page).expect("page").characters().expect("characters").text
}

/// Save as a full copy and reopen, which is what a locked document must do.
fn save_and_reopen(doc: &mut PdfiumDocument) -> PdfiumDocument {
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    PdfiumDocument::open_bytes(bytes, None).expect("reopen")
}

// ------------------------------------------------------------- both halves --

/// **The acceptance test.** Hidden on the page, recoverable from the file.
#[test]
fn a_locked_area_is_off_the_page_and_still_in_the_document() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("text-lines.pdf");
    assert!(text_of(&doc, 0).contains("The quick brown fox"), "control");

    doc.lock_area(&Redaction::new(0, the_fox()), PASSCODE, None).expect("lock");
    let mut reopened = save_and_reopen(&mut doc);

    // Half one: the words are not on the page any more.
    let after = text_of(&reopened, 0);
    assert!(!after.contains("The quick brown fox"), "the words are still readable: {after:?}");
    assert!(after.contains("jumps over the lazy dog"), "it took more than it was pointed at");

    // Half two: the passcode brings them back.
    assert_eq!(reopened.locked_pages().expect("locked pages"), vec![0]);
    let pages = reopened.open_lock(PASSCODE).expect("unlock");
    assert_eq!(pages.len(), 1);

    let (index, pdf) = &pages[0];
    reopened.replace_page(*index, pdf).expect("restore");
    assert!(
        text_of(&reopened, 0).contains("The quick brown fox"),
        "the passcode did not bring the words back"
    );
}

/// **The half that makes it Lock and not Redact**, asserted on its own so that a
/// change breaking recovery cannot hide behind the hiding working.
#[test]
fn the_original_survives_a_save_and_a_reopen() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("text-lines.pdf");
    doc.lock_area(&Redaction::new(0, the_fox()), PASSCODE, None).expect("lock");

    // Twice, because an attachment that survives one rewrite and not two is a
    // document that loses its original the second time somebody saves it.
    let mut once = save_and_reopen(&mut doc);
    let mut twice = save_and_reopen(&mut once);

    assert_eq!(twice.locked_pages().expect("locked"), vec![0]);
    let pages = twice.open_lock(PASSCODE).expect("unlock");
    let (index, pdf) = &pages[0];
    twice.replace_page(*index, pdf).expect("restore");
    assert!(text_of(&twice, 0).contains("The quick brown fox"));
}

// -------------------------------------------------------------- the passcode --

#[test]
fn the_wrong_passcode_recovers_nothing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("text-lines.pdf");
    doc.lock_area(&Redaction::new(0, the_fox()), PASSCODE, None).expect("lock");
    let mut reopened = save_and_reopen(&mut doc);

    assert!(reopened.open_lock(b"correct horse battery stapl").is_err(), "a near miss opened it");
    assert!(reopened.open_lock(b"").is_err());
    assert!(reopened.open_lock(PASSCODE).is_ok());
}

/// **The wrong passcode must fail before anything comes off the page.** A
/// mistyped passcode that redacted anyway would destroy content and leave no way
/// back — the exact opposite of what Lock promises.
#[test]
fn a_wrong_passcode_on_a_second_lock_changes_nothing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.lock_area(&Redaction::new(0, rect(45.0, 130.0, 220.0, 146.0)), PASSCODE, None).expect("lock");
    let before = text_of(&doc, 0);

    let outcome = doc.lock_area(&Redaction::new(0, rect(45.0, 160.0, 220.0, 180.0)), b"wrong", None);
    assert!(outcome.is_err(), "a wrong passcode locked a second area anyway");
    assert_eq!(text_of(&doc, 0), before, "content was removed under a passcode that failed");
}

// ------------------------------------------------------- what it will not do --

/// Everything that refuses a redaction refuses this. Keeping a copy does not
/// make curves removable.
#[test]
fn an_outlined_page_refuses_to_lock() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("outlined.pdf");
    assert!(doc.lock_area(&Redaction::new(0, rect(0.0, 0.0, 500.0, 500.0)), PASSCODE, None).is_err());
    assert!(
        doc.locked_pages().expect("locked").is_empty(),
        "it wrote a vault for a page it could not lock"
    );
}

#[test]
fn a_document_with_no_lock_says_so() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("text-lines.pdf");
    assert!(doc.locked_pages().expect("locked").is_empty());
    assert!(doc.open_lock(PASSCODE).is_err(), "it claimed to unlock an unlocked document");
}

/// A locked document carries its original inside itself, so appending a delta
/// would leave the unsealed page in the file's earlier revision.
#[test]
fn a_locked_document_must_be_saved_as_a_full_copy() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("text-lines.pdf");
    assert!(!doc.must_save_full_copy());
    doc.lock_area(&Redaction::new(0, the_fox()), PASSCODE, None).expect("lock");
    assert!(doc.must_save_full_copy());

    let mut bytes = Vec::new();
    assert!(doc.save_incremental(&mut bytes).is_err());
}

// ------------------------------------------------------------ two areas, two pages --

/// **Every locked area comes back, not just the last one.**
///
/// Reported from use: "if I lock multiple areas then I am able to only unlock
/// the last one I locked." Each lock seals the page *as it stands at that
/// moment*, so the second one is handed a page already missing the first area;
/// the vault used to replace the seal with it, discarding the only copy that
/// still had everything. This test asserted that as intended behaviour, which
/// is how it survived — see `Vault::seal_page`.
#[test]
fn a_second_area_on_the_same_page_keeps_one_way_back() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let original = text_of(&doc, 0);

    // Each lock is checked to have actually taken something off the page.
    // Without that the restore below could pass while proving nothing — which
    // is how the old assertion here stayed green: it named words that neither
    // rectangle covered.
    doc.lock_area(&Redaction::new(0, rect(45.0, 130.0, 220.0, 146.0)), PASSCODE, None).expect("first");
    let after_first = text_of(&doc, 0);
    assert_ne!(after_first, original, "the first lock hid nothing");

    doc.lock_area(&Redaction::new(0, rect(45.0, 160.0, 220.0, 180.0)), PASSCODE, None).expect("second");
    let after_second = text_of(&doc, 0);
    assert_ne!(after_second, after_first, "the second lock hid nothing");

    let mut reopened = save_and_reopen(&mut doc);
    assert_eq!(reopened.locked_pages().expect("locked"), vec![0], "two ways back is one too many");

    let pages = reopened.open_lock(PASSCODE).expect("unlock");
    let (index, pdf) = &pages[0];
    reopened.replace_page(*index, pdf).expect("restore");

    assert_eq!(
        text_of(&reopened, 0),
        original,
        "unlocking did not give back every area that was locked"
    );
}

#[test]
fn two_pages_lock_and_unlock_independently() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("mixed-sizes.pdf");
    let pages = doc.page_count();
    assert!(pages >= 2);

    // Nothing to remove on these, which is fine: what is under test is that two
    // seals coexist and come back keyed to their own pages.
    doc.lock_area(
        &Redaction { fill: None, ..Redaction::new(0, rect(10.0, 10.0, 50.0, 30.0)) },
        PASSCODE, None)
    .expect("page 1");
    doc.lock_area(
        &Redaction { fill: None, ..Redaction::new(1, rect(10.0, 10.0, 50.0, 30.0)) },
        PASSCODE, None)
    .expect("page 2");

    let mut reopened = save_and_reopen(&mut doc);
    assert_eq!(reopened.locked_pages().expect("locked"), vec![0, 1]);

    let opened = reopened.open_lock(PASSCODE).expect("unlock");
    assert_eq!(opened.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(reopened.page_count(), pages, "unlocking changed the page count");
}

// ------------------------------------------------------- through the commands --

/// **Re-insertion is an ordinary command, so undo works** — and no passcode goes
/// anywhere near the undo stack.
#[test]
fn unlocking_goes_through_the_command_stack_and_undoes() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("text-lines.pdf");
    doc.lock_area(&Redaction::new(0, the_fox()), PASSCODE, None).expect("lock");
    let mut doc: Box<dyn Document> = Box::new(save_and_reopen(&mut doc));

    let hidden = text_of(doc.as_ref(), 0);
    assert!(!hidden.contains("The quick brown fox"));

    // Decrypt outside the stack; put the page back through it.
    let pages = doc.as_document_mut().unwrap().open_lock(PASSCODE).expect("unlock");
    let mut history = CommandHistory::default();
    for (index, pdf) in pages {
        history
            .execute(Command::ReplacePage { index, pdf }, doc.as_document_mut().unwrap())
            .expect("restore");
    }
    assert!(text_of(doc.as_ref(), 0).contains("The quick brown fox"), "the page did not come back");
    assert_eq!(history.undo_description().as_deref(), Some("Restore page 1"));

    history.undo(doc.as_document_mut().unwrap()).expect("undo");
    assert_eq!(text_of(doc.as_ref(), 0), hidden, "undo did not re-hide the page");

    history.redo(doc.as_document_mut().unwrap()).expect("redo");
    assert!(text_of(doc.as_ref(), 0).contains("The quick brown fox"), "redo did not restore");
}

// ------------------------------------------------------- whole pages --

/// **Locking the whole document.** Reported from use as not being available at
/// all: `lock` only ever armed a rectangle drag on one page.
///
/// Every page goes blank and every page comes back.
#[test]
fn every_page_can_be_locked_at_once_and_all_of_them_come_back() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let pages: Vec<usize> = (0..doc.page_count()).collect();
    let before: Vec<String> = pages.iter().map(|p| text_of(&doc, *p)).collect();
    assert!(before.iter().any(|t| !t.trim().is_empty()), "nothing to lock");

    assert_eq!(doc.lock_pages(&pages, PASSCODE).expect("lock"), pages.len());

    let mut reopened = save_and_reopen(&mut doc);
    for page in &pages {
        assert!(
            text_of(&reopened, *page).trim().is_empty(),
            "page {} still has its words on it after being locked",
            page + 1
        );
    }
    assert_eq!(reopened.locked_pages().expect("locked"), pages);

    for (index, pdf) in reopened.open_lock(PASSCODE).expect("unlock") {
        reopened.replace_page(index, &pdf).expect("restore");
    }
    let after: Vec<String> = pages.iter().map(|p| text_of(&reopened, *p)).collect();
    assert_eq!(after, before, "unlocking did not give the document back");
}

/// **The reason this is not `lock_area` over the whole page.** Area locking
/// routes through redaction, which refuses a page whose words are curves — see
/// `an_outlined_page_refuses_to_lock`. Hiding the entire page needs no such
/// precision, so it must work here where the other cannot.
#[test]
fn a_page_that_refuses_area_locking_can_still_be_locked_whole() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("outlined.pdf");
    // The refusal that makes this test worth having, asserted rather than
    // assumed — if area locking ever starts accepting outlined pages, the
    // premise here is gone and someone should notice.
    assert!(
        doc.lock_area(&Redaction::new(0, rect(0.0, 0.0, 500.0, 500.0)), PASSCODE, None).is_err(),
        "area locking now accepts an outlined page, so this test proves nothing"
    );

    assert_eq!(doc.lock_pages(&[0], PASSCODE).expect("lock the whole page"), 1);

    let mut reopened = save_and_reopen(&mut doc);
    assert_eq!(reopened.locked_pages().expect("locked"), vec![0]);
    for (index, pdf) in reopened.open_lock(PASSCODE).expect("unlock") {
        reopened.replace_page(index, &pdf).expect("restore");
    }
    // An outlined page has no text to compare, so the proof is the glyph
    // contours: gone while locked, back afterwards.
    let paths = reopened.page(0).expect("page").classify().expect("classify").glyph_paths;
    assert!(paths > 0, "the outlined page's own drawing did not come back");
}

/// A page already locked keeps the way back it has, and a second call does not
/// claim to have locked it again.
#[test]
fn locking_a_page_that_is_already_locked_keeps_the_first_original() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let original = text_of(&doc, 0);

    assert_eq!(doc.lock_pages(&[0], PASSCODE).expect("first"), 1);
    assert_eq!(doc.lock_pages(&[0], PASSCODE).expect("second"), 0, "it locked a blank page over the original");

    let mut reopened = save_and_reopen(&mut doc);
    for (index, pdf) in reopened.open_lock(PASSCODE).expect("unlock") {
        reopened.replace_page(index, &pdf).expect("restore");
    }
    assert_eq!(text_of(&reopened, 0), original, "the original was replaced by the blank page");
}

/// A range with one bad page number refuses outright rather than blanking the
/// good pages before reaching it.
#[test]
fn a_bad_page_number_locks_nothing_at_all() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);
    let past_the_end = doc.page_count() + 5;

    assert!(doc.lock_pages(&[0, past_the_end], PASSCODE).is_err());
    assert_eq!(text_of(&doc, 0), before, "page 1 was blanked before the bad index was reached");
    assert!(doc.locked_pages().expect("locked").is_empty(), "it wrote a vault anyway");
}

/// The wrong passcode on an already-locked document changes nothing — the same
/// guarantee `a_wrong_passcode_on_a_second_lock_changes_nothing` makes for
/// areas.
#[test]
fn a_wrong_passcode_locks_no_further_pages() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.lock_pages(&[0], PASSCODE).expect("lock");
    let locked_text = text_of(&doc, 0);

    assert!(doc.lock_pages(&[0], b"not the passcode").is_err());
    assert_eq!(text_of(&doc, 0), locked_text, "a wrong passcode still changed the page");
}

// ------------------------------------------------------------ images --

/// Images have to be findable before they can be offered as something to
/// lock — what a right-click on the page hit-tests against.
#[test]
fn the_images_on_a_page_can_be_found_and_placed() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("scan-300dpi.pdf");
    let images = doc.images_on(0).expect("images");
    assert!(!images.is_empty(), "a scanned page is an image and none was found");

    let image = &images[0];
    assert!(image.pixel_width > 0 && image.pixel_height > 0, "no pixel size");
    assert!(image.raw_bytes > 0, "no stored bytes, so nothing could be sealed");
    assert!(
        image.rect.right > image.rect.left && image.rect.bottom > image.rect.top,
        "the rect is inside out, so a badge would draw nowhere: {:?}",
        image.rect
    );
    // A scan covers its page, so the placement should be page-sized rather
    // than a default unit square — which is what an unread matrix looks like.
    let size = doc.page_size(0).expect("size");
    assert!(
        image.matrix[0] > size.width_pt * 0.5,
        "the matrix was not read: {:?}",
        image.matrix
    );
}

/// A page with no images says so, rather than reporting the text objects on it.
#[test]
fn a_page_of_text_reports_no_images() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("two-column.pdf");
    assert!(doc.images_on(0).expect("images").is_empty());
}

/// **The acceptance test for locking an image.** Off the page, and back with
/// the passcode — the same two halves this whole suite exists to prove, for the
/// thing that could not be locked at all before.
#[test]
fn a_locked_image_comes_off_the_page_and_the_passcode_puts_it_back() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("scan-300dpi.pdf");
    let before = doc.images_on(0).expect("images");
    assert_eq!(before.len(), 1, "the fixture should have exactly one image");
    let (was_at, was_sized) = (before[0].rect, before[0].matrix);

    let id = doc.lock_image(0, before[0].object, PASSCODE).expect("lock the image");

    // Half one: its pixels are gone. The object stays — it is blanked where it
    // stands rather than removed, because removing needs a content-stream
    // rewrite that corrupts the rest of the page (see `blank_image`) — so what
    // is asserted is that nothing of the picture survives in the file.
    let mut reopened = save_and_reopen(&mut doc);
    let blanked = reopened.images_on(0).expect("images");
    assert_eq!(blanked.len(), 1, "the object should still be there, emptied");
    assert!(
        blanked[0].pixel_width <= 1 && blanked[0].pixel_height <= 1,
        "the image still has its pixels: {}x{}",
        blanked[0].pixel_width,
        blanked[0].pixel_height
    );
    // And the page knows where it was, so a badge can be drawn over the gap.
    let badges = reopened.locked_items_on(0).expect("locked items");
    assert_eq!(badges.len(), 1);
    assert_eq!(badges[0].id, id);
    assert!(
        (badges[0].rect.left - was_at.left).abs() < 0.5,
        "the badge would be drawn in the wrong place: {:?} vs {:?}",
        badges[0].rect,
        was_at
    );

    // Half two: the passcode puts it back, where and as it was.
    reopened.unlock_item(&id, PASSCODE).expect("unlock the image");
    let after = reopened.images_on(0).expect("images");
    assert_eq!(after.len(), 1, "the image did not come back");
    assert!(
        after[0].pixel_width > 1 && after[0].pixel_height > 1,
        "it came back still empty: {}x{}",
        after[0].pixel_width,
        after[0].pixel_height
    );
    assert!(
        (after[0].matrix[0] - was_sized[0]).abs() < 0.5
            && (after[0].matrix[3] - was_sized[3]).abs() < 0.5,
        "it came back the wrong size: {:?} vs {:?}",
        after[0].matrix,
        was_sized
    );
    assert!(
        reopened.locked_items_on(0).expect("locked").is_empty(),
        "the badge outlived the lock it stood for"
    );
}

/// The wrong passcode leaves the image sealed, and says so.
#[test]
fn a_wrong_passcode_does_not_bring_an_image_back() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("scan-300dpi.pdf");
    let object = doc.images_on(0).expect("images")[0].object;
    let id = doc.lock_image(0, object, PASSCODE).expect("lock");

    assert!(doc.unlock_item(&id, b"not the passcode").is_err());
    assert!(
        doc.images_on(0).expect("images")[0].pixel_width <= 1,
        "a wrong passcode filled the image back in"
    );
    assert_eq!(doc.locked_items_on(0).expect("locked").len(), 1, "the seal was dropped");

    doc.unlock_item(&id, PASSCODE).expect("the right one still works");
    assert!(doc.images_on(0).expect("images")[0].pixel_width > 1, "it did not come back");
}

/// Locking an image must not disturb the text around it — the point of sealing
/// the object rather than the page.
#[test]
fn locking_an_image_leaves_the_rest_of_the_page_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("scan-300dpi.pdf");
    let text_before = text_of(&doc, 0);
    let object = doc.images_on(0).expect("images")[0].object;

    let id = doc.lock_image(0, object, PASSCODE).expect("lock");
    assert_eq!(text_of(&doc, 0), text_before, "locking the image changed the text");

    doc.unlock_item(&id, PASSCODE).expect("unlock");
    assert_eq!(text_of(&doc, 0), text_before);
}

/// **Reported from use:** locking an image and then unlocking it left the text
/// above it hidden, recoverable only by unlocking the whole document.
///
/// Against a real catalogue page rather than the scan fixture, whose text is a
/// single caption: the pages this happens on carry a paragraph *and* a photo,
/// and `scan-300dpi.pdf` passes the same assertions while exercising far less.
#[test]
fn locking_and_unlocking_an_image_leaves_a_real_pages_text_untouched() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = format!("{}/Downloads/HSI CATALOG 2026.pdf", std::env::var("HOME").unwrap_or_default());
    if !std::path::Path::new(&path).is_file() {
        eprintln!("skipping: no catalogue in ~/Downloads");
        return;
    }
    // Skipped rather than failed when it will not open. This reads a document
    // from the person's own Downloads folder, which they are entitled to
    // change — and one of them acquired a password mid-development, at which
    // point five tests about locking failed for a reason that had nothing to
    // do with locking.
    let Ok(mut doc) = PdfiumDocument::open_path(&path, None) else {
        eprintln!("skipping: the catalogue is present but would not open");
        return;
    };

    // A page with both a photo and a paragraph — the combination that broke.
    let Some(page) = (0..60).find(|p| {
        !doc.images_on(*p).unwrap_or_default().is_empty() && text_of(&doc, *p).trim().len() > 80
    }) else {
        eprintln!("skipping: no page with text beside an image");
        return;
    };

    let before = text_of(&doc, page);
    let object = doc.images_on(page).expect("images")[0].object;

    let id = doc.lock_image(page, object, PASSCODE).expect("lock");

    // **Exactly unchanged, while locked.** This is what the whole `crate::pdf`
    // writer buys: the page's content stream is copied through byte for byte
    // rather than re-emitted, so the text cannot come back different. Before
    // it, this assertion failed on this very page — `sky-light` turned into
    // `sky -\r\nlight` merely because an image beside it had been blanked.
    assert_eq!(
        text_of(&doc, page),
        before,
        "locking the image rewrote the text around it"
    );

    // And unlocking gives the page back whole.
    doc.unlock_item(&id, PASSCODE).expect("unlock");
    assert_eq!(text_of(&doc, page), before, "unlocking did not restore the page");
}

/// **Reported from use, twice:** locking a passage changed how the text around
/// it drew — heavier, and with the line breaks moved.
///
/// The image path stopped doing this once `crate::pdf` took over the rewrite.
/// A locked *area* still goes through redaction, which removes text objects
/// through PDFium and therefore through `FPDFPage_GenerateContent`, re-emitting
/// every content stream on the page.
///
/// Fixed by `cut_text_in_area`, which cuts the operators that drew the locked
/// words out of the content stream and copies every other byte through — see
/// `crate::pdf::content`. This is the test that proves it.
#[test]
fn locking_a_passage_leaves_the_text_around_it_exactly_as_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = format!("{}/Downloads/HSI CATALOG 2026.pdf", std::env::var("HOME").unwrap_or_default());
    if !std::path::Path::new(&path).is_file() {
        eprintln!("skipping: no catalogue in ~/Downloads");
        return;
    }
    // Skipped rather than failed when it will not open. This reads a document
    // from the person's own Downloads folder, which they are entitled to
    // change — and one of them acquired a password mid-development, at which
    // point five tests about locking failed for a reason that had nothing to
    // do with locking.
    let Ok(mut doc) = PdfiumDocument::open_path(&path, None) else {
        eprintln!("skipping: the catalogue is present but would not open");
        return;
    };

    let Some(page) = (0..60).find(|p| text_of(&doc, *p).contains("Borujerdi")) else {
        eprintln!("skipping: that page is not in this copy");
        return;
    };
    let before = text_of(&doc, page);

    // A few words in the middle of the first line, leaving the rest of the
    // paragraph — which is what must come through untouched — alone.
    let chars = doc.page(page).expect("page").characters().expect("characters");
    let at = before.find("historic Persian").expect("the phrase");
    // Four numbers per code unit — left, top, right, bottom.
    let span = at..at + "historic Persian".len();
    let mut area = Rect { left: f32::MAX, top: f32::MAX, right: f32::MIN, bottom: f32::MIN };
    for index in span {
        let Some(box_) = chars.boxes.get(index * 4..index * 4 + 4) else { continue };
        area.left = area.left.min(box_[0]);
        area.top = area.top.min(box_[1]);
        area.right = area.right.max(box_[2]);
        area.bottom = area.bottom.max(box_[3]);
    }
    assert!(area.right > area.left, "the phrase has no box on the page");

    doc.lock_area(
        &Redaction { require_complete: false, ..Redaction::new(page, area) },
        PASSCODE,
        None,
    )
    .expect("lock the passage");

    let after = text_of(&doc, page);
    assert!(!after.contains("historic Persian"), "the passage was not hidden");

    // **Only the selected words go.** The rest of their own line stays, which
    // is what cutting individual character codes buys over cutting the whole
    // show-text operator.
    assert!(
        after.contains("The Borujerdi House"),
        "it took the words before the selection too:\n{after:?}"
    );
    assert!(
        after.contains("features an ingenious"),
        "it took the words after the selection too:\n{after:?}"
    );

    // And every other line is byte-identical.
    let rest = |text: &str| {
        text.split("design that optimizes")
            .nth(1)
            .map(str::to_owned)
            .unwrap_or_default()
    };
    let (before_rest, after_rest) = (rest(&before), rest(&after));
    assert!(!before_rest.is_empty(), "the fixture page is not what this expects");
    assert_eq!(
        after_rest, before_rest,
        "locking the passage rewrote the text around it"
    );
}

/// **Reported from use:** locking every page left the app lagging badly.
///
/// `locked_items_on` is called while drawing — once per visible page, per frame
/// — and went through the whole attachment each time: copied out of the file,
/// parsed as JSON, every sealed page base64-decoded. Measured at 57 ms a call
/// with eight pages locked, against a sixteen-millisecond frame.
///
/// This pins the cache that fixed it, at a threshold far looser than the 44 µs
/// it actually takes, so it fails on a regression rather than on a slow machine.
#[test]
fn asking_what_is_locked_stays_cheap_once_a_document_is_locked() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let pages: Vec<usize> = (0..doc.page_count()).collect();
    doc.lock_pages(&pages, PASSCODE).expect("lock");

    // The first call may read the file; the ones after it must not.
    let _ = doc.locked_items_on(0).expect("locked items");
    let started = std::time::Instant::now();
    for _ in 0..50 {
        doc.locked_items_on(0).expect("locked items");
    }
    let each = started.elapsed() / 50;
    assert!(
        each < std::time::Duration::from_millis(5),
        "reading the lock costs {each:?} a call, which a frame cannot afford"
    );
}

/// And a locked page carries a badge, so it can be unlocked by clicking it
/// rather than only through the `unlock` verb.
#[test]
fn a_locked_page_has_something_to_click() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.lock_pages(&[0], PASSCODE).expect("lock");

    let badges = doc.locked_items_on(0).expect("locked items");
    assert_eq!(badges.len(), 1, "a locked page offers no way back");
    let size = doc.page_size(0).expect("size");
    assert!(
        badges[0].rect.right >= size.width_pt - 0.5,
        "the badge does not cover the page it stands for: {:?}",
        badges[0].rect
    );

    doc.unlock_item(&badges[0].id, PASSCODE).expect("unlock by badge");
    assert!(text_of(&doc, 0).contains("luminaire"), "the page did not come back");
}

/// **Precision is a property of the mechanism, not of one page.**
///
/// The first version of this was demonstrated on a single catalogue page, which
/// proves nothing about the next one. `examples/lock_precision_probe.rs`
/// answers it across a real document — 32 of 37 pages precise, none wrong, at
/// the time of writing — and this pins the same property on a committed fixture
/// so it runs everywhere.
#[test]
fn locking_a_phrase_takes_the_phrase_and_leaves_its_neighbours() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);

    // A phrase in the middle of a line, with words either side of it that must
    // survive — the case that used to take the whole line.
    let phrase = "opal";
    let (before_it, after_it) = ("the diffuser is", "polycarbonate");
    assert!(
        before.contains(phrase) && before.contains(before_it) && before.contains(after_it),
        "the fixture is not what this test expects"
    );

    let chars = doc.page(0).expect("page").characters().expect("characters");
    let at = before.find(phrase).expect("the phrase");
    let at = before[..at].chars().count();

    let mut area = Rect { left: f32::MAX, top: f32::MAX, right: f32::MIN, bottom: f32::MIN };
    for index in at..at + phrase.chars().count() {
        let b = &chars.boxes[index * 4..index * 4 + 4];
        area.left = area.left.min(b[0]);
        area.top = area.top.min(b[1]);
        area.right = area.right.max(b[2]);
        area.bottom = area.bottom.max(b[3]);
    }

    doc.lock_area(
        &Redaction { require_complete: false, ..Redaction::new(0, area) },
        PASSCODE,
        None,
    )
    .expect("lock the phrase");

    let after = text_of(&doc, 0);
    assert!(!after.contains(phrase), "the phrase was not hidden:\n{after:?}");
    assert!(
        after.contains(before_it),
        "it took the words before it too:\n{after:?}"
    );
    assert!(
        after.contains(after_it),
        "it took the words after it too:\n{after:?}"
    );

    // And the rest of the page is untouched.
    let tail = |t: &str| t.split("Control gear").nth(1).map(str::to_owned).unwrap_or_default();
    assert_eq!(tail(&after), tail(&before), "the rest of the page was rewritten");
}

/// **A ligature must not put the cut on the wrong glyphs.**
///
/// One code can spell two characters — `02DB` spells `"fl"` on a real
/// catalogue page — and a code can spell something PDFium leaves out of its
/// extracted text, which is how an operator comes to draw 44 codes for 42
/// characters. Past either, a character index and a code index are different
/// numbers. `pdf::cmap` reads the font's own table so they can be reconciled;
/// `examples/lock_precision_probe.rs` measures the result at 69 of 79 pages
/// precise, none wrong, none shifted.
///
/// This checks the property on a page that has such a font, and skips where the
/// catalogue is not present.
#[test]
fn locking_a_phrase_is_precise_on_a_page_whose_font_uses_ligatures() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = format!("{}/Downloads/HSI CATALOG 2026.pdf", std::env::var("HOME").unwrap_or_default());
    if !std::path::Path::new(&path).is_file() {
        eprintln!("skipping: no catalogue in ~/Downloads");
        return;
    }
    // Skipped rather than failed when it will not open. This reads a document
    // from the person's own Downloads folder, which they are entitled to
    // change — and one of them acquired a password mid-development, at which
    // point five tests about locking failed for a reason that had nothing to
    // do with locking.
    let Ok(mut doc) = PdfiumDocument::open_path(&path, None) else {
        eprintln!("skipping: the catalogue is present but would not open");
        return;
    };

    // Page 29 is the one that kept shifting before the font's table was read.
    let page = 28;
    let before = text_of(&doc, page);
    let Some(phrase) = ["spot lights", "track and", "design track"]
        .into_iter()
        .find(|p| before.matches(p).count() == 1)
    else {
        eprintln!("skipping: that page is not in this copy");
        return;
    };

    let chars = doc.page(page).expect("page").characters().expect("characters");
    let at = before.find(phrase).expect("the phrase");
    let at = before[..at].chars().count();
    let mut area = Rect { left: f32::MAX, top: f32::MAX, right: f32::MIN, bottom: f32::MIN };
    for index in at..at + phrase.chars().count() {
        let Some(b) = chars.boxes.get(index * 4..index * 4 + 4) else { continue };
        area.left = area.left.min(b[0]);
        area.top = area.top.min(b[1]);
        area.right = area.right.max(b[2]);
        area.bottom = area.bottom.max(b[3]);
    }

    // Where a witness word sits before the lock, so the shift can be measured —
    // extraction reports the same words wherever they are, and cannot see this.
    let witness = "Modern";
    let witness_at = before.find(witness).map(|b| before[..b].chars().count());
    let was = witness_at.and_then(|i| chars.boxes.get(i * 4).copied());

    doc.lock_area(
        &Redaction { require_complete: false, ..Redaction::new(page, area) },
        PASSCODE,
        None,
    )
    .expect("lock the phrase");

    let after = text_of(&doc, page);
    assert!(!after.contains(phrase), "the phrase was not hidden");
    assert!(after.contains(witness), "it took the words before it too:\n{after:?}");

    if let (Some(was), Some(now)) = (
        was,
        doc.page(page)
            .ok()
            .and_then(|p| p.characters().ok())
            .and_then(|c| {
                let i = after.find(witness).map(|b| after[..b].chars().count())?;
                c.boxes.get(i * 4).copied()
            }),
    ) {
        assert!(
            (now - was).abs() < 1.0,
            "the surviving text moved {:.1}pt",
            (now - was).abs()
        );
    }
}

/// **Editing a word changes only that word.**
///
/// Reported from use: the same complaint as locking — changing text rewrote the
/// paragraph around it. `FPDFText_SetText` needs `FPDFPage_GenerateContent`,
/// which re-emits the whole page; `set_run_in_stream` swaps the codes in place
/// instead and touches nothing else.
#[test]
fn editing_a_run_leaves_the_rest_of_the_page_exactly_as_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = format!("{}/Downloads/HSI CATALOG 2026.pdf", std::env::var("HOME").unwrap_or_default());
    if !std::path::Path::new(&path).is_file() {
        eprintln!("skipping: no catalogue in ~/Downloads");
        return;
    }
    // Skipped rather than failed when it will not open. This reads a document
    // from the person's own Downloads folder, which they are entitled to
    // change — and one of them acquired a password mid-development, at which
    // point five tests about locking failed for a reason that had nothing to
    // do with locking.
    let Ok(mut doc) = PdfiumDocument::open_path(&path, None) else {
        eprintln!("skipping: the catalogue is present but would not open");
        return;
    };

    let Some(page) = (0..60).find(|p| text_of(&doc, *p).contains("Borujerdi")) else {
        eprintln!("skipping: that page is not in this copy");
        return;
    };
    let before = text_of(&doc, page);

    // The run holding the first line, and a witness further down the page that
    // must not move or change.
    let runs = doc.text_runs(page).expect("runs");
    let Some(run) = runs.iter().find(|r| r.text.contains("Borujerdi")) else {
        eprintln!("skipping: no run to edit");
        return;
    };
    let object = run.object;

    let tail = |text: &str| {
        text.split("design that optimizes").nth(1).map(str::to_owned).unwrap_or_default()
    };
    let before_tail = tail(&before);
    assert!(!before_tail.is_empty(), "the page is not what this expects");

    doc.set_text_run(page, object, "The Borujerdi House is closed")
        .expect("edit the run");

    let after = text_of(&doc, page);
    assert!(after.contains("is closed"), "the edit did not take:\n{after:?}");
    assert_eq!(
        tail(&after),
        before_tail,
        "editing the run rewrote the text around it"
    );
}

/// **PDFium's own trailing space belongs to no character code.**
///
/// Measured on a real catalogue: a font's table spells
/// `"Color Temperature        6500 k"` where PDFium reports
/// `"Color Temperature 6500 k "` — it collapses the run of spaces and appends
/// one of its own. Refusing to align on account of that last space was the
/// whole of the remaining fallback: with it allowed, every page of the
/// catalogue locks precisely.
///
/// Skips where the catalogue is not present; the property is measured in full
/// by `examples/lock_precision_probe.rs`.
#[test]
fn a_run_padded_with_spaces_still_locks_precisely() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = format!("{}/Downloads/HSI CATALOG 2026.pdf", std::env::var("HOME").unwrap_or_default());
    if !std::path::Path::new(&path).is_file() {
        eprintln!("skipping: no catalogue in ~/Downloads");
        return;
    }
    // Skipped rather than failed when it will not open. This reads a document
    // from the person's own Downloads folder, which they are entitled to
    // change — and one of them acquired a password mid-development, at which
    // point five tests about locking failed for a reason that had nothing to
    // do with locking.
    let Ok(mut doc) = PdfiumDocument::open_path(&path, None) else {
        eprintln!("skipping: the catalogue is present but would not open");
        return;
    };

    // A page whose runs are padded that way.
    let Some((page, phrase)) = (0..149).find_map(|p| {
        let text = text_of(&doc, p);
        ["Color Temperature", "Luminaire Efficacy"]
            .into_iter()
            .find(|needle| text.matches(needle).count() == 1)
            .map(|needle| (p, needle))
    }) else {
        eprintln!("skipping: that page is not in this copy");
        return;
    };

    let before = text_of(&doc, page);
    let chars = doc.page(page).expect("page").characters().expect("characters");
    // The first word **of that phrase**, not the first occurrence of that word
    // anywhere — "Color" appears all over a lighting catalogue, and locking a
    // different one proves nothing.
    let word = phrase.split_whitespace().next().expect("a word");
    let rest = phrase.split_whitespace().nth(1).expect("a second word");
    let at = before.find(phrase).expect("the phrase");
    let at = before[..at].chars().count();

    let mut area = Rect { left: f32::MAX, top: f32::MAX, right: f32::MIN, bottom: f32::MIN };
    for index in at..at + word.chars().count() {
        let Some(b) = chars.boxes.get(index * 4..index * 4 + 4) else { continue };
        area.left = area.left.min(b[0]);
        area.top = area.top.min(b[1]);
        area.right = area.right.max(b[2]);
        area.bottom = area.bottom.max(b[3]);
    }

    let report = doc
        .lock_area(
            &Redaction { require_complete: false, ..Redaction::new(page, area) },
            PASSCODE,
            None,
        )
        .expect("lock the word");

    let after = text_of(&doc, page);
    assert!(!after.contains(phrase), "the phrase is still intact:\n{after:?}");
    // And the word after it, on the same line, survived — which is the whole
    // point of cutting codes rather than the operator.
    assert!(after.contains(rest), "it took the rest of the line too:\n{after:?}");
    assert!(
        report.spilled.is_empty(),
        "it reported taking more than it was asked for: {:?}",
        report.spilled
    );
}

/// **A locked page draws nothing** — in memory, and in the file anyone else
/// opens.
///
/// The companion to the app's `locking_a_page_leaves_no_stale_thumbnail_of_it`,
/// which proves the caches are dropped. This proves there is nothing left for
/// them to hold: a cache fix would otherwise hide a lock that never really
/// removed anything.
#[test]
fn a_locked_page_renders_blank() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    /// How much of a thumbnail-sized render is not paper.
    fn ink(doc: &dyn Document, page: usize) -> f32 {
        let size = doc.page_size(page).expect("size");
        let width = 140u32;
        let scale = width as f32 / size.width_pt;
        let height = (size.height_pt * scale).max(1.0) as u32;

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut target = pdf_core::render::RenderTarget {
            width,
            height,
            stride: (width * 4) as usize,
            order: pdf_core::render::PixelOrder::Rgba,
            pixels: &mut pixels,
        };
        doc.page(page)
            .expect("page")
            .render_into(
                &pdf_core::document::RenderRequest { scale, ..Default::default() },
                &mut target,
            )
            .expect("render");

        let inked = pixels
            .chunks_exact(4)
            .filter(|p| p[0] < 240 || p[1] < 240 || p[2] < 240)
            .count();
        inked as f32 / (width * height) as f32
    }

    let mut doc = open("two-column.pdf");
    assert!(ink(&doc, 0) > 0.01, "the fixture draws nothing, so this proves nothing");

    doc.lock_pages(&[0], PASSCODE).expect("lock");
    assert!(ink(&doc, 0) < 0.0005, "the locked page still draws something");

    // And in the file as saved, which is what anyone else would open.
    let reopened = save_and_reopen(&mut doc);
    assert!(
        ink(&reopened, 0) < 0.0005,
        "the saved file still draws the locked page"
    );
}

/// **Marks over a locked area go with it, and come back.**
///
/// Reported from use: "annotations don't get locked" — ink drawn across a
/// passage stayed exactly where it was, drawn on *top* of the black mark.
/// Annotations are not page content; they live in the page's `/Annots` array,
/// so cutting the content stream never came near them.
///
/// Asserted on the ink's own colour rather than on how much of the page is
/// dark, because the redaction mark is itself a black box — "less ink" would
/// pass whether or not the stroke went.
#[test]
fn locking_an_area_takes_the_marks_over_it_and_gives_them_back() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::{Annotation, Color, Point};

    /// Pixels of the ink's own colour, which no redaction mark can draw.
    fn pink(doc: &dyn Document, page: usize) -> usize {
        let size = doc.page_size(page).expect("size");
        let width = 200u32;
        let scale = width as f32 / size.width_pt;
        let height = (size.height_pt * scale).max(1.0) as u32;

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut target = pdf_core::render::RenderTarget {
            width,
            height,
            stride: (width * 4) as usize,
            order: pdf_core::render::PixelOrder::Rgba,
            pixels: &mut pixels,
        };
        doc.page(page)
            .expect("page")
            .render_into(
                &pdf_core::document::RenderRequest { scale, ..Default::default() },
                &mut target,
            )
            .expect("render");

        pixels
            .chunks_exact(4)
            .filter(|p| p[0] > 180 && p[1] < 120 && p[2] > 100 && p[2] < 200)
            .count()
    }

    let mut doc = open("two-column.pdf");
    let size = doc.page_size(0).expect("size");
    let at = |fx: f32, fy: f32| Point { x: size.width_pt * fx, y: size.height_pt * fy };

    doc.add_annotation(
        0,
        &Annotation::Ink {
            strokes: vec![vec![at(0.2, 0.4), at(0.5, 0.5), at(0.7, 0.35)]],
            color: Color { r: 255, g: 20, b: 147, a: 255 },
            width: 3.0,
        },
    )
    .expect("add ink");
    assert!(pink(&doc, 0) > 0, "the ink did not draw, so this proves nothing");

    let area = Rect {
        left: size.width_pt * 0.15,
        top: size.height_pt * 0.45,
        right: size.width_pt * 0.75,
        bottom: size.height_pt * 0.68,
    };
    doc.lock_area(
        &Redaction { require_complete: false, ..Redaction::new(0, area) },
        PASSCODE,
        None,
    )
    .expect("lock the area");

    assert_eq!(doc.annotations(0).expect("annotations").len(), 0, "the mark is still on the page");
    assert_eq!(pink(&doc, 0), 0, "the ink is still drawn over the locked area");

    // And the passcode brings it back, which is what makes this a lock and not
    // an erasure.
    let sealed = doc.open_lock(PASSCODE).expect("unlock");
    for (index, pdf) in &sealed {
        doc.replace_page(*index, pdf).expect("restore");
    }
    assert_eq!(doc.annotations(0).expect("annotations").len(), 1, "the mark did not come back");
    assert!(pink(&doc, 0) > 0, "the mark came back but does not draw");
}

/// **Nothing new goes over a lock.**
///
/// A mark drawn after locking is not part of the seal: it would sit on top of
/// the black box, be lost on unlocking, and read to anyone else as a note about
/// content they cannot see. Refused rather than silently dropped, so the person
/// is told why.
#[test]
fn a_mark_cannot_be_drawn_over_a_locked_area() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::{Annotation, Color, Point};

    let mut doc = open("two-column.pdf");
    let size = doc.page_size(0).expect("size");
    let at = |fx: f32, fy: f32| Point { x: size.width_pt * fx, y: size.height_pt * fy };
    let ink = |a: (f32, f32), b: (f32, f32)| Annotation::Ink {
        strokes: vec![vec![at(a.0, a.1), at(b.0, b.1)]],
        color: Color { r: 255, g: 20, b: 147, a: 255 },
        width: 3.0,
    };

    let area = Rect {
        left: size.width_pt * 0.15,
        top: size.height_pt * 0.45,
        right: size.width_pt * 0.75,
        bottom: size.height_pt * 0.68,
    };
    doc.lock_area(
        &Redaction { require_complete: false, ..Redaction::new(0, area) },
        PASSCODE,
        None,
    )
    .expect("lock the area");

    // Over the lock: refused, with a reason worth reading.
    let over = doc.add_annotation(0, &ink((0.2, 0.5), (0.6, 0.6)));
    let problem = over.expect_err("a mark was drawn over the locked area").to_string();
    assert!(problem.contains("locked"), "unhelpful refusal: {problem}");
    assert_eq!(
        doc.annotations(0).expect("annotations").len(),
        0,
        "it refused and added the mark anyway"
    );

    // Elsewhere on the same page: perfectly fine. Locking part of a page does
    // not make the rest of it read-only.
    doc.add_annotation(0, &ink((0.1, 0.85), (0.5, 0.9)))
        .expect("a mark clear of the lock was refused");
    assert_eq!(doc.annotations(0).expect("annotations").len(), 1);

    // And a mark that puts no ink on the page is not a mark. The drawing tools
    // write a fully transparent one at the page origin to carry the data that
    // lets a stroke be erased again; refusing it made every drawing on the page
    // fail whenever a lock happened to cover that corner.
    let carrier = Annotation::Ink {
        strokes: vec![vec![at(0.2, 0.5), at(0.6, 0.6)]],
        color: Color { r: 0, g: 0, b: 0, a: 0 },
        width: 1.0,
    };
    doc.add_annotation(0, &carrier)
        .expect("an invisible carrier over the lock was refused");
}

/// And a locked *page* is covered edge to edge, so nothing can be drawn on it
/// at all.
#[test]
fn a_locked_page_takes_no_marks_anywhere() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    use pdf_core::document::{Annotation, Color, Point};

    let mut doc = open("two-column.pdf");
    doc.lock_pages(&[0], PASSCODE).expect("lock");

    let size = doc.page_size(0).expect("size");
    let at = |fx: f32, fy: f32| Point { x: size.width_pt * fx, y: size.height_pt * fy };
    let outcome = doc.add_annotation(
        0,
        &Annotation::Ink {
            strokes: vec![vec![at(0.05, 0.05), at(0.15, 0.1)]],
            color: Color { r: 255, g: 20, b: 147, a: 255 },
            width: 3.0,
        },
    );
    assert!(outcome.is_err(), "a locked page accepted a mark");
}

/// **One passcode locks the words and the picture alike.**
///
/// The vault holds a single key wrapped under a single passcode, so everything
/// sealed in a document shares it — asking somebody to invent a second passcode
/// for an image on a page they had already locked would be asking for something
/// that cannot exist. This is that property, end to end and through the file:
/// lock some text, lock an image with the same passcode, save, reopen, and get
/// both back with it.
///
/// Uses the catalogue because no committed fixture has text and an image on one
/// page; skips where it is not present.
#[test]
fn one_passcode_locks_the_text_and_the_image_together() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    // Long enough, and mixed enough, to be one the app would accept.
    const BOTH: &[u8] = b"Correct-Horse-99-Battery";

    let path = format!(
        "{}/Downloads/HSI CATALOG 2026.pdf",
        std::env::var("HOME").unwrap_or_default()
    );
    if !std::path::Path::new(&path).is_file() {
        eprintln!("skipping: no catalogue in ~/Downloads");
        return;
    }
    let Ok(mut doc) = PdfiumDocument::open_path(&path, None) else {
        eprintln!("skipping: the catalogue is present but would not open");
        return;
    };

    // A page carrying both.
    let Some(page) = (0..doc.page_count().min(60)).find(|p| {
        doc.images_on(*p).map(|i| !i.is_empty()).unwrap_or(false)
            && text_of(&doc, *p).chars().count() > 40
    }) else {
        eprintln!("skipping: no page with text and an image");
        return;
    };

    let before = text_of(&doc, page);
    let images_before = doc.images_on(page).expect("images").len();
    assert!(images_before > 0, "the page has no image after all");

    // -- the words -------------------------------------------------------
    let chars = doc.page(page).expect("page").characters().expect("characters");
    let words: Vec<&str> = before.split_whitespace().collect();
    let Some(phrase) = words
        .windows(2)
        .map(|w| format!("{} {}", w[0], w[1]))
        .find(|p| before.matches(p.as_str()).count() == 1 && p.chars().count() > 8)
    else {
        eprintln!("skipping: no phrase that appears exactly once");
        return;
    };
    let at = before.find(&phrase).expect("the phrase");
    let at = before[..at].chars().count();

    let mut area = Rect { left: f32::MAX, top: f32::MAX, right: f32::MIN, bottom: f32::MIN };
    for index in at..at + phrase.chars().count() {
        let Some(b) = chars.boxes.get(index * 4..index * 4 + 4) else { continue };
        area.left = area.left.min(b[0]);
        area.top = area.top.min(b[1]);
        area.right = area.right.max(b[2]);
        area.bottom = area.bottom.max(b[3]);
    }

    doc.lock_area(
        &Redaction { require_complete: false, ..Redaction::new(page, area) },
        BOTH,
        None,
    )
    .expect("lock the words");

    // -- the picture, with the very same passcode ------------------------
    let image = doc.images_on(page).expect("images")[0].object;
    let badge = doc.lock_image(page, image, BOTH).expect("lock the image with the same passcode");
    assert!(!badge.is_empty(), "the image lock recorded nothing to click");

    // -- through the file ------------------------------------------------
    let mut reopened = save_and_reopen(&mut doc);
    let after = text_of(&reopened, page);
    assert!(!after.contains(&phrase), "the words are still readable: {phrase:?}");

    // The passcode brings the page back whole.
    let pages = reopened.open_lock(BOTH).expect("the one passcode did not open both");
    assert!(!pages.is_empty(), "nothing came back");
    for (index, pdf) in &pages {
        reopened.replace_page(*index, pdf).expect("restore");
    }

    let restored = text_of(&reopened, page);
    assert!(
        restored.contains(&phrase),
        "the words did not come back with the passcode that hid them"
    );
    assert_eq!(
        reopened.images_on(page).expect("images").len(),
        images_before,
        "the picture did not come back with the same passcode"
    );
}

/// **An edit that cannot be made precisely is refused, not made badly.**
///
/// Reported from use twice: editing a word changed the text around it. The
/// content-stream path was already there and already right — what was wrong is
/// that when it could not handle a run it fell through to `FPDFText_SetText`,
/// which needs `FPDFPage_GenerateContent`, which re-emits the whole page.
///
/// So the failure mode was: the edit you asked for, plus a paragraph you did
/// not. Now it says so and changes nothing.
#[test]
fn a_run_that_cannot_be_edited_precisely_is_left_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);

    // A run that exists, asked to become something its font cannot draw.
    let runs = doc.text_runs(0).expect("runs");
    let Some(run) = runs.iter().find(|r| r.text.chars().count() > 8) else {
        eprintln!("skipping: no run long enough");
        return;
    };

    // Characters no Latin subset carries.
    let outcome = doc.set_text_run(0, run.object, "日本語のテキスト");
    assert!(outcome.is_err(), "it accepted characters the font cannot draw");
    assert_eq!(
        text_of(&doc, 0),
        before,
        "a refused edit changed the page anyway"
    );
}

/// And the ordinary case still works, precisely.
#[test]
fn an_edit_it_can_make_changes_only_what_was_asked() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);
    let runs = doc.text_runs(0).expect("runs");
    let Some(run) = runs.iter().find(|r| r.text.chars().count() > 8) else {
        eprintln!("skipping: no run long enough");
        return;
    };
    let was = run.text.clone();
    let object = run.object;

    let Ok(_) = doc.set_text_run(0, object, "CHANGED") else {
        eprintln!("skipping: this run is one of the ones it refuses");
        return;
    };

    let after = text_of(&doc, 0);
    assert!(after.contains("CHANGED"), "the edit did not take");
    assert!(!after.contains(&was), "the old words are still there");

    // Everything that was not the edited run is exactly as it was.
    let rest = |text: &str| {
        text.lines()
            .filter(|line| !line.contains("CHANGED") && !line.contains(was.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(rest(&after), rest(&before), "the text around the edit changed");
}

/// **A selection over two lines is not a rectangle.**
///
/// Reported from use as "the exact selected text isn't getting locked, the
/// whole line is" — and reported *twice*, because the first fix went in without
/// this test beside it. Nothing here was wrong: the engine cut precisely, the
/// single-line case was measured at 32 of 37 pages precise, and every test
/// passed. The app was collapsing the selection into the smallest rectangle
/// holding it before the engine ever saw it, and that rectangle holds the head
/// of the first line and the tail of the last as well.
///
/// Measured on this fixture: a 39-character selection took 68 characters and
/// the word before it. With the lines sent as themselves, 38 — the selection,
/// exactly.
#[test]
fn locking_a_selection_across_two_lines_leaves_the_ends_of_those_lines_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);

    // A phrase that runs off the end of one line and onto the next, with a word
    // before it on the first line and a word after it on the second. Those two
    // are what the union used to swallow.
    let (head, tail) = ("The", "ed");
    let phrase = "luminaire housing is formed from";
    assert!(
        before.contains(&format!("{head} {phrase}")),
        "the fixture is not what this test expects:\n{before:?}"
    );

    let chars = doc.page(0).expect("page").characters().expect("characters");
    let at = before.find(phrase).expect("the phrase");
    let at = before[..at].chars().count();
    // Past the line break and a few characters into the next line.
    let to = at + phrase.chars().count() + 8;

    let box_at = |i: usize| {
        let b = &chars.boxes[i * 4..i * 4 + 4];
        Rect { left: b[0], top: b[1], right: b[2], bottom: b[3] }
    };

    // One rect per line, which is what `Characters::line_rects` builds and what
    // the app now sends.
    let mut lines: Vec<Rect> = Vec::new();
    let mut current = box_at(at);
    for index in at + 1..to {
        let b = box_at(index);
        if b.top < current.bottom && b.bottom > current.top {
            current = Rect {
                left: current.left.min(b.left),
                top: current.top.min(b.top),
                right: current.right.max(b.right),
                bottom: current.bottom.max(b.bottom),
            };
        } else {
            lines.push(current);
            current = b;
        }
    }
    lines.push(current);
    assert!(lines.len() >= 2, "the selection should span two lines, not {}", lines.len());

    let request = Redaction {
        require_complete: false,
        ..Redaction::over(0, lines).expect("shapes")
    };
    doc.lock_area(&request, PASSCODE, None).expect("lock the selection");

    let after = text_of(&doc, 0);
    assert!(!after.contains(phrase), "the selection was not hidden:\n{after:?}");
    assert!(
        after.contains(head),
        "it took the word before the selection, on the line above:\n{after:?}"
    );
    assert!(
        after.contains(tail),
        "it took the words after the selection, on the line below:\n{after:?}"
    );
}

/// The union — what the app used to send — is measurably worse on the same
/// selection, which is the whole reason the shapes travel separately.
///
/// Kept as a test rather than a comment so that anyone tempted to simplify
/// `Redaction::parts` away is told what it costs.
#[test]
fn the_union_of_two_lines_takes_more_than_the_selection() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);
    let phrase = "luminaire housing is formed from";
    let at = before.find(phrase).expect("the phrase");
    let at = before[..at].chars().count();
    let to = at + phrase.chars().count() + 8;

    let chars = doc.page(0).expect("page").characters().expect("characters");
    let mut area = Rect { left: f32::MAX, top: f32::MAX, right: f32::MIN, bottom: f32::MIN };
    for index in at..to {
        let b = &chars.boxes[index * 4..index * 4 + 4];
        area.left = area.left.min(b[0]);
        area.top = area.top.min(b[1]);
        area.right = area.right.max(b[2]);
        area.bottom = area.bottom.max(b[3]);
    }

    let report = doc
        .lock_area(
            &Redaction { require_complete: false, ..Redaction::new(0, area) },
            PASSCODE,
            None,
        )
        .expect("lock the union");

    assert!(
        report.characters > to - at,
        "the union should over-remove — if it no longer does, this test has \
         stopped measuring anything ({} for a {}-character selection)",
        report.characters,
        to - at
    );
}

/// **Two pictures the same size on one page can still be locked.**
///
/// Reported from use, on a brochure: *"lock: two images of the same size on one
/// page, which cannot be told apart"* — and the padlock appeared anyway, over a
/// picture that was still perfectly visible.
///
/// The cause was the way a locked image was matched back to its PDF object:
/// by the pixel size PDFium reported, which two images of the same size share.
/// The content stream is what actually joins them — the picture is drawn by a
/// `Do` that names it — and this fixture has two 4 × 4 images precisely so that
/// the size can never be the discriminator again.
#[test]
fn two_pictures_of_the_same_size_can_be_told_apart() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("pictures.pdf");
    let pictures = doc.images_on(0).expect("images");
    assert_eq!(pictures.len(), 2, "the fixture should have two pictures");
    assert_eq!(
        pictures[0].pixel_width, pictures[1].pixel_width,
        "the fixture's pictures must be the same size, or this proves nothing"
    );

    // The fixture's two pictures are flat red and flat blue, so which one went
    // is readable off a render — where counting objects is not: a locked
    // picture is replaced by a blank mask, and PDFium still reports an image.
    let red = (0xE0, 0x20, 0x20);
    let blue = (0x20, 0x40, 0xE0);
    let was = colours(&doc, 0);
    assert!(near(&was, red) > 20, "the fixture should draw something red: {was:?}");
    assert!(near(&was, blue) > 20, "the fixture should draw something blue: {was:?}");

    let first = pictures[0].object;
    doc.lock_image(0, first, PASSCODE).expect("lock the first picture");

    let now = colours(&doc, 0);
    assert!(near(&now, red) < 5, "the locked picture is still on the page: {now:?}");
    assert!(near(&now, blue) > 20, "locking one picture took the other as well: {now:?}");
}

/// How much of a render is each colour, quantised so anti-aliasing does not
/// invent a bucket per edge pixel.
fn colours(doc: &dyn Document, page: usize) -> std::collections::BTreeMap<(u8, u8, u8), usize> {
    let size = doc.page_size(page).expect("size");
    let width = 300u32;
    let scale = width as f32 / size.width_pt;
    let height = (size.height_pt * scale).max(1.0) as u32;

    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut target = pdf_core::render::RenderTarget {
        width,
        height,
        stride: (width * 4) as usize,
        order: pdf_core::render::PixelOrder::Rgba,
        pixels: &mut pixels,
    };
    doc.page(page)
        .expect("page")
        .render_into(
            &pdf_core::document::RenderRequest { scale, ..Default::default() },
            &mut target,
        )
        .expect("render");

    let mut counts = std::collections::BTreeMap::new();
    for p in pixels.chunks_exact(4) {
        *counts.entry((p[0], p[1], p[2])).or_insert(0usize) += 1;
    }
    counts
}

/// How many pixels sit near a colour, which is what survives anti-aliasing.
fn near(counts: &std::collections::BTreeMap<(u8, u8, u8), usize>, want: (u8, u8, u8)) -> usize {
    counts
        .iter()
        .filter(|((r, g, b), _)| {
            (*r as i32 - want.0 as i32).abs() < 40
                && (*g as i32 - want.1 as i32).abs() < 40
                && (*b as i32 - want.2 as i32).abs() < 40
        })
        .map(|(_, n)| *n)
        .sum()
}

/// **And the badge does not appear when the picture does not go.**
///
/// The vault is written before the page is changed, so a failure in between
/// leaves a document claiming a lock that is not there. Asserted by counting
/// badges against pictures rather than by forcing a failure, which no fixture
/// here can do now that the identification is exact.
#[test]
fn a_padlock_never_stands_over_a_picture_that_is_still_there() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("pictures.pdf");
    // Named up front. A locked picture is replaced by a blank mask rather than
    // removed, so it is still an image object afterwards and re-reading the
    // list would hand back the one just locked.
    let both: Vec<usize> = doc.images_on(0).expect("images").iter().map(|i| i.object).collect();
    let before = both.len();
    assert_eq!(before, 2, "the fixture should have two pictures");

    for object in &both {
        doc.lock_image(0, *object, PASSCODE).expect("lock a picture");
    }

    let badges = doc.locked_items_on(0).expect("badges").len();
    assert_eq!(badges, before, "a badge is missing for a picture that did go");

    // And nothing either badge stands over is still being drawn.
    let now = colours(&doc, 0);
    assert!(near(&now, (0xE0, 0x20, 0x20)) < 5, "a padlock stands over a picture still on the page: {now:?}");
    assert!(near(&now, (0x20, 0x40, 0xE0)) < 5, "a padlock stands over a picture still on the page: {now:?}");
}

/// **Unlocking one thing does not release another.**
///
/// Reported from use: *"the locked text became visible again somehow but the
/// padlock is there"*. Undoing a lock replaces the whole page with its sealed
/// original — which brings back everything that was ever taken off it — and the
/// step that re-hides things afterwards only knew about pictures. A locked
/// *area* is a shape somebody drew and is nowhere on the page to be found, so
/// it came back and stayed back, with its badge still sitting over it.
#[test]
fn unlocking_a_picture_leaves_the_locked_words_on_that_page_locked() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("pictures.pdf");
    let before = text_of(&doc, 0);
    let phrase = "A paragraph that must not move";
    assert!(before.contains(phrase), "the fixture is not what this test expects");

    // Lock the words, then a picture — the order the report described.
    let chars = doc.page(0).expect("page").characters().expect("characters");
    let at = before.find(phrase).expect("the phrase");
    let at = before[..at].chars().count();
    let mut area = Rect { left: f32::MAX, top: f32::MAX, right: f32::MIN, bottom: f32::MIN };
    for index in at..at + phrase.chars().count() {
        let b = &chars.boxes[index * 4..index * 4 + 4];
        area.left = area.left.min(b[0]);
        area.top = area.top.min(b[1]);
        area.right = area.right.max(b[2]);
        area.bottom = area.bottom.max(b[3]);
    }
    doc.lock_area(
        &Redaction { require_complete: false, ..Redaction::new(0, area) },
        PASSCODE,
        None,
    )
    .expect("lock the words");
    assert!(!text_of(&doc, 0).contains(phrase), "the words were not hidden");

    let picture = doc.images_on(0).expect("images").first().map(|i| i.object).expect("a picture");
    doc.lock_image(0, picture, PASSCODE).expect("lock the picture");
    assert!(!text_of(&doc, 0).contains(phrase), "locking a picture brought the words back");

    // Now let the picture go. The words must stay gone.
    let badge = doc
        .locked_items_on(0)
        .expect("badges")
        .into_iter()
        .find(|i| !i.is_area)
        .expect("the picture's badge");
    doc.unlock_item(&badge.id, PASSCODE).expect("unlock the picture");

    assert_eq!(doc.images_on(0).expect("images").len(), 2, "the picture did not come back");
    assert!(
        !text_of(&doc, 0).contains(phrase),
        "unlocking the picture brought the locked words back:\n{}",
        text_of(&doc, 0)
    );
    // And the words' badge is still the one thing standing over them.
    let left = doc.locked_items_on(0).expect("badges");
    assert_eq!(left.len(), 1, "the badges are not what is actually locked: {left:#?}");
    assert!(left[0].is_area, "the surviving badge should be the locked words");
}

/// **A badge over a picture that is still there is finished, not left.**
///
/// Reported from use, with a screenshot: a grey layer over a picture that could
/// not be selected, moved, or sent back. It was not a layer. It was the
/// chequerboard the app draws over a *locked* picture — over a picture that
/// had never been taken off the page, because the lock recorded its badge
/// first and then failed to find the picture among two of the same size. The
/// cause is fixed; the documents written while it was not still carry the
/// badge.
///
/// Built here the way such a document actually is: a real lock, saved, with
/// the picture's original bytes put back behind the badge.
#[test]
fn a_lock_that_never_took_its_picture_is_completed_on_repair() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    // The picture's own object, before anything happens to it.
    let original = {
        let mut doc = open("pictures.pdf");
        let mut bytes = Vec::new();
        doc.save_full_copy(&mut bytes).expect("save");
        let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
        let mut found = None;
        for number in 1..64u32 {
            if let Ok(pdf_core::pdf::Object::Stream(dict, span)) = file.object(number) {
                if dict.get(b"Subtype").and_then(pdf_core::pdf::Object::as_name) == Some(&b"Image"[..])
                    && dict.get(b"Width").and_then(pdf_core::pdf::Object::as_i64) == Some(4)
                {
                    // The stream's raw bytes, exactly as the file holds them.
                    found = Some((number, pdf_core::pdf::write_stream(&dict, &bytes[span])));
                    break;
                }
            }
        }
        found.expect("the fixture's first picture")
    };

    // A real lock, then the picture put back behind its badge — which is what
    // a document written by the version with the bug looks like.
    let mut doc = open("pictures.pdf");
    let picture = doc.images_on(0).expect("images")[0].object;
    doc.lock_image(0, picture, PASSCODE).expect("lock");
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
    let broken = file.rewrite(&[original]).expect("put the picture back");
    let mut doc = PdfiumDocument::open_bytes(broken, None).expect("reopen");

    // Which is exactly the state that was reported.
    let badges = doc.locked_items_on(0).expect("badges");
    assert_eq!(badges.len(), 1);
    assert!(badges[0].stale, "the badge should know its picture is still there");
    assert!(near(&colours(&doc, 0), (0xE0, 0x20, 0x20)) > 20, "the picture should still be drawn");

    // Repair finishes the lock.
    let (completed, dropped) = doc.repair_locks().expect("repair");
    assert_eq!((completed, dropped), (1, 0), "the lock should have been completed, not dropped");
    let badges = doc.locked_items_on(0).expect("badges");
    assert_eq!(badges.len(), 1, "the badge should still be there — it is now true");
    assert!(!badges[0].stale);
    assert!(
        near(&colours(&doc, 0), (0xE0, 0x20, 0x20)) < 5,
        "the picture is still on the page after repair"
    );

    // And the passcode still brings it back, which is the whole point of the
    // badge having been kept.
    doc.unlock_item(&badges[0].id, PASSCODE).expect("unlock");
    assert!(near(&colours(&doc, 0), (0xE0, 0x20, 0x20)) > 20, "the picture did not come back");
}

/// **A document with nothing wrong is left exactly alone by repair.**
#[test]
fn repair_touches_nothing_when_every_lock_is_whole() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("pictures.pdf");
    assert_eq!(doc.repair_locks().expect("repair"), (0, 0), "nothing is locked");

    let picture = doc.images_on(0).expect("images")[0].object;
    doc.lock_image(0, picture, PASSCODE).expect("lock");
    assert_eq!(doc.repair_locks().expect("repair"), (0, 0), "a whole lock is not repaired");
}
