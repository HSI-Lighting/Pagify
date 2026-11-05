//! hayro against PDFium, page by page, on a folder of PDFs: before a page
//! picture is drawn by hayro, the owner sees on their own files where the two
//! renderers differ and whether `Preview::differs_because` sends every such
//! page to PDFium.
//!
//! `cargo run --release -p pagify_shell --example compare_renderers -- <folder> <out>`
//! with `PAGIFY_PDFIUM_LIB` set. Every `*.pdf` under `<folder>`, at 1.5 pixels a
//! point (the longer side at most 4096). Writes `<out>/report.md`, and both
//! pictures of every page that differs to `<out>/pages/`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pagify_shell::preview::{compare, Comparison, Difference, Preview, DIFF_MEAN, INK_MOVED, INK_RATIO};
use pagify_shell::{PageRaster, Session};

struct Row {
    file: String,
    /// From 1, as the report shows it.
    page: usize,
    hayro_ms: f64,
    pdfium_ms: f64,
    /// `None` when either drew nothing, or the two are not the same size.
    comparison: Option<Comparison>,
    reason: Option<Difference>,
    /// What went wrong, when the two pictures could not be compared.
    note: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Verdict {
    /// Differs, and the rule lets hayro draw it: must be none.
    Missed,
    /// Differs, and the rule sends it to PDFium.
    Caught,
    /// The rule sends it to PDFium, though the two drew it alike: speed lost.
    OverCautious,
    /// hayro drew nothing, so PDFium draws it; or PDFium failed.
    NotCompared,
    Fine,
}

impl Row {
    fn verdict(&self) -> Verdict {
        let differs = match (self.note, self.comparison) {
            ("", Some(c)) => c.differs(),
            ("sizes differ", _) => true,
            _ => return Verdict::NotCompared,
        };
        match (differs, self.reason) {
            (true, None) => Verdict::Missed,
            (true, Some(_)) => Verdict::Caught,
            (false, Some(_)) => Verdict::OverCautious,
            (false, None) => Verdict::Fine,
        }
    }
}

fn pdfs(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            pdfs(&path, found);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            found.push(path);
        }
    }
}

