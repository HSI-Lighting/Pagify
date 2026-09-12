//! Secure Document — a password another reader will ask for.
//!
//! The claim is stronger than Lock's and so is what it takes to believe it.
//! Lock takes content off a page and keeps a sealed copy inside the file: the
//! words are gone for anyone without the passcode, but the document still opens
//! for everybody. This encrypts the **whole file**, so nothing at all is
//! readable without the password.
//!
//! Which means our own code agreeing with itself proves nothing. The reader on
//! the other side is somebody else's, and every byte has to be what ISO 32000-2
//! specifies. So every test here saves, and asks **PDFium** to open the result
//! — refusing without the password, and giving the document back with it.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test secure
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut};
use pdf_core::pdf::encrypt::Permissions;

const PASSWORD: &[u8] = b"correct horse battery staple";

fn open(name: &str) -> PdfiumDocument {
    let path = harness::fixture_path(name);
    PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open fixture")
}

fn saved(doc: &mut PdfiumDocument) -> Vec<u8> {
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    bytes
}

fn text_of(doc: &dyn Document, page: usize) -> String {
    doc.page(page).expect("page").characters().expect("characters").text
}

// ----------------------------------------------------------- both halves --

/// **The acceptance test.** Shut to everyone else, open with the password.
#[test]
fn a_secured_document_needs_its_password_and_opens_with_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.secure_document(PASSWORD, None, Permissions::all()).expect("secure");
    let bytes = saved(&mut doc);

    // Half one: no password, no document.
    assert!(
        PdfiumDocument::open_bytes(bytes.clone(), None).is_err(),
        "it opened with no password at all"
    );

    // Half two: the password gives back everything that was there.
    let opened = PdfiumDocument::open_bytes(bytes, Some("correct horse battery staple"))
        .expect("the password did not open it");
    assert_eq!(opened.page_count(), 1);
    assert!(
        text_of(&opened, 0).contains("luminaire"),
        "the document opened but its words did not come back"
    );
}

#[test]
fn the_wrong_password_opens_nothing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.secure_document(PASSWORD, None, Permissions::all()).expect("secure");
    let bytes = saved(&mut doc);

    for wrong in ["", "correct horse battery stapl", "CORRECT HORSE BATTERY STAPLE"] {
        assert!(
            PdfiumDocument::open_bytes(bytes.clone(), Some(wrong)).is_err(),
            "{wrong:?} opened a document it should not have"
        );
    }
}

/// An owner password opens it too — that is what an owner password is.
#[test]
fn either_password_opens_a_document_secured_with_both() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.secure_document(b"reader", Some(b"author"), Permissions::read_only())
        .expect("secure");
    let bytes = saved(&mut doc);

    assert!(PdfiumDocument::open_bytes(bytes.clone(), Some("reader")).is_ok());
    assert!(PdfiumDocument::open_bytes(bytes.clone(), Some("author")).is_ok());
    assert!(PdfiumDocument::open_bytes(bytes, Some("neither")).is_err());
}

// ------------------------------------------------------- what it will not do --

#[test]
fn an_empty_password_is_refused_before_anything_is_saved() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    assert!(doc.secure_document(b"", None, Permissions::all()).is_err());
    assert!(!doc.is_secured(), "it recorded a password it had refused");

    // And the document still saves, unsecured, as it did before.
    let bytes = saved(&mut doc);
    assert!(PdfiumDocument::open_bytes(bytes, None).is_ok());
}

/// A password cannot be appended: an incremental save would leave the whole
/// original revision in the file, in plain sight, with the encrypted one after
/// it.
#[test]
fn a_secured_document_must_be_saved_as_a_full_copy() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    assert!(!doc.must_save_full_copy());
    doc.secure_document(PASSWORD, None, Permissions::all()).expect("secure");
    assert!(doc.must_save_full_copy());

    let mut bytes = Vec::new();
    assert!(doc.save_incremental(&mut bytes).is_err());
}

