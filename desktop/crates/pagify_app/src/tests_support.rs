thread_local! {
    /// While set on this thread, [`PagifyApp::page_blocks`] fails as if
    /// PDFium could not read the page's text — to prove Edit Text still
    /// works on the run alone. Per thread, so per test.
    pub static SNAPSHOT_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// **The tests that read a real, large file or run recognition take turns.**
/// PDFium is used under one lock for the whole process, so a test that
/// holds it for a second at a time makes every other test wait — the
/// budget tests among them, which time a click and an apply on a busy page
/// against a wall clock. Run together, half a dozen of them queue
/// everything behind them at once; one at a time, the same work is spread
/// out and no other test waits for more than one of them.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// -- helpers for the tests that edit a page and compare it with what it was --

use crate::PagifyApp;
use pdf_core::document::TextRun;

/// Every text run of page 1 as it is now, in object order: words, colour,
/// box, origin and size.
pub fn runs_in_order(app: &PagifyApp) -> Vec<TextRun> {
    runs_on_page(app, 0)
}

/// [`runs_in_order`] for any page.
pub fn runs_on_page(app: &PagifyApp, page: usize) -> Vec<TextRun> {
    let mut runs = app.tab().doc.as_ref().expect("open").session.text_runs(page).expect("runs");
    runs.sort_by_key(|r| r.object);
    runs
}

/// Two runs that read the same: words and colour exactly, box, origin and
/// size within a thousandth of a point. **Object numbers are not
/// compared** — they move when pieces come off the page.
pub fn same_run(a: &TextRun, b: &TextRun) -> bool {
    let near = |x: f32, y: f32| (x - y).abs() < 1e-3;
    a.text == b.text
        && a.color == b.color
        && near(a.rect.left, b.rect.left)
        && near(a.rect.top, b.rect.top)
        && near(a.rect.right, b.rect.right)
        && near(a.rect.bottom, b.rect.bottom)
        && near(a.origin.x, b.origin.x)
        && near(a.origin.y, b.origin.y)
        && near(a.size, b.size)
}

/// Words with whitespace squeezed: PDFium reads two written spaces back as
/// one, and a piece's own trailing space is not always kept.
pub fn squeeze(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `was` and `now` differ only in **which of two coincident objects
/// PDFium credits with a letter**: one reads empty and the other the letter,
/// on the same box, origin, size and colour, and the page had (or has) another
/// object on exactly that spot. The datasheet draws some words twice (a faux
/// bold) and reads each pair as one; when the page is rebuilt after an edit —
/// above all one that retypes the twin that was credited with the letter — the
/// letter can go to the other member of the pair. Nothing visible changed, so
/// it is not a difference for the tests that check "nothing else moved".
/// `before` and `after` are the page's runs either side of the edit.
fn twin_flip(was: &TextRun, now: &TextRun, before: &[TextRun], after: &[TextRun]) -> bool {
    let near = |x: f32, y: f32| (x - y).abs() < 1e-3;
    let same_spot = |a: &TextRun, b: &TextRun| {
        near(a.rect.left, b.rect.left)
            && near(a.rect.top, b.rect.top)
            && near(a.rect.right, b.rect.right)
            && near(a.rect.bottom, b.rect.bottom)
            && near(a.origin.x, b.origin.x)
            && near(a.origin.y, b.origin.y)
            && near(a.size, b.size)
            && a.color == b.color
    };
    was.text.trim().is_empty() != now.text.trim().is_empty()
        && same_spot(was, now)
        && (before.iter().filter(|r| same_spot(r, was)).count() >= 2
            || after.iter().filter(|r| same_spot(r, now)).count() >= 2)
}

/// **What a page reads after an edit that retyped some runs and took others
/// off the page**: the runs from before, in order, without the removed
/// ones, each exactly as it was — except a retyped run, which holds its new
/// words and keeps its colour, origin and size. The page has lost exactly
/// the runs removed, no more. The first way it is not so, said in words.
pub fn page_after_problem(
    before: &[TextRun],
    after: &[TextRun],
    retyped: &[(usize, &str)],
    removed: &[usize],
) -> Option<String> {
    let expected: Vec<&TextRun> = before.iter().filter(|r| !removed.contains(&r.object)).collect();
    if after.len() != expected.len() {
        return Some(format!(
            "the page should have lost exactly the {} removed run(s) — had {}, has {}",
            removed.len(),
            before.len(),
            after.len()
        ));
    }
    for (was, now) in expected.iter().zip(after) {
        match retyped.iter().find(|(object, _)| *object == was.object) {
            Some((_, words)) => {
                // **A hyphen written comes back from PDFium as the control code
                // U+0002** (see `wrap_hyphen_tests`), not as "-": the same hyphen,
                // read two ways. Line-end hyphens are in the buffer now — the thin
                // glyphs the page draws them with are members of their lines — so a
                // retyped line that ends in one reads back differently from how it
                // was typed, for no reason that is damage.
                let hyphens_alike = |s: &str| squeeze(s).replace('\u{2}', "-");
                if hyphens_alike(&now.text) != hyphens_alike(words) {
                    return Some(format!("object {} does not hold the new words: {:?} (wanted {:?})", was.object, now.text, words));
                }
                if now.color != was.color {
                    return Some(format!("the retyped run changed colour: {was:?} now {now:?}"));
                }
                if (now.origin.x - was.origin.x).abs() >= 1e-3
                    || (now.origin.y - was.origin.y).abs() >= 1e-3
                    || (now.size - was.size).abs() >= 1e-3
                {
                    return Some(format!("the retyped run moved or changed size: {was:?} now {now:?}"));
                }
            }
            None if !same_run(was, now) && !twin_flip(was, now, before, after) => {
                return Some(format!("a run nobody touched changed: {was:?} now {now:?}"));
            }
            None => {}
        }
    }
    None
}

/// [`page_after_problem`], failing the test with what it found.
pub fn assert_page_after(
    what: &str,
    before: &[TextRun],
    after: &[TextRun],
    retyped: &[(usize, &str)],
    removed: &[usize],
) {
    if let Some(problem) = page_after_problem(before, after, retyped, removed) {
        panic!("{what}: {problem}");
    }
}

/// The page as a picture, for comparing what is *seen* rather than what is
/// stored.
pub fn picture(app: &PagifyApp) -> Vec<u8> {
    app.tab().doc.as_ref().expect("open").session.render_page(0, 2.0).expect("render").pixels
}

/// Whether two pictures of the page are the same.
pub fn same_picture(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a == b
}
