//! Document-wide counts: what `wordcount` reports.
//!
//! A shell function over `Session`, so it needs no window and can be tested
//! against the committed fixtures.

use crate::Session;

/// Words and characters across `pages`, in the order given, from the pages'
/// own text runs.
///
/// Runs are joined with a space before splitting: they are fragments, and
/// two adjacent runs are two words far more often than they are one. (A word
/// the file itself splits mid-run counts as two — the same limit every
/// extraction here has.)
pub fn words_and_characters(session: &Session, pages: &[usize]) -> pdf_core::Result<(usize, usize)> {
    let mut words = 0usize;
    let mut characters = 0usize;
    for &page in pages {
        let snapshot = session.page_text_snapshot(page)?;
        let text = snapshot
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for word in text.split_whitespace() {
            words += 1;
            characters += word.chars().count();
        }
    }
    Ok((words, characters))
}
