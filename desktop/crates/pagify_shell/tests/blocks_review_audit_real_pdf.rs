//! V4 REVIEW AUDIT (not part of the product's suite; every test here is #[ignore]d).
//!
//! What the real engine reports for the text objects that `block_input::adapt` calls `degenerate` (no ink area),
//! and what the paragraph editor would show for the line they sit in. Read-only: opens the datasheet, prints.
//!
//! cargo test -p pagify_shell --release --test blocks_review_audit_real_pdf -- --ignored --nocapture
//! (needs PAGIFY_PDFIUM_LIB and the datasheet on the Desktop; skips with a line when it is not there)

use pagify_shell::block_input::*;
use pagify_shell::Session;

const DATASHEET: &str = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";

#[test]
#[ignore = "V4 review audit: prints, asserts nothing about the product"]
fn v4_audit_area_less_objects_with_text_are_dropped_from_the_editor_line() {
    if std::env::var("V4_AUDIT_PDF").is_err() && !std::path::Path::new(DATASHEET).is_file() {
        eprintln!("skipped: the datasheet is not at {DATASHEET}");
        return;
    }
    let path = std::env::var("V4_AUDIT_PDF").unwrap_or_else(|_| DATASHEET.to_string());
    let session = Session::open(&path).expect("open the pdf");
    let pages = session.page_count().expect("page count").min(3);
    let mut total_non_blank = 0usize;
    for page in 0..pages {
        let snapshot = session.page_text_snapshot(page).expect("snapshot");
        let all = snapshot.runs.len();
        let pb = build_page_blocks(page, 1, 1, snapshot.clone());
        // every run the snapshot holds that has no area: what text does the engine give it?
        let mut blank = 0usize;
        let mut inked: Vec<(usize, String, [f32; 4])> = Vec::new();
        for r in &snapshot.runs {
            let (w, h) = ((r.rect.right - r.rect.left).abs(), (r.rect.bottom - r.rect.top).abs());
            if w > 0.5 && h > 0.5 {
                continue;
            }
            if r.text.trim().is_empty() {
                blank += 1;
            } else {
                inked.push((r.object, r.text.clone(), [r.rect.left, r.rect.top, r.rect.right, r.rect.bottom]));
            }
        }
        total_non_blank += inked.len();
        eprintln!("page {}: {all} text objects, {} excluded as degenerate; of the area-less ones {blank} have blank text and {} have REAL TEXT:", page + 1, pb.excluded.degenerate.len(), inked.len());
        for (o, text, r) in &inked {
            let in_degenerate = pb.excluded.degenerate.contains(o);
            // the member nearest to it, and the editor text of that member's line
            let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
            let mut best: Option<(f32, usize)> = None;
            for (&m, run) in &pb.runs {
                if !pb.by_object.contains_key(&m) {
                    continue;
                }
                let dx = (run.rect.left - cx).max(cx - run.rect.right).max(0.0);
                let dy = (run.rect.top - cy).max(cy - run.rect.bottom).max(0.0);
                let d = dx.hypot(dy);
                if best.map_or(true, |b| d < b.0) {
                    best = Some((d, m));
                }
            }
            let shown = best
                .map(|(d, m)| {
                    let (bi, li) = pb.by_object[&m];
                    let specs = editor_lines(&pb, bi);
                    let texts = line_texts(&pb, &specs);
                    format!("{d:.1} pt from member {m}; editor line {li} of block {bi}: {:?}; guard says {:?}", texts[li], check_editor_invariants(&pb, bi))
                })
                .unwrap_or_default();
            eprintln!("   object {o} text {text:?} rect [{:.1},{:.1},{:.1},{:.1}] ({:.2} x {:.2} pt) degenerate={in_degenerate}: {shown}", r[0], r[1], r[2], r[3], (r[2] - r[0]).abs(), (r[3] - r[1]).abs());
        }
    }
    eprintln!("total area-less objects with real text over the three pages: {total_non_blank}");
}

/// Dump every page of the PDF named by V4_AUDIT_PDF the way a click would see it: blocks, lines, the objects of each
/// line left to right (id, left edge, text) and the text the editor would show. For right-to-left and CJK probes.
#[test]
#[ignore = "V4 review audit: needs V4_AUDIT_PDF=<a pdf>, prints, asserts nothing about the product"]
fn v4_audit_dump_blocks_of_any_pdf() {
    let Ok(path) = std::env::var("V4_AUDIT_PDF") else {
        eprintln!("skipped: set V4_AUDIT_PDF");
        return;
    };
    let session = Session::open(&path).expect("open the pdf");
    let pages = session.page_count().expect("page count");
    for page in 0..pages.min(4) {
        let snapshot = session.page_text_snapshot(page).expect("snapshot");
        let n_runs = snapshot.runs.len();
        if let Some((a, b)) = std::env::var("V4_AUDIT_IDS").ok().and_then(|v| v.split_once('-').map(|(a, b)| (a.parse::<usize>().unwrap_or(0), b.parse::<usize>().unwrap_or(0)))) {
            for r in snapshot.runs.iter().filter(|r| r.object >= a && r.object <= b) {
                eprintln!("OBJ {} {:?} rect [{:.1},{:.1},{:.1},{:.1}] origin ({:.1},{:.1}) size {:.2}", r.object, r.text, r.rect.left, r.rect.top, r.rect.right, r.rect.bottom, r.origin.x, r.origin.y, r.size);
            }
        }
        let pb = build_page_blocks(page, 1, 1, snapshot);
        eprintln!("=== page {}: {n_runs} text objects, admitted {}, blocks {}, excluded blank {} invisible {} degenerate {} shadowed {} rotated {}", page + 1, pb.frags.len(), pb.blocks.len(), pb.excluded.blank.len(), pb.excluded.invisible.len(), pb.excluded.degenerate.len(), pb.excluded.shadowed.len(), pb.excluded.rotated.len());
        for (bi, b) in pb.blocks.iter().enumerate() {
            let specs = editor_lines(&pb, bi);
            let texts = line_texts(&pb, &specs);
            let guard = check_editor_invariants(&pb, bi);
            eprintln!("block {bi} starts {:?}: {} lines, rect [{:.0},{:.0},{:.0},{:.0}], guard {:?}", b.starts_because, b.lines.len(), b.left, b.top, b.right, b.bottom, guard);
            if std::env::var("V4_AUDIT_EDGES").ok().and_then(|v| v.parse::<usize>().ok()) == Some(bi) {
                for (li, s) in specs.iter().enumerate() {
                    let edges: Vec<String> = s.objects.iter().map(|o| format!("{o}[{:.1},{:.1}]{:?}", pb.runs[o].rect.left, pb.runs[o].rect.right, pb.runs[o].text)).collect();
                    eprintln!("EDGES block {bi} line {li}: {}", edges.join(" "));
                }
            }
            for (li, s) in specs.iter().enumerate() {
                let parts: Vec<String> = s.objects.iter().map(|o| format!("{o}@{:.0}:{:?}", pb.runs[o].rect.left, pb.runs[o].text)).collect();
                if std::env::var("V4_AUDIT_COMPACT").is_ok() {
                    eprintln!("   line {li} [{}] {:?}", if s.frozen { "frozen" } else { "" }, texts[li]);
                } else {
                    eprintln!("   line {li} [{}] text {:?}  objects {}", if s.frozen { "frozen" } else { "" }, texts[li], parts.join(" "));
                }
            }
        }
    }
}
