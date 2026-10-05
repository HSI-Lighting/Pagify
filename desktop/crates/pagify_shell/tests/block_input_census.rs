//! A census of the editor guard over real documents: a tool, not a check (ignored by default).
//!
//! For every page of every PDF named in `PAGIFY_CENSUS_PDFS` (paths separated by `;`), the page is read
//! the way a click reads it (`Session::page_text_snapshot` + `build_page_blocks`), and one JSON line per
//! block with more than one text object (the blocks a click can open as a paragraph) says what
//! `check_editor_invariants` made of it. `PAGIFY_CENSUS_OUT` is the file the lines go to (default:
//! standard output); `PAGIFY_CENSUS_PAGES` limits the pages read per document.
//!
//! `cargo test -p pagify_shell --release --test block_input_census -- --ignored --nocapture`
//!
//! The summary at the end counts, per document, the blocks, those with more than one text object, how many
//! the guard refused and for which invariant, and how many text objects carry thin glyphs, twins and
//! right-to-left characters. It reads documents, never changes them.

use pagify_shell::block_input::*;
use pagify_shell::Session;
use std::collections::BTreeMap;
use std::io::Write;

#[test]
#[ignore = "a census over the documents named in PAGIFY_CENSUS_PDFS: run with --ignored --nocapture"]
fn block_input_census() {
    let Ok(list) = std::env::var("PAGIFY_CENSUS_PDFS") else {
        eprintln!("census: set PAGIFY_CENSUS_PDFS to the PDFs to read, separated by ';'");
        return;
    };
    let limit: usize = std::env::var("PAGIFY_CENSUS_PAGES").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let mut out: Box<dyn Write> = match std::env::var("PAGIFY_CENSUS_OUT") {
        Ok(path) => Box::new(std::fs::File::create(path).expect("the census output file")),
        Err(_) => Box::new(std::io::stdout()),
    };
    for path in list.split(';').filter(|p| !p.trim().is_empty()) {
        let name = std::path::Path::new(path).file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        let session = match Session::open(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("census: {name}: cannot open ({e})");
                continue;
            }
        };
        let pages = session.page_count().unwrap_or(0).min(limit);
        let (mut blocks, mut multi, mut refused, mut thin, mut twins, mut rtl_blocks) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
        let mut by_reason: BTreeMap<String, usize> = BTreeMap::new();
        for page in 0..pages {
            let Ok(snapshot) = session.page_text_snapshot(page) else {
                eprintln!("census: {name} page {}: the page's text cannot be read", page + 1);
                continue;
            };
            let pb = build_page_blocks(page, 1, 1, snapshot);
            thin += pb
                .by_object
                .keys()
                .filter(|o| pb.runs.get(o).is_some_and(|r| (r.rect.right - r.rect.left).abs() <= 0.5 || (r.rect.bottom - r.rect.top).abs() <= 0.5))
                .count();
            twins += pb.twins.values().map(Vec::len).sum::<usize>();
            for (bi, b) in pb.blocks.iter().enumerate() {
                blocks += 1;
                let objects = b.objects();
                if objects.len() < 2 {
                    continue;
                }
                multi += 1;
                let verdict = check_editor_invariants(&pb, bi);
                if let Err(e) = &verdict {
                    refused += 1;
                    // the invariant and the start of the message, up to its first number
                    let (invariant, rest) = e.split_once(": ").unwrap_or((e.as_str(), ""));
                    let shape = rest.split(|c: char| c.is_ascii_digit()).next().unwrap_or("").trim();
                    *by_reason.entry(format!("{invariant}: {shape}")).or_default() += 1;
                    if e.starts_with("C15:") {
                        rtl_blocks += 1;
                    }
                }
                let lines: Vec<String> = b
                    .lines
                    .iter()
                    .map(|l| format!("{{\"objs\":{:?},\"outl\":{}}}", l.objects, l.outlined.len()))
                    .collect();
                let guard = match &verdict {
                    Ok(()) => "null".to_string(),
                    Err(e) => format!("{e:?}"),
                };
                let _ = writeln!(out, "{{\"doc\":{name:?},\"page\":{},\"block\":{bi},\"first\":{},\"n\":{},\"lines\":[{}],\"guard\":{guard}}}", page + 1, objects[0], objects.len(), lines.join(","));
            }
        }
        eprintln!("census {name}: {pages} pages, {blocks} blocks, {multi} with 2+ text objects, {refused} refused {by_reason:?}; {thin} thin-glyph members, {twins} twins, {rtl_blocks} right-to-left blocks");
    }
}