fn save(raster: &PageRaster, path: &Path) {
    let image = image::RgbaImage::from_raw(raster.width, raster.height, raster.pixels.clone()).expect("RGBA");
    if let Err(e) = image.save(path) {
        eprintln!("could not save {}: {e}", path.display());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [folder, out] = &args[..] else {
        eprintln!("usage: compare_renderers <folder> <out>");
        std::process::exit(2);
    };
    let (folder, out) = (Path::new(folder), Path::new(out));
    std::fs::create_dir_all(out.join("pages")).expect("the output folder");
    let mut files = Vec::new();
    pdfs(folder, &mut files);

    let (mut rows, mut skipped, mut file_lines) = (Vec::new(), Vec::new(), Vec::new());
    for path in &files {
        let name = path.strip_prefix(folder).unwrap_or(path).display().to_string();
        let Ok(session) = Session::open(path) else {
            skipped.push(format!("{name}: PDFium cannot open it"));
            continue;
        };
        let Some(preview) = session.current_bytes().ok().and_then(Preview::new) else {
            skipped.push(format!("{name}: hayro cannot read it"));
            continue;
        };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let pages = session.page_count().unwrap_or(0);
        let first = rows.len();
        for page in 0..pages {
            let Ok(size) = session.page_size(page) else { continue };
            let scale = 1.5f32.min(4096.0 / size.width_pt.max(size.height_pt));
            let width = (size.width_pt * scale).round().max(1.0) as u32;
            let started = Instant::now();
            let ours = preview.render(page, width);
            let hayro_ms = started.elapsed().as_secs_f64() * 1000.0;
            let started = Instant::now();
            let theirs = session.render_page(page, width as f32 / size.width_pt).ok();
            let pdfium_ms = started.elapsed().as_secs_f64() * 1000.0;
            let (comparison, note) = match (&ours, &theirs) {
                (None, _) => (None, "hayro drew nothing"),
                (_, None) => (None, "PDFium failed"),
                (Some(a), Some(b)) => match compare(a, b) {
                    Some(c) => (Some(c), ""),
                    None => (None, "sizes differ"),
                },
            };
            let row = Row { file: name.clone(), page: page + 1, hayro_ms, pdfium_ms, comparison, reason: preview.differs_because(page), note };
            if matches!(row.verdict(), Verdict::Missed | Verdict::Caught) {
                for (raster, by) in [(&ours, "hayro"), (&theirs, "pdfium")] {
                    if let Some(r) = raster {
                        save(r, &out.join("pages").join(format!("{stem}-p{}-{by}.png", page + 1)));
                    }
                }
            }
            eprintln!("{name} p{}: hayro {hayro_ms:.0} ms, PDFium {pdfium_ms:.0} ms", page + 1);
            rows.push(row);
        }
        let (h, p): (f64, f64) = rows[first..].iter().map(|r| (r.hayro_ms, r.pdfium_ms)).fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
        file_lines.push(format!(
            "| {name} | {pages} | {} | {:.2} | {:.2} |",
            if preview.has_layers() { "yes" } else { "no" },
            h / 1000.0,
            p / 1000.0
        ));
    }

    rows.sort_by(|a, b| {
        let moved = |r: &Row| r.comparison.map_or(f64::INFINITY, |c| c.ink_moved);
        a.verdict().cmp(&b.verdict()).then(moved(b).total_cmp(&moved(a)))
    });
    let count = |v: Verdict| rows.iter().filter(|r| r.verdict() == v).count();
    let (h, p) = rows.iter().fold((0.0, 0.0), |a, r| (a.0 + r.hayro_ms, a.1 + r.pdfium_ms));
    let mut report = String::new();
    let _ = writeln!(report, "# hayro against PDFium — {}\n", folder.display());
    let _ = writeln!(
        report,
        "{} files, {} pages, drawn at 1.5 px a point (the longer side at most 4096 px). A page differs when the mean \
         difference a channel is over {DIFF_MEAN}, hayro's ink over PDFium's is outside {:?}, or over {INK_MOVED} of \
         the ink has none of the other picture's within a pixel.\n",
        files.len(),
        rows.len(),
        INK_RATIO
    );
    let _ = writeln!(report, "| | pages |\n|---|---|");
    let _ = writeln!(report, "| **missed** — differs, and the rule lets hayro draw it (must be 0) | {} |", count(Verdict::Missed));
    let _ = writeln!(report, "| caught — differs, and the rule sends it to PDFium | {} |", count(Verdict::Caught));
    let _ = writeln!(report, "| over-cautious — sent to PDFium, though drawn alike (speed lost) | {} |", count(Verdict::OverCautious));
    let _ = writeln!(report, "| fine — drawn alike, hayro draws it | {} |", count(Verdict::Fine));
    let _ = writeln!(report, "| not compared — hayro drew nothing (PDFium draws it) or PDFium failed | {} |", count(Verdict::NotCompared));
    let _ = writeln!(report, "\nhayro {:.1} s, PDFium {:.1} s, every page drawn once by each.\n", h / 1000.0, p / 1000.0);
    let _ = writeln!(report, "| file | pages | PDF layers | hayro s | PDFium s |\n|---|---|---|---|---|");
    for line in &file_lines {
        let _ = writeln!(report, "{line}");
    }
    if !skipped.is_empty() {
        let _ = writeln!(report, "\nSkipped:\n");
        for line in &skipped {
            let _ = writeln!(report, "- {line}");
        }
    }

    let _ = writeln!(report, "\n## Pages, worst first\n");
    let _ = writeln!(report, "| file | page | hayro ms | PDFium ms | mean | ink ratio | moved | differs | reason |");
    let _ = writeln!(report, "|---|---|---|---|---|---|---|---|---|");
    for r in &rows {
        let (mean, ratio, moved) = r.comparison.map_or((r.note.to_string(), String::new(), String::new()), |c| {
            (format!("{:.2}", c.mean_diff), format!("{:.2}", c.ink_ratio), format!("{:.3}", c.ink_moved))
        });
        let verdict = match r.verdict() {
            Verdict::Missed => "**MISSED**",
            Verdict::Caught => "caught",
            Verdict::OverCautious => "no",
            Verdict::NotCompared => "—",
            Verdict::Fine => "no",
        };
        let _ = writeln!(
            report,
            "| {} | {} | {:.0} | {:.0} | {mean} | {ratio} | {moved} | {verdict} | {} |",
            r.file,
            r.page,
            r.hayro_ms,
            r.pdfium_ms,
            r.reason.map_or("", |d| d.reason())
        );
    }
    std::fs::write(out.join("report.md"), report).expect("the report");
    eprintln!(
        "missed {}, caught {}, over-cautious {}, fine {}, not compared {} — {}",
        count(Verdict::Missed),
        count(Verdict::Caught),
        count(Verdict::OverCautious),
        count(Verdict::Fine),
        count(Verdict::NotCompared),
        out.join("report.md").display()
    );
}
