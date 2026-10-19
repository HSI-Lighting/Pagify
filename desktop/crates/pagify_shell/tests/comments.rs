//! `managecomments`' listing — against a document with a mark added by the
//! test itself (the index the list hands out is the index the removal takes),
//! and against a hand-written document carrying the PDF's own reply links
//! (`/IRT`), so a thread another application made reads back as one.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test comments
//! ```

use pagify_shell::{comments, Session};

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn a_note_lands_in_the_list_and_can_be_removed_by_its_listed_index() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    assert!(
        comments::comments(&session).expect("list").is_empty(),
        "the fixture starts with no annotations"
    );

    session
        .execute(pdf_core::command::Command::AddAnnotation {
            page_index: 0,
            annotation: pdf_core::document::Annotation::Note {
                rect: pdf_core::document::Rect {
                    left: 40.0,
                    top: 40.0,
                    right: 60.0,
                    bottom: 60.0,
                },
                contents: "check this figure".into(),
                color: pdf_core::document::Color { r: 255, g: 220, b: 0, a: 255 },
                reply_to: None,
            },
        })
        .expect("add a note");

    let listed = comments::comments(&session).expect("list");
    assert_eq!(listed.len(), 1, "one note was added: {listed:?}");
    let row = &listed[0];
    assert_eq!(row.page, 0);
    assert_eq!(row.kind, "note");
    assert!(row.detail.contains("check this figure"), "no contents: {row:?}");

    session
        .execute(pdf_core::command::Command::RemoveAnnotation {
            page_index: row.page,
            index: row.index,
        })
        .expect("remove by the listed index");

    assert!(
        comments::comments(&session).expect("list").is_empty(),
        "the removal did not take the listed annotation"
    );
}

#[test]
fn a_picture_reads_as_its_pixel_size() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    session
        .execute(pdf_core::command::Command::AddAnnotation {
            page_index: 0,
            annotation: pdf_core::document::Annotation::Image {
                rect: pdf_core::document::Rect {
                    left: 100.0,
                    top: 100.0,
                    right: 140.0,
                    bottom: 130.0,
                },
                rgba: vec![0u8; 4 * 8 * 6],
                width: 8,
                height: 6,
            },
        })
        .expect("add a picture");

    let listed = comments::comments(&session).expect("list");
    let row = listed.first().expect("one row");
    assert_eq!(row.kind, "picture");
    assert_eq!(row.detail, "8×6");
}

/// A small, valid PDF with the objects given, written with a plain xref
/// table — the shape PDFium opens and `pdf_core::pdf` reads.
fn pdf(objects: &[&str]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// One page, three text annotations: a root note, a reply to it, and a reply
/// to the reply — the PDF's own `/IRT` thread construct.
fn threaded_pdf() -> Vec<u8> {
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 4 0 R \
         /Annots [5 0 R 6 0 R 7 0 R] >>",
        "<< /Length 0 >>\nstream\n\nendstream",
        "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (first) \
         /NM (root) >>",
        "<< /Type /Annot /Subtype /Text /Rect [40 10 60 30] /Contents (second) \
         /IRT 5 0 R /RT /R /NM (reply-one) >>",
        "<< /Type /Annot /Subtype /Text /Rect [70 10 90 30] /Contents (third) \
         /IRT 6 0 R /RT /R /NM (reply-two) >>",
    ])
}

/// Write `bytes` somewhere no other test uses, and hand back the path.
fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir()
        .join(format!("pagify-comments-{name}-{}-{n}.pdf", std::process::id()));
    std::fs::write(&path, bytes).expect("the fixture could not be written");
    path
}

fn reply_to(mark: &pdf_core::document::IndexedAnnotation) -> Option<usize> {
    match &mark.annotation {
        pdf_core::document::Annotation::Note { reply_to, .. } => *reply_to,
        other => panic!("expected a note, got {other:?}"),
    }
}

/// **A thread made by another application reads back as one.** The engine
/// reads the file's `/IRT` link, resolves it to the parent annotation the page
/// already holds, and the panel's own ordering indents the replies under it.
#[test]
fn a_thread_another_application_made_reads_back_with_its_replies() {
    let path = temp("thread", &threaded_pdf());
    let session = Session::open(&path).expect("open the threaded fixture");
    let marks = session.annotations(0).expect("annotations");
    assert_eq!(marks.len(), 3, "three notes were written: {marks:?}");
    assert_eq!(reply_to(&marks[0]), None, "the root answers nothing");
    assert_eq!(reply_to(&marks[1]), Some(marks[0].index), "the first reply answers the root");
    assert_eq!(reply_to(&marks[2]), Some(marks[1].index), "the second answers the first");

    let rows = comments::thread_rows(comments::comments(&session).expect("list"));
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows.iter().map(|row| row.depth).collect::<Vec<_>>(), [0, 1, 2], "{rows:?}");
    assert!(rows[0].comment.detail.contains("first"), "{rows:?}");
    assert!(rows[1].comment.detail.contains("second"), "{rows:?}");
    std::fs::remove_file(&path).ok();
}

/// **The thread survives this engine's own save.** PDFium copies annotation
/// dictionaries it did not write, so a `/IRT` link read from a file is still
/// there in the file this program writes.
#[test]
fn a_thread_survives_this_engines_own_save() {
    let source = temp("round-trip", &threaded_pdf());
    let saved = std::env::temp_dir()
        .join(format!("pagify-comments-saved-{}.pdf", std::process::id()));
    let session = Session::open(&source).expect("open the threaded fixture");
    session.save_to(&saved, false).expect("save");
    let reopened = Session::open(&saved).expect("reopen the saved file");
    let marks = reopened.annotations(0).expect("annotations");
    assert_eq!(marks.len(), 3, "{marks:?}");
    assert_eq!(reply_to(&marks[0]), None);
    assert_eq!(reply_to(&marks[1]), Some(marks[0].index), "the first reply lost its /IRT");
    assert_eq!(reply_to(&marks[2]), Some(marks[1].index), "the second lost its /IRT");
    std::fs::remove_file(&source).ok();
    std::fs::remove_file(&saved).ok();
}

