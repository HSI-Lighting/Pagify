//! Replacing the pieces of a line, on every block of the real datasheet.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --release --test replace_lines_sweep -- --nocapture
//! ```
//!
//! `--nocapture` is the point of the flag, not a convenience: a test here that
//! cannot find PDFium or the real datasheet prints why and returns, and still
//! counts as passed. Look for the counts it prints.
//!
//! **What it asks.** Blocks are found exactly as the app finds them
//! ([`Session::page_text_snapshot`], [`build_page_blocks`], [`editor_lines`]). In
//! every block that has more than one text object, the first plain — not frozen —
//! line has one word changed and is typed onto its first piece, its other pieces
//! removed, through `Command::ReplaceTextLines`: what the app does when somebody
//! corrects a word. Each is made to a **fresh copy** of the datasheet, so that no
//! attempt can be helped or hurt by another, in a session that has the two fonts
//! the app types with ([`typing_fonts`]).
//!
//! **What it holds each to.** The batch applied; the first piece reads what was
//! typed; every removed object is gone; every other text object of the page is
//! where it was, with the same words, colour, box and size (at its new number,
//! since the removed objects no longer count); and undo puts every object back
//! exactly as it was.
//!
//! **Two passes.** The first removes exactly the objects the editor lists with the
//! line. A line has more on the page than that: a lone `l` or `.` under half a
//! point wide is no object the detector keeps (it has no area), and a faux-bold
//! word is drawn twice, the second copy a twin the detector leaves out. Taking the
//! listed pieces off and leaving those either moves a piece that is seen (the
//! engine refuses: "draws something has nothing repositioning it") or leaves a
//! twin showing where the line was. The second pass is a corrected app: it lists
//! [`hidden_pieces`] as well, and is held to the same checks. **Every attempt the
//! first pass does not get through must be explained by what the second pass
//! lists, or by a named limit of the text writer, and the second pass must then
//! succeed**; anything else fails the test.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use pagify_shell::block_input::{build_page_blocks, check_editor_invariants, editor_lines, line_texts, PageBlocks, SAME_ORIGIN_PT, TWIN_RECT_PT};
use pagify_shell::Session;
use pdf_core::command::Command;
use pdf_core::document::{TextLineEdit, TextRun, TextStyle};

/// The datasheet this was built against: three A4 pages from Illustrator. Not in
/// the repository (42 MB), so a machine without it skips.
const DATASHEET: &str = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";

/// What the writer cannot do, whoever lists what. Not the removal's: the same
/// refusal comes from retyping that one piece alone (`set_text_run_styled`).
///
/// A piece whose font has a code with no Unicode spelling — the degree sign of the
/// polar diagram's labels — cannot have its codes lined up with the text PDFium
/// read from it, and the writer does not guess which codes to replace.
const WRITER_LIMITS: &[&str] = &["that run's codes cannot be lined up with its text"];

fn have_pdfium() -> bool {
    if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
        eprintln!("skipped: set PAGIFY_PDFIUM_LIB to a desktop PDFium to run this");
        return false;
    }
    true
}

/// The fonts the app types with: its two bundled ones (`BUNDLED_OUTLINED_FONTS` in
/// `pagify_app`), which it gives every session it opens.
fn typing_fonts() -> Vec<Vec<u8>> {
    ["Montserrat-Regular.ttf", "Montserrat-Bold.ttf"]
        .iter()
        .map(|name| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/fonts").join(name);
            std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        })
        .collect()
}

/// The document swept: the datasheet, or the file `PAGIFY_SWEEP_PDF` names (the quotation and datasheet copies
/// the paragraph release check runs this on).
fn source() -> String {
    std::env::var("PAGIFY_SWEEP_PDF").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| DATASHEET.to_string())
}

/// A copy of the datasheet, removed when dropped.
struct Copy(PathBuf);