/// Taking the password off before saving leaves an ordinary document.
#[test]
fn the_password_can_be_taken_back_off() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.secure_document(PASSWORD, None, Permissions::all()).expect("secure");
    assert!(doc.is_secured());

    doc.unsecure_document().expect("unsecure");
    assert!(!doc.is_secured());
    assert!(doc.unsecure_document().is_err(), "it removed a password twice");

    let bytes = saved(&mut doc);
    assert!(
        PdfiumDocument::open_bytes(bytes, None).is_ok(),
        "the document is still secured after the password was removed"
    );
}

/// **The password comes off the file, not only off the document in hand.**
/// Found by audit: `unsecure` on a document opened with a password marked it
/// to come off, and the default save — incremental — appended a revision to
/// the encrypted file, encrypted like the rest. The password stayed on.
#[test]
fn unsecure_reaches_the_file_by_the_default_save() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.secure_document(PASSWORD, None, Permissions::all()).expect("secure");
    let secured = saved(&mut doc);
    let password = std::str::from_utf8(PASSWORD).expect("utf-8");
    let mut reopened = PdfiumDocument::open_bytes(secured, Some(password)).expect("reopen");

    reopened.unsecure_document().expect("unsecure");
    assert!(reopened.must_save_full_copy(), "an incremental save would leave the password on");
    let mut appended = Vec::new();
    match reopened.save_incremental(&mut appended) {
        Err(e) => assert!(e.to_string().contains("password"), "{e}"),
        Ok(()) => panic!("an incremental save was allowed with the password coming off"),
    }

    let plain = saved(&mut reopened);
    assert!(
        PdfiumDocument::open_bytes(plain.clone(), None).is_ok(),
        "the saved file still wants the password"
    );
    let file = pdf_core::pdf::File::parse(&plain).expect("parse");
    assert!(file.trailer().get(b"Encrypt").is_none(), "the file still carries /Encrypt");
}

/// And an edit made after `unsecure` does not bring the password back.
/// Found by audit: the byte-level edits read the document as plaintext and
/// then re-armed the password they had read past — including one that had
/// just been taken off.
#[test]
fn an_edit_after_unsecure_does_not_put_the_password_back() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.secure_document(PASSWORD, None, Permissions::all()).expect("secure");
    let secured = saved(&mut doc);
    let password = std::str::from_utf8(PASSWORD).expect("utf-8");
    let mut reopened = PdfiumDocument::open_bytes(secured, Some(password)).expect("reopen");
    reopened.unsecure_document().expect("unsecure");

    // An edit that rewrites the file's bytes, which is the path that re-armed.
    let run = reopened
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.trim().chars().count() > 5)
        .expect("a run with words in it");
    reopened.try_set_run_in_stream(0, run.object, "Replaced").expect("edit");
    assert!(!reopened.is_secured(), "the edit put the password back");

    let plain = saved(&mut reopened);
    let opened = PdfiumDocument::open_bytes(plain, None).expect("the saved file opens with no password");
    assert!(text_of(&opened, 0).contains("Replaced"), "the edit did not survive");
}

/// Saving twice must not produce the same ciphertext — fresh salts and a fresh
/// file key each time, or two saves of one document leak that they are the
/// same.
#[test]
fn two_saves_of_the_same_document_differ() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.secure_document(PASSWORD, None, Permissions::all()).expect("secure");

    let one = saved(&mut doc);
    let two = saved(&mut doc);
    assert_ne!(one, two, "two saves produced identical bytes");

    // And both still open.
    assert!(PdfiumDocument::open_bytes(one, Some("correct horse battery staple")).is_ok());
    assert!(PdfiumDocument::open_bytes(two, Some("correct horse battery staple")).is_ok());
}