/// A note to hang a thread on, added through the engine's ordinary path.
fn note(session: &Session, page: usize, left: f32, words: &str) -> usize {
    session
        .note(
            page,
            pdf_core::document::Rect {
                left,
                top: 20.0,
                right: left + 20.0,
                bottom: 40.0,
            },
            words.into(),
            pdf_core::document::Color { r: 255, g: 220, b: 0, a: 255 },
        )
        .expect("a note");
    session
        .annotations(page)
        .expect("annotations")
        .into_iter()
        .find(|mark| matches!(&mark.annotation, pdf_core::document::Annotation::Note { contents, .. } if contents == words))
        .expect("the note just added")
        .index
}

fn note_contents(mark: &pdf_core::document::IndexedAnnotation) -> String {
    match &mark.annotation {
        pdf_core::document::Annotation::Note { contents, .. } => contents.clone(),
        other => panic!("expected a note, got {other:?}"),
    }
}

/// **The write half: a reply made here is a reply before the file is touched,
/// and the `/IRT` link reaches the file on save.** The in-session answer comes
/// from the engine's own record; the reopened one from the file's dictionary,
/// which is the proof that the save patch wrote it.
#[test]
fn a_reply_made_here_is_threaded_before_and_after_a_save() {
    let source = std::env::temp_dir().join(format!("pagify-reply-src-{}.pdf", std::process::id()));
    let saved = std::env::temp_dir().join(format!("pagify-reply-saved-{}.pdf", std::process::id()));
    std::fs::copy(fixture("two-column.pdf"), &source).expect("copy the fixture");

    let session = Session::open(&source).expect("open");
    let root = note(&session, 0, 20.0, "root words");
    session
        .reply_note(
            0,
            root,
            pdf_core::document::Rect { left: 40.0, top: 34.0, right: 60.0, bottom: 54.0 },
            "reply words".into(),
            pdf_core::document::Color { r: 255, g: 220, b: 0, a: 255 },
        )
        .expect("a reply");

    // Before any save: the engine's own record says it is a reply.
    let marks = session.annotations(0).expect("annotations");
    let reply = marks
        .iter()
        .find(|mark| note_contents(mark) == "reply words")
        .expect("the reply just added");
    assert_eq!(reply_to(reply), Some(root), "the reply does not answer the root");
    let rows = comments::thread_rows(comments::comments(&session).expect("list"));
    assert_eq!(rows.iter().map(|row| row.depth).collect::<Vec<_>>(), [0, 1], "{rows:?}");

    // After a save and reopen: the same relation, read from the file's `/IRT`.
    session.save_to(&saved, false).expect("save");
    let reopened = Session::open(&saved).expect("reopen the saved file");
    let marks = reopened.annotations(0).expect("annotations");
    assert_eq!(marks.len(), 2, "the two notes are not both in the saved file: {marks:?}");
    let root_mark = marks
        .iter()
        .find(|mark| note_contents(mark) == "root words")
        .expect("the root in the saved file");
    let reply_mark = marks
        .iter()
        .find(|mark| note_contents(mark) == "reply words")
        .expect("the reply in the saved file");
    assert_eq!(
        reply_to(reply_mark),
        Some(root_mark.index),
        "the saved file's /IRT link does not name the root"
    );

    std::fs::remove_file(&source).ok();
    std::fs::remove_file(&saved).ok();
}

/// Removing a reply takes its link with it: the file that is saved afterwards
/// has the root and no reply, rather than a link to something gone.
#[test]
fn a_removed_reply_leaves_the_file_without_a_thread() {
    let source = std::env::temp_dir().join(format!("pagify-reply-rm-src-{}.pdf", std::process::id()));
    let saved = std::env::temp_dir().join(format!("pagify-reply-rm-saved-{}.pdf", std::process::id()));
    std::fs::copy(fixture("two-column.pdf"), &source).expect("copy the fixture");

    let session = Session::open(&source).expect("open");
    let root = note(&session, 0, 20.0, "root words");
    session
        .reply_note(
            0,
            root,
            pdf_core::document::Rect { left: 40.0, top: 34.0, right: 60.0, bottom: 54.0 },
            "reply words".into(),
            pdf_core::document::Color { r: 255, g: 220, b: 0, a: 255 },
        )
        .expect("a reply");
    let reply = session
        .annotations(0)
        .expect("annotations")
        .into_iter()
        .find(|mark| note_contents(mark) == "reply words")
        .expect("the reply")
        .index;
    session
        .execute(pdf_core::command::Command::RemoveAnnotation { page_index: 0, index: reply })
        .expect("remove the reply");

    session.save_to(&saved, false).expect("save");
    let reopened = Session::open(&saved).expect("reopen the saved file");
    let marks = reopened.annotations(0).expect("annotations");
    assert_eq!(marks.len(), 1, "the removed reply is still in the saved file: {marks:?}");
    assert_eq!(reply_to(&marks[0]), None);
    assert_eq!(note_contents(&marks[0]), "root words");

    std::fs::remove_file(&source).ok();
    std::fs::remove_file(&saved).ok();
}