impl Copy {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("pagify-sweep-{}-{tag}.pdf", std::process::id()));
        std::fs::copy(source(), &path).expect("copy the datasheet");
        Copy(path)
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Words as PDFium reads them back: its hyphen marker as a hyphen, and every run
/// of spaces as one, which is what it makes of a line that was typed with two.
fn clean(words: &str) -> String {
    words.replace('\u{2}', "-").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// An object that draws nothing: a space (PDFium gives it a box with no height, or
/// none at all), or text drawn fully transparent. The engine's own rule, so that
/// what this skips is what the engine lets move. Not an object with *little* area:
/// a lone `l` is 0.4 pt wide and a `.` 0.46 pt tall, and both are there to be seen.
fn draws_nothing(r: &TextRun) -> bool {
    r.color.a == 0 || (r.rect.right - r.rect.left).abs() < 0.01 || (r.rect.bottom - r.rect.top).abs() < 0.01
}

/// A line's own text with one word changed: its longest word, spelt backwards.
/// Made of the same letters, so the first piece's font can spell it wherever it
/// could spell the line.
fn one_word_changed(line: &str) -> String {
    let mut words: Vec<String> = line.split(' ').map(str::to_string).collect();
    if let Some(at) = words
        .iter()
        .enumerate()
        .filter(|(_, w)| w.chars().count() >= 3)
        .max_by_key(|(i, w)| (w.chars().count(), usize::MAX - i))
        .map(|(i, _)| i)
    {
        let reversed: String = words[at].chars().rev().collect();
        if reversed != words[at] {
            words[at] = reversed;
        }
    }
    words.join(" ")
}

fn renumbered(object: usize, removed: &[usize]) -> usize {
    object - removed.iter().filter(|r| **r < object).count()
}

/// What a line has on the page that the editor does not list with it, and why it
/// must come off with the line. **What a corrected app lists besides the line's
/// own objects.**
///
/// * `"twin"` — a text object drawn on top of (or under) one of the line's pieces:
///   an origin within [`SAME_ORIGIN_PT`] and a box within [`TWIN_RECT_PT`] on every
///   edge, which the detector leaves out as `blank` (PDFium reads the covered copy
///   as empty) or `shadowed`. A faux-bold word is two of these. Left behind when
///   the piece it shadows goes, it shows.
/// * `"thin glyph"` — an object with no area for the detector (`degenerate`: a side
///   of half a point or less) that nonetheless has ink, on the line's baseline
///   between its first piece's left edge and its last piece's right edge, with half
///   an em to spare for a full stop that ends the line. Left behind, it is carried
///   by the pen from the piece before it and moves when that piece goes.
///
/// Objects that draw nothing are not listed: a space does not move anything seen.
/// Members of another line are not either, whatever their place.
fn hidden_pieces(pb: &PageBlocks, runs: &[TextRun], listed: &[usize]) -> Vec<(usize, &'static str)> {
    let members: Vec<&TextRun> = runs.iter().filter(|r| listed.contains(&r.object)).collect();
    let Some(head) = members.iter().find(|r| Some(&r.object) == listed.first()) else { return Vec::new() };
    let left = members.iter().map(|m| m.rect.left.min(m.rect.right)).fold(f32::MAX, f32::min);
    let right = members.iter().map(|m| m.rect.left.max(m.rect.right)).fold(f32::MIN, f32::max);
    let slack = head.size * 0.5;
    let mut found = Vec::new();
    for r in runs {
        if listed.contains(&r.object) || draws_nothing(r) {
            continue;
        }
        let Some(why) = pb.excluded.reason(r.object) else { continue };
        if !matches!(why, "blank" | "shadowed" | "degenerate") {
            continue;
        }
        let twin = members.iter().any(|m| {
            (m.origin.x - r.origin.x).hypot(m.origin.y - r.origin.y) <= SAME_ORIGIN_PT
                && [m.rect.left - r.rect.left, m.rect.top - r.rect.top, m.rect.right - r.rect.right, m.rect.bottom - r.rect.bottom]
                    .iter()
                    .all(|gap| gap.abs() <= TWIN_RECT_PT)
        });
        if twin {
            found.push((r.object, "twin"));
        } else if why == "degenerate"
            && (r.origin.y - head.origin.y).abs() <= 0.5
            && r.origin.x >= left - slack
            && r.origin.x <= right + slack
        {
            found.push((r.object, "thin glyph"));
        }
    }
    found
}

/// How one attempt ended.
#[derive(Clone, Debug)]
enum Verdict {
    /// Applied, read back as typed, nothing else moved, undone exactly.
    Ok,
    /// The batch was refused, with this message.
    Refused(String),
    /// Applied, and something is wrong with the page it left (or with undo).
    Damaged(String),
}

struct Attempt {
    /// What was finally typed: the line's own words with one changed, or — when
    /// the first piece's font could not spell them — the first piece's own.
    typed: String,
    retried_for_font: bool,
    verdict: Verdict,
}

/// One attempt, on a fresh copy: the batch, and everything it is held to.
/// `SWEEP_ONLY` (see the test) turns `debugging` on.
fn attempt(tag: &str, page: usize, first: usize, remove: &[usize], text: &str, debugging: bool) -> Attempt {
    let copy = Copy::new(tag);
    let session = Session::open(&copy.0).expect("open the copy");
    session.set_typing_fonts(typing_fonts()).expect("typing fonts");
    let before = session.page_text_snapshot(page).expect("snapshot").runs;

    let run = |typed: &str| {
        session.execute(Command::ReplaceTextLines {
            page_index: page,
            edits: vec![TextLineEdit::Retype {
                first,
                text: typed.to_string(),
                style: TextStyle::default(),
                remove: remove.to_vec(),
                justify_to: None,
            }],
        })
    };
    let mut typed = text.to_string();
    // SWEEP_TEXT (with SWEEP_ONLY) types this instead, to tell a writer fault from a reader one.
    if debugging {
        if let Ok(t) = std::env::var("SWEEP_TEXT") {
            typed = t;
        }
    }
    let mut outcome = run(&typed);
    let mut retried_for_font = false;
    // The line's own words may include a letter that its first piece's font
    // cannot spell and that no font the app types with has: that is the font's,
    // not the engine's, and a refusal is atomic, so the first piece's own words
    // are typed instead, which that font spells by construction.
    if let Err(error) = &outcome {
        if error.to_string().contains("is not in this text's font") {
            let own = before.iter().find(|r| r.object == first).map(|r| clean(&r.text)).unwrap_or_default();
            typed = one_word_changed(&own);
            retried_for_font = true;
            outcome = run(&typed);
        }
    }
    if let Err(error) = outcome {
        if debugging {
            println!("first {first}, remove {remove:?}, typed {typed:?}: REFUSED: {error}");
            let (lo, hi) = (first.saturating_sub(1), remove.iter().copied().max().unwrap_or(first) + 3);
            for b in before.iter().filter(|b| (lo..=hi).contains(&b.object)) {
                let listed = if b.object == first || remove.contains(&b.object) { "listed " } else { "UNLISTED" };
                println!(
                    "  {listed} {:>5} {:?} origin ({:.2}, {:.2}) box {:.3}..{:.3} x {:.3}..{:.3} size {} colour {:?}",
                    b.object, b.text, b.origin.x, b.origin.y, b.rect.left, b.rect.right, b.rect.top, b.rect.bottom, b.size, b.color
                );
            }
        }
        return Attempt { typed, retried_for_font, verdict: Verdict::Refused(error.to_string()) };
    }

    // What it left.
    let after = session.page_text_snapshot(page).expect("snapshot").runs;
    let mut problems: Vec<String> = Vec::new();
    if after.len() != before.len() - remove.len() {
        problems.push(format!("{} text objects after, {} expected", after.len(), before.len() - remove.len()));
    }
    match after.iter().find(|r| r.object == renumbered(first, remove)) {
        Some(a) if clean(&a.text) == clean(&typed) => {}
        Some(a) => problems.push(format!("the first piece reads {:?}, not {:?}", clean(&a.text), clean(&typed))),
        None => problems.push("the first piece is gone".into()),
    }
    // A removed piece that is seen is no longer on the page.
    for b in before.iter().filter(|b| remove.contains(&b.object) && !draws_nothing(b)) {
        if let Some(a) = after.iter().find(|a| a.rect == b.rect && clean(&a.text) == clean(&b.text)) {
            problems.push(format!("removed object {} ({:?}) is still on the page as object {}", b.object, clean(&b.text), a.object));
            break;
        }
    }
    // Every other piece that is seen is where it was. What draws nothing may
    // have moved with the pen, and reads as it reads.
    for b in before.iter().filter(|b| b.object != first && !remove.contains(&b.object) && !draws_nothing(b)) {
        match after.iter().find(|r| r.object == renumbered(b.object, remove)) {
            Some(a) if clean(&a.text) == clean(&b.text) && a.rect == b.rect && a.origin == b.origin && a.size == b.size && a.color == b.color => {}
            Some(a) => {
                problems.push(format!("object {} changed: {:?} {:?} -> {:?} {:?}", b.object, b.text, b.origin, a.text, a.origin));
                break;
            }
            None => {
                problems.push(format!("object {} is gone", b.object));
                break;
            }
        }
    }

    if debugging {
        let (lo, hi) = (first.saturating_sub(3), remove.iter().copied().max().unwrap_or(first) + 6);
        println!("first {first}, remove {remove:?}, typed {typed:?}");
        for b in before.iter().filter(|b| (lo..=hi).contains(&b.object)) {
            println!("  before {:>5} {:?} origin ({:.2}, {:.2}) box {:.2}..{:.2}", b.object, b.text, b.origin.x, b.origin.y, b.rect.left, b.rect.right);
        }
        for a in after.iter().filter(|a| (renumbered(lo, remove)..=renumbered(hi, remove)).contains(&a.object)) {
            println!("  after  {:>5} {:?} origin ({:.2}, {:.2}) box {:.2}..{:.2}", a.object, a.text, a.origin.x, a.origin.y, a.rect.left, a.rect.right);
        }
    }

    // Undo.
    match session.undo() {
        Ok((true, _)) => {
            let undone = session.page_text_snapshot(page).expect("snapshot").runs;
            if undone != before {
                problems.push("after undo the objects are not exactly what they were".into());
            }
        }
        other => problems.push(format!("undo did not undo: {:?}", other.map(|(done, _)| done))),
    }

    let verdict = if problems.is_empty() { Verdict::Ok } else { Verdict::Damaged(problems.join("; ")) };
    Attempt { typed, retried_for_font, verdict }
}

/// One block's attempt.
struct Job {
    page: usize,
    block: usize,
    first: usize,
    /// The rest of the line, as the editor lists it.
    rest: Vec<usize>,
    /// What the line has besides: see [`hidden_pieces`].
    hidden: Vec<(usize, &'static str)>,
    text: String,
}

/// What became of the attempts on one page.
#[derive(Default)]
struct Tally {
    blocks_with_more_than_one_object: usize,
    /// Of those, the ones the editor's guard refuses (not swept: see the loop).
    guard_refused: usize,
    no_plain_line: usize,
    attempted: usize,
    /// Lines with something the editor does not list, whether or not the first
    /// pass got through.
    with_hidden_pieces: usize,
    ok: usize,
    /// Lines typed again as their first piece's own words, for its font.
    font_retries: usize,
    refused: Vec<(usize, usize, String)>, // (block, first object, message)
    damaged: Vec<(usize, usize, String)>,
    /// Second pass.
    second_ok: usize,
    /// Second pass, refused for a limit of the writer: named, and not the removal's.
    writer_limit: Vec<(usize, usize, String)>,
    /// Anything not explained, either pass.
    unexplained: Vec<(usize, usize, String)>,
}

/// **Every multi-piece block of the datasheet takes a retyped line.**
#[test]
fn every_block_with_more_than_one_object_takes_a_retyped_line_and_undoes_cleanly() {
    if !have_pdfium() {
        return;
    }
    let pdf = source();
    if !std::path::Path::new(&pdf).exists() {
        eprintln!("skipped: the real datasheet is not at {pdf}");
        return;
    }
    println!("sweeping {pdf}");

    let started = Instant::now();
    let reader = Session::open(&pdf).expect("open the datasheet");
    let pages = reader.page_count().expect("page count");
    let (mut tallies, jobs) = sweep_collect_jobs(&reader, pages);
    drop(reader);
    println!(
        "{} blocks with more than one text object; {} attempts to make",
        jobs.len() + tallies.values().map(|t| t.no_plain_line).sum::<usize>(),
        jobs.len()
    );

    // SWEEP_ONLY=page:block runs just that block (pages from 1) and says more.
    let only: Option<(usize, usize)> = std::env::var("SWEEP_ONLY").ok().and_then(|v| {
        let (p, b) = v.split_once(':')?;
        Some((p.parse::<usize>().ok()? - 1, b.parse().ok()?))
    });
    let debugging = only.is_some();

    for (n, job) in jobs.iter().enumerate() {
        if only.is_some_and(|o| o != (job.page, job.block)) {
            continue;
        }
        let tally = tallies.get_mut(&job.page).expect("tally");
        tally.attempted += 1;
        if !job.hidden.is_empty() {
            tally.with_hidden_pieces += 1;
        }

        // The first pass: what the editor lists.
        let one = attempt(&format!("{n}a"), job.page, job.first, &job.rest, &job.text, debugging);
        tally.font_retries += usize::from(one.retried_for_font);
        let why = match &one.verdict {
            Verdict::Ok => {
                tally.ok += 1;
                continue;
            }
            Verdict::Refused(message) => {
                tally.refused.push((job.block, job.first, message.clone()));
                message.clone()
            }
            Verdict::Damaged(problems) => {
                tally.damaged.push((job.block, job.first, problems.clone()));
                problems.clone()
            }
        };

        // The second pass: what a corrected app lists. Only for what the first
        // pass did not get through.
        let mut removing = job.rest.clone();
        removing.extend(job.hidden.iter().map(|(o, _)| *o));
        let two = attempt(&format!("{n}b"), job.page, job.first, &removing, &job.text, debugging);
        let kinds: Vec<&str> = job.hidden.iter().map(|(_, k)| *k).collect();
        match (&one.verdict, &two.verdict) {
            // Whatever it did, a second pass that is damaged is not explained.
            (_, Verdict::Damaged(problems)) => {
                tally.unexplained.push((job.block, job.first, format!("second pass damaged the page: {problems}")));
            }
            (_, Verdict::Refused(message)) if WRITER_LIMITS.iter().any(|limit| message.contains(limit)) => {
                tally.writer_limit.push((job.block, job.first, message.clone()));
            }
            (_, Verdict::Refused(message)) => {
                tally.unexplained.push((job.block, job.first, format!("first pass: {why}; second pass: {message}")));
            }
            (Verdict::Refused(message), Verdict::Ok) if message.contains("has nothing repositioning it") && !job.hidden.is_empty() => {
                tally.second_ok += 1;
            }
            // A twin left behind shows where the line was: explained by a twin.
            (Verdict::Damaged(problems), Verdict::Ok) if problems.contains("is still on the page") && kinds.contains(&"twin") => {
                tally.second_ok += 1;
            }
            (_, Verdict::Ok) => {
                tally.unexplained.push((job.block, job.first, format!("first pass: {why}; nothing hidden explains it (hidden: {:?})", job.hidden)));
            }
        }
    }

    println!("---------------------------------------------------------------------------");
    println!("FIRST PASS: the objects the editor lists with the line");
    let (mut attempted, mut ok, mut refused, mut damaged) = (0, 0, 0, 0);
    for (page, t) in &tallies {
        println!(
            "page {}: {} blocks with more than one object ({} refused by the guard, not swept), {} without a plain line; attempted {}, ok {}, refused {}, damaged {} ({} typed again for their font; {} lines have pieces the editor does not list)",
            page + 1,
            t.blocks_with_more_than_one_object,
            t.guard_refused,
            t.no_plain_line,
            t.attempted,
            t.ok,
            t.refused.len(),
            t.damaged.len(),
            t.font_retries,
            t.with_hidden_pieces
        );
        attempted += t.attempted;
        ok += t.ok;
        refused += t.refused.len();
        damaged += t.damaged.len();
    }
    println!("all pages: attempted {attempted}, ok {ok}, refused {refused}, damaged {damaged}, in {:.0} s", started.elapsed().as_secs_f64());
    let mut reasons: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (page, t) in &tallies {
        for (block, _first, message) in &t.refused {
            // The object named in the message is a number; group by the sentence around it.
            let sentence: String = message.chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect();
            reasons.entry(sentence).or_default().push((page + 1, *block));
        }
    }
    for (message, at) in &reasons {
        println!("refused {}x: {message}  (page, block): {:?}", at.len(), &at[..at.len().min(12)]);
    }
    for (page, t) in &tallies {
        for (block, first, message) in &t.damaged {
            println!("DAMAGED page {} block {block} (first object {first}): {message}", page + 1);
        }
    }

    println!("SECOND PASS: the same, with what the editor does not list (hidden_pieces) taken off too");
    let (mut second_ok, mut limits, mut unexplained) = (0, 0, 0);
    for (page, t) in &tallies {
        println!(
            "page {}: first pass got through {} of {}; second pass: ok {}, writer limit {}, unexplained {}",
            page + 1,
            t.ok,
            t.attempted,
            t.second_ok,
            t.writer_limit.len(),
            t.unexplained.len()
        );
        second_ok += t.second_ok;
        limits += t.writer_limit.len();
        unexplained += t.unexplained.len();
        for (block, first, message) in &t.writer_limit {
            println!("  writer limit, page {} block {block} (first object {first}): {message}", page + 1);
        }
        for (block, first, message) in &t.unexplained {
            println!("  UNEXPLAINED page {} block {block} (first object {first}): {message}", page + 1);
        }
    }
    println!(
        "all pages: {attempted} attempts = {ok} ok at once + {second_ok} ok once the hidden pieces are listed + {limits} stopped by a writer limit + {unexplained} unexplained"
    );

    // (the datasheet has hundreds of multi-piece blocks; another document, via PAGIFY_SWEEP_PDF, may have few or none)
    if pdf == DATASHEET {
        assert!(attempted > 100, "only {attempted} attempts were made");
    }
    assert_eq!(unexplained, 0, "{unexplained} of {attempted} edits are neither done nor explained");
    assert_eq!(ok + second_ok + limits, attempted, "every attempt is done or explained");
}

/// Collect the sweep's jobs: every block with more than one text object that
/// the editor guard accepts, one plain line to retype in each. Part of
/// [`every_block_with_more_than_one_object_takes_a_retyped_line_and_undoes_cleanly`].
fn sweep_collect_jobs(reader: &Session, pages: usize) -> (BTreeMap<usize, Tally>, Vec<Job>) {
    let mut tallies: BTreeMap<usize, Tally> = BTreeMap::new();
    let mut jobs: Vec<Job> = Vec::new();
    for page in 0..pages {
        let tally = tallies.entry(page).or_default();
        let snapshot = reader.page_text_snapshot(page).expect("snapshot");
        let runs = snapshot.runs.clone();
        let pb = build_page_blocks(page, 0, 0, snapshot);
        for block in 0..pb.blocks.len() {
            if pb.blocks[block].objects().len() <= 1 {
                continue;
            }
            tally.blocks_with_more_than_one_object += 1;
            // The app opens a block as a paragraph only when the guard passes it (a refused block opens the one
            // word, as it always did): a block it refuses is never retyped line by line, so it is not swept.
            if check_editor_invariants(&pb, block).is_err() {
                tally.guard_refused += 1;
                continue;
            }
            let lines = editor_lines(&pb, block);
            let texts = line_texts(&pb, &lines);
            let Some((i, line)) = lines.iter().enumerate().find(|(_, l)| !l.frozen && !l.objects.is_empty()) else {
                tally.no_plain_line += 1;
                continue;
            };
            jobs.push(Job {
                page,
                block,
                first: line.objects[0],
                rest: line.objects[1..].to_vec(),
                hidden: hidden_pieces(&pb, &runs, &line.objects),
                text: one_word_changed(&texts[i]),
            });
        }
    }
    (tallies, jobs)
}
