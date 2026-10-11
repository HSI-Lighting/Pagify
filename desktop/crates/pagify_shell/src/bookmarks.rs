//! Auto Create Bookmarks: a title for each page, taken from the page's own
//! largest text run — the Navigation tab's button.
//!
//! A shell function over `Session`, so it can be tested against the committed
//! fixtures with no window; the app turns the titles into
//! `Command::AddBookmark`s.

use crate::Session;

/// How long a heading may be before it is cut. An outline entry is a title,
/// not a paragraph.
const TITLE_CAP: usize = 80;

/// The pages that have text to be named after, as `(page, title)` in page
/// order — the trimmed text of each page's largest run, cut to [`TITLE_CAP`].
/// A page with no non-blank text contributes nothing.
///
/// **The largest run is a heuristic, and it is offered as one.** Every
/// auto-bookmark tool since Acrobat's has used it; a page whose biggest words
/// are a watermark gets the watermark as its title. The entries land as
/// ordinary bookmarks, so saying the wrong thing costs one rename or one
/// undo, not a rebuilt document.
pub fn heading_titles(session: &Session) -> pdf_core::Result<Vec<(usize, String)>> {
    let mut out = Vec::new();
    for page in 0..session.page_count()? {
        let snapshot = session.page_text_snapshot(page)?;
        let largest = snapshot
            .runs
            .iter()
            .filter(|run| !run.text.trim().is_empty())
            .max_by(|a, b| a.size.partial_cmp(&b.size).unwrap_or(std::cmp::Ordering::Equal));
        let Some(run) = largest else { continue };
        let mut title = run.text.trim().to_string();
        if title.chars().count() > TITLE_CAP {
            title = title.chars().take(TITLE_CAP - 1).collect::<String>() + "…";
        }
        out.push((page, title));
    }
    Ok(out)
}
