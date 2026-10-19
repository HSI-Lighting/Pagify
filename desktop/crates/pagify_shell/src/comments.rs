//! Every annotation in the document as one list — behind `managecomments` —
//! and the thread each one belongs to.
//!
//! A shell function over `Session`, so it needs no window and can be tested
//! against fixtures. The list is read once when the panel opens and re-read
//! after every removal; it is not rebuilt every frame.
//!
//! **A thread is the PDF's own.** A text annotation that answers another one
//! carries `/IRT` — "in reply to" — naming the annotation it answers, and the
//! engine reads that link back as [`Comment::reply_to`]. Other readers show the
//! same thread, which is what makes this the honest model: no sidecar file, no
//! Pagify-only convention. Creating a reply is not built yet; reading one that
//! another application made already works.

use std::collections::HashMap;

use crate::Session;
use pdf_core::document::Annotation;

/// One annotation, as the comments panel needs it.
#[derive(Debug, Clone)]
pub struct Comment {
    /// Which page it is on, zero-based.
    pub page: usize,
    /// PDFium's own index for it — what `RemoveAnnotation` takes.
    pub index: usize,
    /// The kind, as a word.
    pub kind: &'static str,
    /// A short reading of what it says, where it says anything.
    pub detail: String,
    /// For a reply: the PDFium index of the annotation it answers, on the same
    /// page. `None` starts a thread.
    pub reply_to: Option<usize>,
}

/// One row of the Comments panel: a mark, and where it sits in a thread.
#[derive(Debug)]
pub struct Row {
    pub comment: Comment,
    /// 0 for a mark that starts a thread, 1 for a reply to it, deeper for a
    /// reply to a reply. The panel indents by this.
    pub depth: usize,
}

/// Every annotation in the document, page by page, in page order.
pub fn comments(session: &Session) -> pdf_core::Result<Vec<Comment>> {
    let mut out = Vec::new();
    for page in 0..session.page_count()? {
        for mark in session.annotations(page)? {
            let (kind, detail, reply_to) = match &mark.annotation {
                Annotation::Highlight { .. } => ("highlight", String::new(), None),
                Annotation::Underline { .. } => ("underline", String::new(), None),
                Annotation::StrikeOut { .. } => ("strikethrough", String::new(), None),
                Annotation::Squiggly { .. } => ("squiggle", String::new(), None),
                Annotation::Ink { strokes, .. } => {
                    ("ink", format!("{} stroke(s)", strokes.len()), None)
                }
                Annotation::Note { contents, reply_to, .. } => {
                    ("note", snippet(contents), *reply_to)
                }
                Annotation::Text { text, .. } => ("text", snippet(text), None),
                Annotation::Image { width, height, .. } => {
                    ("picture", format!("{width}×{height}"), None)
                }
                Annotation::Fill { .. } => ("shape", String::new(), None),
                Annotation::Link { uri, .. } => ("link", snippet(uri), None),
            };
            out.push(Comment { page, index: mark.index, kind, detail, reply_to });
        }
    }
    Ok(out)
}