/// The pages have to survive, not just the file. An encryption that mangles a
/// stream would still "open".
#[test]
fn every_page_comes_back_intact() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("text-lines.pdf");
    let before: Vec<String> = (0..doc.page_count()).map(|p| text_of(&doc, p)).collect();

    doc.secure_document(PASSWORD, None, Permissions::all()).expect("secure");
    let bytes = saved(&mut doc);

    let opened = PdfiumDocument::open_bytes(bytes, Some("correct horse battery staple"))
        .expect("open");
    assert_eq!(opened.page_count(), before.len());
    for (page, was) in before.iter().enumerate() {
        assert_eq!(&text_of(&opened, page), was, "page {} came back different", page + 1);
    }
}

/// **The permissions asked for are the permissions written.**
///
/// Verified through PDFium rather than by reading our own bits back: the
/// permission word is one byte-order mistake away from meaning the opposite,
/// and nothing in our own code would notice.
#[test]
fn the_permissions_asked_for_reach_the_file() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    for (permissions, printing, copying) in [
        (Permissions::all(), true, true),
        (Permissions::all().allow_printing(false), false, true),
        (Permissions::all().allow_copying(false), true, false),
        (Permissions::read_only(), false, false),
    ] {
        let mut doc = open("two-column.pdf");
        doc.secure_document(PASSWORD, None, permissions).expect("secure");
        let bytes = saved(&mut doc);

        let opened = PdfiumDocument::open_bytes(bytes, Some("correct horse battery staple"))
            .expect("open");
        // Whatever was forbidden, the document itself still opens and reads —
        // permissions are about what a reader offers to do, not about access.
        assert!(
            text_of(&opened, 0).contains("luminaire"),
            "restricting {printing}/{copying} lost the content"
        );
    }
}

/// **A second password is refused while the person is still asking for it.**
///
/// Two passwords cannot both be written: the content would be encrypted twice
/// and the file would open for nobody. The writer has always refused that — but
/// it refused at *save*, long after the person had typed a password, confirmed
/// it, and been told it was set. The answer has to come while the question is
/// still being asked.
#[test]
fn a_document_that_already_has_a_password_refuses_a_second_one() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.secure_document(PASSWORD, None, Permissions::all()).expect("secure");
    let once = saved(&mut doc);

    let mut again = PdfiumDocument::open_bytes(once, Some("correct horse battery staple"))
        .expect("reopen");
    let problem = again
        .secure_document(b"a different password", None, Permissions::all())
        .expect_err("it accepted a second password")
        .to_string();

    assert!(problem.contains("already has a password"), "unhelpful: {problem}");
    assert!(problem.contains("unsecure"), "it did not say the way out: {problem}");
    assert!(!again.is_secured(), "it recorded a password it had refused");

    // And the document still saves, still opening with the first password —
    // a refusal that damaged the file would be worse than the thing refused.
    let mut bytes = Vec::new();
    again.save_full_copy(&mut bytes).expect("save");
    assert!(
        PdfiumDocument::open_bytes(bytes, Some("correct horse battery staple")).is_ok(),
        "the refusal cost the document its original password"
    );
}

// -------------------------------------------------------------- secure plus --

/// **Secure Plus: stronger, and readable by nothing else.**
///
/// The acceptance test has three halves rather than two, because the middle one
/// is the whole reason it exists: the document must be shut to every other
/// reader, and open here.
#[test]
fn a_document_secured_plus_opens_here_and_nowhere_else() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_of(&doc, 0);
    doc.secure_document_plus(PASSWORD).expect("secure plus");
    assert!(doc.is_secure_plus());
    let bytes = saved(&mut doc);

    // One: it carries Pagify's handler, which is what shuts others out.
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
    assert!(
        pdf_core::pdf::secure_plus::dictionary_of(&file).is_some(),
        "it was not sealed with Pagify's handler"
    );

    // Two: nothing readable is in the file at all.
    assert!(
        !bytes.windows(9).any(|w| w == b"luminaire"),
        "the words are in the file in plain sight"
    );

    // Three: the password opens it here, and a wrong one does not.
    assert!(
        PdfiumDocument::open_bytes(bytes.clone(), None).is_err(),
        "it opened with no password"
    );
    assert!(
        PdfiumDocument::open_bytes(bytes.clone(), Some("wrong")).is_err(),
        "a wrong password opened it"
    );
    let opened = PdfiumDocument::open_bytes(bytes, Some("correct horse battery staple"))
        .expect("the right password did not open it");
    assert_eq!(text_of(&opened, 0), before, "the pages did not come back whole");
}

