//! Comparing two documents page by page — `compare`, the View tab's tool.
//!
//! Word-level, and honest about order: two files whose pages carry the same
//! words in a different order are *not* called identical — that is exactly
//! the difference between the committed `two-column.pdf` and `shredded.pdf`,
//! and the point of a comparison is to say what is different, not to smooth
//! it over.
//!
//! The words are the file's own: runs in the order the file stores them,
//! split on whitespace, case kept. That is what selecting all and copying
//! would give — `reflow` is the verb for reading order, and a compare run
//! after a reflow compares the reflowed text.

use crate::Session;

/// What one page's comparison found.
pub enum Verdict {
    /// The same words, in the same order.
    Identical,
    /// Every word the same, in a different order.
    SameWordsDifferentOrder,
    /// The words differ.
    Different {
        /// Words here the other file has no match for (with counts).
        only_here: Vec<String>,
        /// And the other way round.
        only_there: Vec<String>,
    },
}

/// One page's comparison.
pub struct PageComparison {
    /// Zero-based page, one-based in [`describe`].
    pub page: usize,
    pub verdict: Verdict,
}

/// Compare the first `pages` pages of `session` against `other`'s.
pub fn compare(
    session: &Session,
    other: &Session,
    pages: usize,
) -> pdf_core::Result<Vec<PageComparison>> {
    let mut out = Vec::new();
    for page in 0..pages {
        let here = words(session, page)?;
        let there = words(other, page)?;
        let verdict = if here == there {
            Verdict::Identical
        } else {
            let mut sorted_here = here.clone();
            sorted_here.sort();
            let mut sorted_there = there.clone();
            sorted_there.sort();
            if sorted_here == sorted_there {
                Verdict::SameWordsDifferentOrder
            } else {
                Verdict::Different {
                    only_here: residue(&sorted_here, &sorted_there),
                    only_there: residue(&sorted_there, &sorted_here),
                }
            }
        };
        out.push(PageComparison { page, verdict });
    }
    Ok(out)
}

/// One comparison as the line the command bar says. Wording lives here so
/// the app and the tests say the same thing.
pub fn describe(comparison: &PageComparison) -> String {
    match &comparison.verdict {
        Verdict::Identical => format!("page {}: identical.", comparison.page + 1),
        Verdict::SameWordsDifferentOrder => {
            format!("page {}: the same words, in a different order.", comparison.page + 1)
        }
        Verdict::Different { only_here, only_there } => format!(
            "page {}: only here: {}; only there: {}.",
            comparison.page + 1,
            list(only_here),
            list(only_there)
        ),
    }
}

/// At most eight words listed, so one rewritten page cannot fill the bar.
fn list(words: &[String]) -> String {
    if words.is_empty() {
        return "nothing".into();
    }
    if words.len() > 8 {
        return format!("{} (+{} more)", words[..8].join(", "), words.len() - 8);
    }
    words.join(", ")
}

/// A page's words, in the file's own run order.
fn words(session: &Session, page: usize) -> pdf_core::Result<Vec<String>> {
    let snapshot = session.page_text_snapshot(page)?;
    let text = snapshot
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(text.split_whitespace().map(str::to_string).collect())
}

/// The words of `a` with every word of `b` cancelled out once — multiset
/// subtraction, so two `the`s against one `the` leaves one `the`.
fn residue(a: &[String], b: &[String]) -> Vec<String> {
    let mut counts: std::collections::BTreeMap<&str, i64> = std::collections::BTreeMap::new();
    for word in a {
        *counts.entry(word.as_str()).or_default() += 1;
    }
    for word in b {
        *counts.entry(word.as_str()).or_default() -= 1;
    }
    let mut out = Vec::new();
    for (word, count) in counts {
        for _ in 0..count.max(0) {
            out.push(word.to_string());
        }
    }
    out
}
