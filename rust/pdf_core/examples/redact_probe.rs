//! Two questions redaction cannot answer from its own tests.
//!
//! **Is the text really gone from the bytes?** The suite reopens the saved file
//! with PDFium, which is a different code path from the writer but not a
//! different implementation. This writes the file out so a reader with nothing
//! in common with it — see `tools/verify_redaction.py` — can be pointed at it.
//!
//! **Should a path crossing the rectangle refuse the redaction?** The reasoning
//! says no for a table rule and yes for outlined type, and reasoning is not
//! measurement. This counts both populations over real documents.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example redact_probe -- <file.pdf> [needle]
//! ```

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, PageTextKind, Rect, Redaction, Uncleared};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: redact_probe <file.pdf> [needle]");
        std::process::exit(2);
    };
    let needle = args.next();

    let doc = PdfiumDocument::open_path(&path, None).expect("open");
    let pages = doc.page_count();
    println!("{path}\n{pages} pages\n");

    // -- what a redaction rectangle actually meets, page by page --------------
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    let mut blocked = 0usize;
    let mut clear = 0usize;
    let mut refused = 0usize;
    let mut decorative = 0usize;
    let mut outlined_paths = 0usize;
    let mut images = 0usize;
    let mut forms = 0usize;
    let mut annotations = 0usize;
    let mut why = std::collections::BTreeMap::<String, usize>::new();
    let mut kinds_of_image_block = std::collections::BTreeMap::<&str, usize>::new();
    let mut chars_recovered = 0usize;
    let mut recoverable_pages = 0usize;

    // One document for the whole survey. Each page's answer depends only on
    // that page, so redacting page 3 does not change what page 4 reports — and
    // reopening an eighty-megabyte file once per page turns a measurement into
    // an afternoon.
    let mut probe = PdfiumDocument::open_path(&path, None).expect("open");
    let sampled = pages.min(200);
    for index in 0..sampled {
        let Ok(page) = doc.page(index) else { continue };
        let Ok(verdict) = page.classify() else { continue };
        *kinds.entry(format!("{:?}", verdict.kind)).or_default() += 1;
        drop(page);

        let Ok(size) = doc.page_size(index) else { continue };
        // A rectangle over the middle of the page: where a name or a price sits,
        // and where a redaction is most likely to be drawn.
        let area = Rect {
            left: size.width_pt * 0.30,
            top: size.height_pt * 0.45,
            right: size.width_pt * 0.70,
            bottom: size.height_pt * 0.52,
        };

        // The preview, not the redaction: it reports everything the survey
        // found without the refusals in the way, which is what a measurement
        // needs to see.
        match probe.preview_redaction(
            &Redaction {
                require_complete: false,
                fill: None,
                ..Redaction::new(index, area)
            },
            None,
        ) {
            Err(_) => refused += 1,
            Ok(report) => {
                let mut image_share = 0.0f32;
                for item in &report.uncleared {
                    match item {
                        Uncleared::Path { .. } => decorative += 1,
                        Uncleared::OutlinedText { .. } => outlined_paths += 1,
                        Uncleared::Image { covers, .. } => {
                            images += 1;
                            image_share = image_share.max(*covers);
                        }
                        Uncleared::Form { .. } => forms += 1,
                        Uncleared::Annotation { .. } => annotations += 1,
                    }
                }
                let mut reasons: Vec<&str> = report
                    .blockers()
                    .iter()
                    .map(|b| match b {
                        Uncleared::Image { .. } => "image",
                        Uncleared::Form { .. } => "form",
                        Uncleared::Annotation { .. } => "annotation",
                        Uncleared::OutlinedText { .. } => "outlined type",
                        Uncleared::Path { .. } => "path",
                    })
                    .collect();
                reasons.sort_unstable();
                reasons.dedup();
                if !reasons.is_empty() {
                    *why.entry(reasons.join(" + ")).or_default() += 1;
                }
                if report.blockers().is_empty() {
                    clear += 1;
                } else {
                    blocked += 1;
                }

                // **The question that decides whether re-encoding is needed.**
                //
                // Two situations refuse identically and want opposite answers.
                // If text came out of the rectangle, the sensitive content was
                // text and the image beside it is a background or a photograph
                // — the user can look and say whether it matters. If nothing
                // came out and an image lies under the whole area, the words
                // *are* the picture and only re-encoding will do.
                let image_blocks = report
                    .blockers()
                    .iter()
                    .any(|b| matches!(b, Uncleared::Image { .. }));
                if image_blocks {
                    // **An acknowledgement answers for images and nothing else.**
                    // A page also blocked by outlined type stays blocked whatever
                    // the user says, so counting it here would overstate what the
                    // dialog buys.
                    let recoverable = report.only_images_are_in_the_way();
                    let scanned = report.uncleared.iter().any(
                        |u| matches!(u, Uncleared::Image { may_hold_text: true, .. }),
                    );
                    let bucket = if !recoverable {
                        "also blocked by outlined type  -> stays blocked"
                    } else if report.would_only_draw_a_mark() {
                        "would only draw a mark         -> refused, needs re-encoding"
                    } else if report.characters > 0 && scanned {
                        "text removed, SCANNED figure   -> ask, loudly"
                    } else if report.characters > 0 {
                        "text removed, image alongside  -> ask"
                    } else if image_share > 0.90 {
                        "no text, image covers the area -> re-encode"
                    } else {
                        "no text, image at the edge     -> ask"
                    };
                    *kinds_of_image_block.entry(bucket).or_default() += 1;
                    if recoverable && !report.would_only_draw_a_mark() {
                        recoverable_pages += 1;
                        chars_recovered += report.characters;
                    }
                }
            }
        }
    }

    println!("== page kinds (first {sampled}) ==");
    for (kind, count) in &kinds {
        println!("  {kind:<12} {count}");
    }
    println!("\n== a mid-page rectangle, per page ==");
    println!("  clears completely       {clear}");
    println!("  would be refused        {blocked}");
    println!("  refused outright        {refused}   (outlined or scanned page)");
    println!("\n== why a page would be refused ==");
    for (reason, count) in &why {
        println!("  {reason:<28} {count}");
    }
    if !kinds_of_image_block.is_empty() {
        println!("\n== of the pages an image blocks ==");
        for (kind, count) in &kinds_of_image_block {
            println!("  {kind:<38} {count}");
        }
        println!(
            "  -> an acknowledgement unblocks         {recoverable_pages} pages, {chars_recovered} characters"
        );
    }
    println!("\n== everything crossing that rectangle ==");
    println!("  images                  {images}");
    println!("  forms                   {forms}");
    println!("  annotations             {annotations}");
    println!("  decorative (reported)   {decorative}");
    println!("  outlined type (refuses) {outlined_paths}");
    if decorative + outlined_paths > 0 {
        let share = outlined_paths as f32 / (decorative + outlined_paths) as f32;
        println!("  outlined share          {:.1}%", share * 100.0);
    }

    // -- write one redacted file for an outside reader ------------------------
    let Some(needle) = needle else { return };
    for index in 0..pages {
        let Ok(page) = doc.page(index) else { continue };
        let Ok(chars) = page.characters() else { continue };
        let Some(at) = chars.text.find(&needle) else { continue };
        if !matches!(
            page.classify().map(|c| c.kind),
            Ok(PageTextKind::Native) | Ok(PageTextKind::Hybrid)
        ) {
            continue;
        }
        let start = chars.text[..at].chars().count();
        let mut area: Option<Rect> = None;
        for i in start..start + needle.chars().count() {
            let b = &chars.boxes[i * 4..i * 4 + 4];
            area = Some(match area {
                None => Rect { left: b[0], top: b[1], right: b[2], bottom: b[3] },
                Some(r) => Rect {
                    left: r.left.min(b[0]),
                    top: r.top.min(b[1]),
                    right: r.right.max(b[2]),
                    bottom: r.bottom.max(b[3]),
                },
            });
        }
        drop(page);
        let Some(area) = area else { continue };

        let mut probe = PdfiumDocument::open_path(&path, None).expect("open");
        match probe.redact(
            &Redaction { require_complete: false, ..Redaction::new(index, area) },
            None,
        ) {
            Ok(report) => {
                let out = std::env::var("PAGIFY_REDACT_OUT")
                    .unwrap_or_else(|_| "/tmp/redacted.pdf".into());
                let mut bytes = Vec::new();
                probe.save_full_copy(&mut bytes).expect("save");
                std::fs::write(&out, &bytes).expect("write");
                println!(
                    "\n== wrote {out} ==\n  page {} of {pages}, removed {:?} ({} chars, {} objects)",
                    index + 1,
                    needle,
                    report.characters,
                    report.objects
                );
                if !report.uncleared.is_empty() {
                    println!("  left behind: {}", report.uncleared.len());
                }
            }
            Err(e) => println!("\npage {} refused: {e}", index + 1),
        }
        return;
    }
    println!("\n{needle:?} not found on a redactable page");
}