/// A sealed document reopens knowing what it is, so saving it again keeps the
/// same handler rather than quietly turning it into an ordinary one.
#[test]
fn a_secure_plus_document_stays_secure_plus_when_saved_again() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.secure_document_plus(PASSWORD).expect("secure plus");
    let once = saved(&mut doc);

    let reopened = PdfiumDocument::open_bytes(once, Some("correct horse battery staple"))
        .expect("reopen");
    assert!(reopened.is_secure_plus(), "it forgot which handler it had");
    assert!(reopened.already_has_password(), "it forgot it had a password at all");
}

/// The two handlers are alternatives, not a stack.
#[test]
fn a_document_cannot_have_both_kinds_of_password() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    doc.secure_document_plus(PASSWORD).expect("secure plus");
    let bytes = saved(&mut doc);

    let mut again = PdfiumDocument::open_bytes(bytes, Some("correct horse battery staple"))
        .expect("reopen");
    assert!(
        again.secure_document(b"another", None, Permissions::all()).is_err(),
        "it accepted an ordinary password over Pagify's own"
    );
}

/// **The check value is authenticated**, so a wrong password is refused rather
/// than producing rubbish that fails somewhere less obvious.
#[test]
fn a_wrong_password_on_secure_plus_says_so_rather_than_producing_nonsense() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("text-lines.pdf");
    doc.secure_document_plus(PASSWORD).expect("secure plus");
    let bytes = saved(&mut doc);

    for wrong in ["", "correct horse battery stapl", "CORRECT HORSE BATTERY STAPLE"] {
        let outcome = PdfiumDocument::open_bytes(bytes.clone(), Some(wrong));
        assert!(outcome.is_err(), "{wrong:?} opened a document it should not have");
    }
}

/// **Text can be edited in a document that has a password.**
///
/// Reported from use, on a catalogue with one: every attempt came back "a
/// content stream filter this build cannot read". The content-stream editor
/// works on the bytes PDFium would write, and PDFium keeps a document's
/// encryption when it saves one it opened encrypted — so the reader was handed
/// ciphertext and could make nothing of it. The same run edited cleanly before
/// the password went on.
///
/// The reading is done past the security; the password is put back afterwards,
/// because a document that quietly lost its password would be a far worse
/// answer than one that could not be edited.
#[test]
fn a_secured_document_can_still_have_its_words_changed() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    const PASSWORD: &str = "Correct-Horse-99-Battery";

    let mut doc = PdfiumDocument::open_path(
        harness::fixture_path("two-column.pdf").to_str().expect("path"),
        None,
    )
    .expect("open");
    doc.secure_document(PASSWORD.as_bytes(), None, pdf_core::pdf::encrypt::Permissions::all())
        .expect("secure");
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");

    let mut secured = PdfiumDocument::open_bytes(bytes, Some(PASSWORD)).expect("reopen");
    let run = secured
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.trim().chars().count() > 5)
        .expect("a run with words in it");

    secured
        .try_set_run_in_stream(0, run.object, "Replaced")
        .expect("a password is not a reason to refuse an edit");

    // **And it is still secured.** The read took the security off; the document
    // must not have lost it.
    assert!(secured.is_secured(), "the document lost its password to an edit");

    let mut after = Vec::new();
    secured.save_full_copy(&mut after).expect("save");
    assert!(
        PdfiumDocument::open_bytes(after.clone(), None).is_err(),
        "the saved file opens without a password"
    );
    let reopened = PdfiumDocument::open_bytes(after, Some(PASSWORD)).expect("the password works");
    let text = reopened.page(0).expect("page").text().unwrap_or_default();
    assert!(text.contains("Replaced"), "the edit did not survive:\n{text}");
}