/// The comments arranged as threads: each mark that starts one followed by its
/// replies, in document order, with the depth the panel indents by.
///
/// **A reply whose parent is not in the list is shown as its own root**, not
/// dropped: it is still a mark somebody put there, and a parent this engine
/// does not model (or one that has been removed) is not a reason to hide it.
/// A hand-written file can also make a chain that loops; the first link that
/// would close the loop is ignored so the list still terminates.
pub fn thread_rows(comments: Vec<Comment>) -> Vec<Row> {
    let count = comments.len();
    let mut by_place: HashMap<(usize, usize), usize> = HashMap::with_capacity(count);
    for (position, comment) in comments.iter().enumerate() {
        by_place.insert((comment.page, comment.index), position);
    }

    let mut parent: Vec<Option<usize>> = vec![None; count];
    for (position, comment) in comments.iter().enumerate() {
        let Some(target) = comment.reply_to else { continue };
        let Some(&candidate) = by_place.get(&(comment.page, target)) else { continue };
        if candidate == position || reaches(candidate, position, &parent) {
            continue;
        }
        parent[position] = Some(candidate);
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut roots = Vec::new();
    for position in 0..count {
        match parent[position] {
            Some(root) => children[root].push(position),
            None => roots.push(position),
        }
    }

    // Depth-first, iterative: a document is somebody else's file and a reply
    // chain a few thousand deep should not be able to run the stack out.
    let mut rows = Vec::with_capacity(count);
    let mut stack: Vec<(usize, usize)> = roots.iter().rev().map(|&root| (root, 0)).collect();
    while let Some((position, depth)) = stack.pop() {
        rows.push(Row { comment: comments[position].clone(), depth });
        for &child in children[position].iter().rev() {
            stack.push((child, depth + 1));
        }
    }
    rows
}

/// Whether following `position`'s parent chain reaches `target` — the cycle
/// check that keeps a hand-written reply loop from becoming a non-terminating
/// one.
fn reaches(position: usize, target: usize, parent: &[Option<usize>]) -> bool {
    let mut at = Some(position);
    for _ in 0..parent.len() {
        match at {
            Some(current) if current == target => return true,
            Some(current) => at = parent[current],
            None => return false,
        }
    }
    false
}

/// The first few words of what a mark says, one line with an ellipsis — the
/// panel is a list, not a reader.
fn snippet(text: &str) -> String {
    let text = text.trim().replace(['\n', '\r'], " ");
    if text.chars().count() > 60 {
        text.chars().take(57).collect::<String>() + "…"
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(page: usize, index: usize, reply_to: Option<usize>) -> Comment {
        Comment {
            page,
            index,
            kind: "note",
            detail: format!("note {index}"),
            reply_to,
        }
    }

    /// The plain list: marks that answer nothing all sit at the top level.
    #[test]
    fn comments_that_start_no_thread_are_all_roots() {
        let rows = thread_rows(vec![comment(0, 1, None), comment(0, 2, None), comment(1, 0, None)]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().map(|row| row.depth).collect::<Vec<_>>(), [0, 0, 0]);
        assert_eq!(rows[0].comment.index, 1);
        assert_eq!(rows[2].comment.page, 1);
    }

    /// A reply follows the note it answers, however far apart they are in the
    /// page's annotation order — and a reply to a reply sits one deeper.
    #[test]
    fn replies_are_indented_under_their_root() {
        let rows = thread_rows(vec![
            comment(0, 5, Some(7)),  // a reply that appears *before* its parent
            comment(0, 7, None),     // the root
            comment(0, 9, Some(5)),  // a reply to the first reply
        ]);
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(rows[0].comment.index, 7, "the root leads its thread");
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].comment.index, 5);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[2].comment.index, 9);
        assert_eq!(rows[2].depth, 2);
    }

    /// Two roots keep their own replies, in document order.
    #[test]
    fn each_thread_keeps_its_own_replies() {
        let rows = thread_rows(vec![
            comment(0, 1, None),
            comment(0, 2, None),
            comment(0, 3, Some(1)),
            comment(0, 4, Some(2)),
        ]);
        let order: Vec<(usize, usize)> = rows.iter().map(|r| (r.comment.index, r.depth)).collect();
        assert_eq!(order, [(1, 0), (3, 1), (2, 0), (4, 1)]);
    }

    /// A reply to something not in the list is still shown — as a root. Its
    /// parent may be a mark this engine does not model, or one already removed.
    #[test]
    fn a_reply_to_a_missing_parent_is_shown_as_a_root() {
        let rows = thread_rows(vec![comment(0, 1, Some(99))]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].depth, 0);

        // And a reply pointing at another page's index is not its parent.
        let rows = thread_rows(vec![comment(0, 1, Some(2)), comment(1, 2, None)]);
        assert_eq!(rows.iter().map(|r| r.depth).collect::<Vec<_>>(), [0, 0]);
    }

    /// A loop a hand-written file could contain terminates, and both marks are
    /// kept: the link that would close the loop is ignored.
    #[test]
    fn a_reply_loop_terminates() {
        let rows = thread_rows(vec![comment(0, 1, Some(2)), comment(0, 2, Some(1))]);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows.iter().filter(|row| row.depth == 0).count(), 1, "{rows:?}");
    }
}
