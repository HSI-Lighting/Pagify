//! The lines of a paragraph: what applying an edit does to each of them, and
//! what the engine is told to do about it.
//!
//! Everything in this file is a function of text and object numbers alone — no
//! document, no PDFium — so the decisions can be reasoned about, and tested,
//! without a page. Reading the page and running the result is
//! `PagifyApp::paragraph_edit_commands` and `PagifyApp::apply_paragraph_edit`.
//!
//! **A retyped line replaces its pieces.** A line is often drawn as several
//! pieces — separate show-text operators, one text object each. Typing the
//! line's new words into its first piece and painting the others in the page's
//! colour (what this used to do) is wrong three ways: the painted pieces are
//! drawn *after* the new words, and where one is positioned by its own `Td` and
//! the new first piece is longer, their glyphs erase part of it (measured on a
//! real datasheet: 23% of one line's ink); the old words stay in the file as
//! invisible text, so an old price in an edited quotation is still searchable;
//! and a later pick sees the painted pieces as members of the line again.
//!
//! So a retyped line is [`TextLineEdit::Retype`]: its first piece takes the
//! words and its other pieces are *removed*; a line the person deleted is
//! [`TextLineEdit::Remove`]. A line that still says what it said is not
//! mentioned at all, so a justified line of three to eight pieces stays three
//! to eight pieces. Undo is by page snapshot, which the engine's
//! `Command::ReplaceTextLines` takes — exact, where re-typing the old words
//! loses the kerning of a `TJ`.

use pdf_core::command::Command;
use pdf_core::document::{Rect, TextLineEdit, TextStyle};

/// `"1 line"`, `"13 lines"`: a count with its noun, in the number it is.
pub fn count_of(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What applying an edit does to one line of a paragraph — see
/// [`plan_paragraph_edit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineFate<'a> {
    /// The page draws (part of) this line as shapes: never touched, whatever
    /// was typed over it — see `EditingRun::frozen`.
    Frozen,
    /// Says what it said (compared trimmed) and no new size or font was asked
    /// for: left exactly as the page has it, every piece of it where it is.
    Kept,
    /// Retyped: the new words go onto the line's first object and every other
    /// object of the line comes off the page — see [`line_edits`].
    Written(&'a str),
    /// Nothing is typed for it any more: every object of the line comes off the
    /// page.
    Removed,
}

/// What applying the typed text does to a paragraph, decided from the text
/// alone. Built by [`plan_paragraph_edit`].
#[derive(Debug, PartialEq, Eq)]
pub struct ParagraphPlan<'a> {
    /// One per original line, top to bottom.
    pub fates: Vec<LineFate<'a>>,
    /// How many whole lines at the top and at the bottom the typed text shares
    /// with the original (compared trimmed). For the session log.
    pub prefix: usize,
    pub suffix: usize,
    /// Typed lines with no original line left to hold them, top to bottom:
    /// written as new lines below original line `surplus_below`.
    pub surplus: Vec<&'a str>,
    pub surplus_below: usize,
    /// Frozen lines whose typed text is not what they said — a line typed away
    /// counts, as the empty text it became. Those words were written nowhere.
    pub frozen_changed: usize,
}

/// Decide what an apply does to each line of a paragraph.
///
/// `original` is the paragraph's text as picked, one entry per line; `frozen`
/// says which of those lines the page draws as shapes (a missing flag reads as
/// not frozen); `typed` is the buffer split into lines; `restyle` says a new
/// size or font was asked for, which has to reach every line that is not
/// frozen — so none of those is left alone.
///
/// **Lines are matched by content, not by position**: the longest run of lines
/// at the top and at the bottom that say what they said (compared trimmed) is
/// found first, and only the lines between them are paired off in order, so
/// every line after a touched region keeps its own object whichever index it
/// falls at. (Zipping by index shifted every later line onto the previous
/// line's object after an Enter or a Backspace across a line break — reported
/// from use, twice.) A genuine net growth in the touched middle has nowhere
/// existing to go and is `surplus`.
///
/// What each original line then gets:
/// * **frozen** → [`LineFate::Frozen`], always, and counted in
///   `frozen_changed` when what is typed over it differs from what it said;
/// * typed text that is not blank → [`LineFate::Written`] — **unless it says
///   what the line said** (trimmed) and `restyle` is off, which is
///   [`LineFate::Kept`]: not written at all;
/// * no typed text, or a blank line → [`LineFate::Removed`], except that a
///   line that was blank to begin with has nothing to remove and is `Kept`.
///
/// **Blank lines typed past the paragraph's own line count are dropped**: an
/// Enter at the very end, or blank lines left behind, are not words. Blank
/// lines inside the paragraph's own count are kept — a blanked line is how a
/// line is removed.
pub fn plan_paragraph_edit<'a>(
    original: &[&str],
    frozen: &[bool],
    typed: &[&'a str],
    restyle: bool,
) -> ParagraphPlan<'a> {
    let n = original.len();
    let mut typed = typed;
    while typed.len() > n && typed.last().is_some_and(|line| line.trim().is_empty()) {
        typed = &typed[..typed.len() - 1];
    }
    let m = typed.len();

    // **Trimmed for the comparison only — the real, untrimmed line is what
    // gets written.**
    let prefix = original.iter().zip(typed).take_while(|(a, b)| a.trim() == b.trim()).count();
    let max_suffix = n.saturating_sub(prefix).min(m.saturating_sub(prefix));
    let suffix = (0..max_suffix)
        .take_while(|&k| original[n - 1 - k].trim() == typed[m - 1 - k].trim())
        .count();

    let orig_mid_end = n - suffix;
    let new_mid_end = m - suffix;
    let shared_mid = (orig_mid_end - prefix).min(new_mid_end - prefix);

    // Which typed line (if any) original line `i` now holds — not `typed[i]`:
    // unchanged lines on either side of the touched region keep their own
    // index on their own side.
    let typed_for = |i: usize| -> Option<&'a str> {
        let j = if i < prefix {
            i
        } else if i >= orig_mid_end {
            i + new_mid_end - orig_mid_end
        } else if i - prefix < shared_mid {
            i
        } else {
            return None;
        };
        typed.get(j).copied()
    };

    let mut frozen_changed = 0;
    let fates: Vec<LineFate<'a>> = (0..n)
        .map(|i| {
            let line = typed_for(i);
            if frozen.get(i).copied().unwrap_or(false) {
                if line.unwrap_or("").trim() != original[i].trim() {
                    frozen_changed += 1;
                }
                return LineFate::Frozen;
            }
            match line.filter(|text| !text.trim().is_empty()) {
                None if original[i].trim().is_empty() => LineFate::Kept,
                None => LineFate::Removed,
                Some(text) if !restyle && text.trim() == original[i].trim() => LineFate::Kept,
                Some(text) => LineFate::Written(text),
            }
        })
        .collect();

    let surplus = if new_mid_end > orig_mid_end {
        typed[prefix + shared_mid..new_mid_end].to_vec()
    } else {
        Vec::new()
    };
    ParagraphPlan {
        fates,
        prefix,
        suffix,
        surplus,
        surplus_below: orig_mid_end.saturating_sub(1),
        frozen_changed,
    }
}

/// Whether a paragraph's pieces still line up, before anything is built from
/// them: one entry of text and one frozen flag per line, a text object to write
/// to on every line that is not frozen, and no object on two lines.
///
/// `twins` are the objects that duplicate a piece of a line — a faux bold: the
/// same word drawn twice, a hair apart — one list per line, or none at all for
/// a paragraph without any. They travel with their line (see [`line_edits`]), so
/// the same must hold of them: a list per line, none of them a piece of a line,
/// none named twice. The engine refuses a batch that names an object twice, and
/// writing to a twin as if it were a piece would put the words on the page twice.
///
/// A release build used to index past the end, or pair every later line with
/// the wrong object and write the words there (only a `debug_assert` stood
/// guard). An `Err` comes before anything is built, so the page is as it was.
pub fn check_lines_up(
    text_lines: usize,
    lines: &[(Vec<usize>, Rect)],
    twins: &[Vec<usize>],
    frozen: &[bool],
) -> Result<(), String> {
    let n = lines.len();
    let reason = if n == 0 || text_lines != n || frozen.len() != n {
        format!(
            "{}, {} of text, {}",
            count_of(n, "line", "lines"),
            count_of(text_lines, "line", "lines"),
            count_of(frozen.len(), "frozen flag", "frozen flags")
        )
    } else if !twins.is_empty() && twins.len() != n {
        format!("{}, {}", count_of(n, "line", "lines"), count_of(twins.len(), "list of twins", "lists of twins"))
    } else if let Some(i) = lines
        .iter()
        .zip(frozen)
        .position(|((objects, _), frozen)| objects.is_empty() && !frozen)
    {
        format!("line {} has no text to write to", i + 1)
    } else if {
        let mut seen = std::collections::HashSet::new();
        lines.iter().flat_map(|(objects, _)| objects).any(|object| !seen.insert(*object))
    } {
        "an object belongs to two lines".to_string()
    } else if {
        // Every object of the paragraph, pieces and twins together, is named once.
        let mut seen: std::collections::HashSet<usize> =
            lines.iter().flat_map(|(objects, _)| objects.iter().copied()).collect();
        twins.iter().flatten().any(|twin| !seen.insert(*twin))
    } {
        "a twin is also a piece of a line, or is listed twice".to_string()
    } else {
        return Ok(());
    };
    Err(format!(
        "this paragraph no longer lines up with what was picked ({reason}), so nothing was \
         changed — pick it again."
    ))
}

/// What the engine is told for a plan, one edit per line that changes, top to
/// bottom: a [`LineFate::Written`] line is a [`TextLineEdit::Retype`] — its
/// first object takes the words, **every other object of the line is removed**
/// — and a [`LineFate::Removed`] line is a [`TextLineEdit::Remove`] of all its
/// objects. `Kept` and `Frozen` lines are not mentioned.
///
/// A single-piece line is a `Retype` with nothing to remove, not a different
/// call: one path, one transaction, and one undo (a page snapshot, which is
/// exact), where re-typing a single piece through `SetTextRuns` undoes by
/// re-typing the old words.
///
/// **`style` is narrowed to what a `Retype` can carry — the font and the size.**
/// A colour or a position on a `Retype` is refused by the engine (and the
/// paragraph's colour is applied separately, see [`recolour_targets`]), so
/// neither is ever passed on, whatever the caller's style holds.
///
/// `justified` says the paragraph is one whose retyped lines are to be kept as
/// wide as they were (see [`justify_targets`]); each retyped line then carries
/// the width it is to span in `justify_to`.
///
/// **A line's twins go with it.** A twin is an object that duplicates a piece of
/// the line (the faux bold of a heading: every letter drawn twice, a hair
/// apart); it is not part of the line's words, and left behind it goes on
/// drawing the old words over the new ones. So a retyped line removes its twins
/// besides its other pieces, and a removed line removes them besides its
/// pieces. `twins` has one list per line (or is empty, for a paragraph with
/// none); a line that is kept or frozen is not written, so **its twins are never
/// touched** — a frozen line above all, which is never written at all.
///
/// A line with no object has nothing to write to or remove and produces
/// nothing; [`check_lines_up`] refuses such a paragraph before it gets here.
pub fn line_edits(
    fates: &[LineFate<'_>],
    lines: &[(Vec<usize>, Rect)],
    twins: &[Vec<usize>],
    style: &TextStyle,
    justified: bool,
) -> Vec<TextLineEdit> {
    let style = TextStyle { face: style.face.clone(), size: style.size, ..Default::default() };
    let spans = justify_targets(fates, lines, justified);
    fates
        .iter()
        .zip(lines)
        .zip(spans)
        .enumerate()
        .filter_map(|(i, ((fate, (objects, _)), justify_to))| {
            let twins = twins.get(i).map_or(&[][..], Vec::as_slice);
            match (fate, objects.split_first()) {
                (LineFate::Written(text), Some((first, rest))) => Some(TextLineEdit::Retype {
                    first: *first,
                    text: (*text).to_string(),
                    style: style.clone(),
                    remove: rest.iter().chain(twins).copied().collect(),
                    justify_to,
                }),
                (LineFate::Removed, Some(_)) => {
                    Some(TextLineEdit::Remove { objects: objects.iter().chain(twins).copied().collect() })
                }
                _ => None,
            }
        })
        .collect()
}

/// Whether a retyped line of a justified paragraph is asked to keep its width at
/// all — [`TextLineEdit::Retype`]'s `justify_to`.
///
/// **On: the engine honours the field.** It spreads the words over exactly the
/// width asked for (the left edge stays where it was; the width is the right edge
/// minus the left of the *original* line's box), exact to a thousandth of a point
/// on the datasheet's lines and CAMINO's. It refuses, whole and with nothing
/// written, a line it cannot stretch — a one-letter line, a gap that would have to
/// close or open by more than a third of an em, angled text — and does not say
/// which line, so [`PagifyApp::apply_paragraph_edit`] asks again without the width
/// and says the lines came out ragged ([`asks_for_width`]).
///
/// Kept as a switch rather than deleted so that a regression in the engine's
/// stretching can be turned off in one word: with it off a retyped line of a
/// justified paragraph ends short, and the detector then reads that line as the
/// paragraph's last.
pub const STRETCH_JUSTIFIED_LINES: bool = true;

/// Whether any retyped line in `commands` asks for the width it had — whether a
/// refusal of them might be the stretching's doing, and so worth asking again
/// without it.
pub fn asks_for_width(commands: &[Command]) -> bool {
    commands.iter().any(|command| match command {
        Command::ReplaceTextLines { edits, .. } => edits
            .iter()
            .any(|edit| matches!(edit, TextLineEdit::Retype { justify_to: Some(_), .. })),
        _ => false,
    })
}

/// Whether a paragraph's lines end at one margin — what tells a **justified**
/// paragraph from one that is merely left-aligned.
///
/// The editor draws a block justified whenever its lines start at one margin
/// (`paragraph_should_justify`), which is also true of every ordinary
/// left-aligned, ragged-right paragraph. Stretching a retyped line back to its
/// old width is right for the first and wrong for the second (a word deleted
/// from a ragged line would be spaced out to fill the room it left), so what a
/// line is asked to span ([`justify_targets`]) also needs this: most of the
/// lines that are not the last end within a few points of the same right edge.
/// Measured on the datasheet (spread 0.7 pt) and on CAMINO, whose justified
/// lines end at 313.67 except a few that end 3.2 pt short — hence the
/// tolerance of 4 pt and the "three quarters" rather than "all".
///
/// A paragraph of one line has no margin to speak of: `false`. One non-last
/// line is trivially at "one margin"; with so little to go on it counts.
pub fn ends_at_one_margin(lines: &[(Vec<usize>, Rect)]) -> bool {
    let rights: Vec<f32> = lines[..lines.len().saturating_sub(1)]
        .iter()
        .map(|(_, rect)| rect.left.max(rect.right))
        .collect();
    let Some(furthest) = rights.iter().copied().reduce(f32::max) else { return false };
    let at_the_margin = rights.iter().filter(|right| furthest - **right <= 4.0).count();
    at_the_margin * 4 >= rights.len() * 3
}

/// How wide each line of a justified paragraph should be left once it is
/// retyped, in points — `None` for every line that is not to be stretched.
///
/// A retyped line loses the producer's inter-word spacing (the engine writes
/// plain words), so a justified line comes out short of the margin — and the
/// paragraph detector, which reads a short line as the paragraph's last, then
/// opens only part of the paragraph the next time it is clicked. So a line that
/// is **retyped, in a paragraph that is justified** (`justified`: the editor's
/// own `paragraph_should_justify` — its lines start at one margin — and
/// [`ends_at_one_margin`], which the editor's does not ask), and is **not the
/// paragraph's last line**, is asked to span the width the original line had:
/// the right edge minus the left of its box, which is the union of its pieces.
/// The engine stretches the spaces to that width, never shrinks, and caps it.
///
/// Nothing for a kept line (it is not written), a removed or frozen one, the
/// paragraph's last line (a short closing line is not stretched to fill the
/// column just because the lines above it were), or any line of a ragged
/// paragraph. "Last" is the last of the *original* lines: a last line that was
/// removed does not make the one above it the end of the paragraph — it was
/// full width, and stays so.
pub fn justify_targets(
    fates: &[LineFate<'_>],
    lines: &[(Vec<usize>, Rect)],
    justified: bool,
) -> Vec<Option<f32>> {
    let last = lines.len().saturating_sub(1);
    fates
        .iter()
        .zip(lines)
        .enumerate()
        .map(|(i, (fate, (_, rect)))| match fate {
            LineFate::Written(_) if justified && i != last => Some((rect.right - rect.left).abs()),
            _ => None,
        })
        .collect()
}

/// How many objects the edits take off the page: the pieces the `Retype`s
/// remove and every object of a `Remove`. **Zero means nothing is renumbered**
/// (a page whose lines are all one piece keeps its object numbers); anything
/// else moves every object after a removed one down.
pub fn removed_pieces(edits: &[TextLineEdit]) -> usize {
    edits
        .iter()
        .map(|edit| match edit {
            TextLineEdit::Retype { remove, .. } => remove.len(),
            TextLineEdit::Remove { objects } => objects.len(),
        })
        .sum()
}

/// The objects that are still on the page once the edit has run and are not
/// drawn as shapes — the ones a new colour for the whole paragraph has to
/// reach: every piece of a line that is kept, and the first piece of a line
/// that is retyped (its other pieces are about to be removed). Nothing for a
/// removed line or a frozen one.
pub fn recolour_targets(fates: &[LineFate<'_>], lines: &[(Vec<usize>, Rect)]) -> Vec<usize> {
    let mut targets = Vec::new();
    for (fate, (objects, _)) in fates.iter().zip(lines) {
        match fate {
            LineFate::Kept => targets.extend(objects.iter().copied()),
            LineFate::Written(_) => targets.extend(objects.first().copied()),
            LineFate::Frozen | LineFate::Removed => {}
        }
    }
    targets
}

/// What to execute for one paragraph, in order.
///
/// **The colour first, then the replace**: the colour edits name objects by
/// number and the replace renumbers the page (every object after a removed one
/// moves down). Run as one `Command::Batch`, undo runs in reverse — the page
/// snapshot first, which puts the page back as it was just before the replace,
/// and then the colour on objects that are numbered as they were when it ran.
///
/// Nothing at all when there is nothing to do.
pub fn commands_for(
    page: usize,
    recolour: Vec<(usize, String, TextStyle)>,
    edits: Vec<TextLineEdit>,
) -> Vec<Command> {
    let mut commands = Vec::new();
    if !recolour.is_empty() {
        commands.push(Command::SetTextRuns { page_index: page, edits: recolour });
    }
    if !edits.is_empty() {
        commands.push(Command::ReplaceTextLines { page_index: page, edits });
    }
    commands
}

/// The session-log line for a plan. **Content-free, on purpose** — line
/// *indices* and counts, never the words on them: which lines this specific
/// edit decided were unchanged, retyped, removed or left alone is exactly what
/// a report of a wrong line needs to confirm or rule out, and it does not need
/// the page's own text.
pub fn plan_log_line(
    typed_lines: usize,
    plan: &ParagraphPlan<'_>,
    lines: &[(Vec<usize>, Rect)],
    twins: &[Vec<usize>],
) -> String {
    let mut retyped = Vec::new();
    let mut removed = Vec::new();
    let mut frozen = Vec::new();
    let mut kept = 0usize;
    let mut pieces_off = 0usize;
    let mut twins_off = 0usize;
    for (i, (fate, (objects, _))) in plan.fates.iter().zip(lines).enumerate() {
        let twin_count = twins.get(i).map_or(0, Vec::len);
        match fate {
            LineFate::Written(_) => {
                retyped.push(i);
                pieces_off += objects.len().saturating_sub(1);
                twins_off += twin_count;
            }
            LineFate::Removed => {
                removed.push(i);
                pieces_off += objects.len();
                twins_off += twin_count;
            }
            LineFate::Frozen => frozen.push(i),
            LineFate::Kept => kept += 1,
        }
    }
    // Only said when there are any, so the line for a paragraph without twins
    // is what it always was.
    let twins_said = if twins_off > 0 { format!(" (and {twins_off} twins)") } else { String::new() };
    format!(
        "paragraph diff: {} original lines, {typed_lines} typed lines, prefix {}, suffix {}, \
         retyped lines {retyped:?}, removed lines {removed:?}, {pieces_off} pieces taken off the \
         page{twins_said}, frozen lines {frozen:?} ({} of them typed over), unchanged lines {kept}",
        plan.fates.len(),
        plan.prefix,
        plan.suffix,
        plan.frozen_changed
    )
}

/// What a paragraph apply says once it has worked.
///
/// `nothing_written` is an apply that found nothing to change on the page —
/// every line says what it said, or is drawn as shapes. Frozen lines whose
/// typed text differs from their own are named, so the person is not left
/// believing words they typed over a drawn line are on the page; and a new
/// position is named as not applied, because a paragraph is several lines and
/// has no single place to move to.
pub fn paragraph_applied_message(
    nothing_written: bool,
    frozen_changed: usize,
    position_ignored: bool,
) -> String {
    let mut said = String::from(if nothing_written { "unchanged" } else { "paragraph changed" });
    match frozen_changed {
        0 => {}
        1 => said.push_str("; 1 line drawn as shapes was left as it is"),
        n => said.push_str(&format!("; {n} lines drawn as shapes were left as they are")),
    }
    if position_ignored {
        said.push_str("; the position was not changed — a paragraph has no single place to move to");
    }
    said.push('.');
    said
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_core::document::Color;
    use LineFate::{Frozen, Kept, Removed, Written};

    const BOX: Rect = Rect { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 };

    // The paragraphs most of these tests build have no twins, so the functions
    // that take a list of them are called through these, which pass none. The
    // tests that are about twins call the real ones (`super::…`) with theirs.
    fn line_edits(
        fates: &[LineFate<'_>],
        lines: &[(Vec<usize>, Rect)],
        style: &TextStyle,
        justified: bool,
    ) -> Vec<TextLineEdit> {
        super::line_edits(fates, lines, &[], style, justified)
    }

    fn check_lines_up(text_lines: usize, lines: &[(Vec<usize>, Rect)], frozen: &[bool]) -> Result<(), String> {
        super::check_lines_up(text_lines, lines, &[], frozen)
    }

    fn plan_log_line(typed_lines: usize, plan: &ParagraphPlan<'_>, lines: &[(Vec<usize>, Rect)]) -> String {
        super::plan_log_line(typed_lines, plan, lines, &[])
    }

    /// Lines from their object lists.
    fn lines(objects: &[&[usize]]) -> Vec<(Vec<usize>, Rect)> {
        objects.iter().map(|o| (o.to_vec(), BOX)).collect()
    }

    // -- the plan ------------------------------------------------------------

    /// The plan for `typed` over `original` (both `\n`-joined), `frozen` as given.
    fn plan<'a>(original: &str, frozen: &[bool], typed: &'a str, restyle: bool) -> ParagraphPlan<'a> {
        let original: Vec<&str> = original.split('\n').collect();
        let typed: Vec<&'a str> = typed.split('\n').collect();
        plan_paragraph_edit(&original, frozen, &typed, restyle)
    }

    #[test]
    fn lines_that_say_what_they_said_are_kept_and_only_the_retyped_one_is_written() {
        let p = plan("one\ntwo\nthree\nfour", &[false; 4], "one\nTWO\nthree\nfour", false);
        assert_eq!(p.fates, [Kept, Written("TWO"), Kept, Kept]);
        assert_eq!((p.prefix, p.suffix), (1, 2));
        assert!(p.surplus.is_empty());
    }

    /// Not only the common prefix and suffix: a line in the middle of the
    /// touched region that happens to say what it said is kept too.
    #[test]
    fn a_line_inside_the_touched_region_that_says_what_it_said_is_kept_too() {
        let p = plan("a\nb\nc\nd\ne", &[false; 5], "a\nB\nc\nD\ne", false);
        assert_eq!(p.fates, [Kept, Written("B"), Kept, Written("D"), Kept]);
        assert_eq!((p.prefix, p.suffix), (1, 1));
    }

    /// Compared trimmed: whitespace at a line's edges is not a change.
    #[test]
    fn a_line_that_differs_only_in_its_edge_whitespace_is_kept() {
        assert_eq!(plan("one\ntwo ", &[false; 2], " one\ntwo", false).fates, [Kept, Kept]);
    }

    #[test]
    fn a_removed_line_is_removed_and_so_is_one_blanked_with_spaces() {
        let gone = plan("a\nb\nc\nd", &[false; 4], "a\nb", false);
        assert_eq!(gone.fates, [Kept, Kept, Removed, Removed]);
        // Blanked with spaces: removed like every other deleted line — not
        // *written* as spaces, which encodes to no codes at all.
        let blanked = plan("a\nb\nc\nd", &[false; 4], "a\n  \nc\nd", false);
        assert_eq!(blanked.fates, [Kept, Removed, Kept, Kept]);
    }

    #[test]
    fn a_line_that_was_blank_has_nothing_to_remove() {
        assert_eq!(plan("a\n \nc", &[false; 3], "a\n\nc", false).fates, [Kept, Kept, Kept]);
        assert_eq!(plan("a\n \nc", &[false; 3], "a\nc", false).fates, [Kept, Kept, Kept]);
    }

    /// A new size or font has to reach every line, so none is passed over as
    /// unchanged — except a frozen one, which nothing can be written to.
    #[test]
    fn a_new_size_or_font_reaches_every_line_that_is_not_frozen() {
        let p = plan("a\nb\nc", &[false, true, false], "a\nb\nc", true);
        assert_eq!(p.fates, [Written("a"), Frozen, Written("c")]);
        assert_eq!(p.frozen_changed, 0);
    }

    #[test]
    fn frozen_lines_are_never_written_or_removed_and_those_typed_over_are_counted() {
        let p = plan("A\n\nB\nC", &[false, true, true, false], "A2\n\nB typed\nC2", false);
        assert_eq!(p.fates, [Written("A2"), Frozen, Frozen, Written("C2")]);
        assert_eq!(p.frozen_changed, 1, "only the line with words typed over it counts; the empty one stayed empty");

        // A frozen line typed away is a change to what it said — and still not removed.
        let deleted = plan("A\nB\nC", &[false, true, false], "A\nC", false);
        assert_eq!(deleted.fates, [Kept, Frozen, Kept]);
        assert_eq!(deleted.frozen_changed, 1);
        // ...unless it said nothing.
        let empty = plan("A\n\nC", &[false, true, false], "A\nC", false);
        assert_eq!(empty.fates, [Kept, Frozen, Kept]);
        assert_eq!(empty.frozen_changed, 0);
    }

    /// An empty frozen first line is an empty first line of the buffer, and
    /// every other line must stay matched to its own object.
    #[test]
    fn a_leading_empty_frozen_line_keeps_every_other_line_matched_to_its_own() {
        let p = plan("\nA\nB\nC", &[true, false, false, false], "\nA\nB\nC2", false);
        assert_eq!(p.fates, [Frozen, Kept, Kept, Written("C2")]);

        // Not what the apply does — it is why the apply does not trim the
        // buffer as a whole: with the leading empty line trimmed away, every
        // line pairs with the one before it, line "A" is retyped as "B" and
        // the last line is removed.
        let trimmed = plan("\nA\nB\nC", &[true, false, false, false], "A\nB\nC2", false);
        assert_eq!(trimmed.fates, [Frozen, Written("B"), Written("C2"), Removed]);
    }

    /// An Enter at the very end, or blank lines left behind, are not words.
    #[test]
    fn blank_lines_typed_past_the_paragraphs_own_count_are_not_words() {
        let p = plan("a\nb", &[false; 2], "a\nb\n\n  ", false);
        assert_eq!(p.fates, [Kept, Kept]);
        assert!(p.surplus.is_empty());

        let grown = plan("a\nb", &[false; 2], "a\nb\n\nx", false);
        assert_eq!(grown.surplus, ["", "x"]);

        // Inside the paragraph's own count a blank line is how a line is removed.
        let inside = plan("a\nb\nc", &[false; 3], "a\nb\n", false);
        assert_eq!(inside.fates, [Kept, Kept, Removed]);
    }

    #[test]
    fn a_grown_paragraphs_new_lines_go_below_the_line_above_the_touched_region() {
        let middle = plan("a\nb\nc", &[false; 3], "a\nb\nNEW\nc", false);
        assert_eq!(middle.fates, [Kept, Kept, Kept]);
        assert_eq!((middle.surplus.as_slice(), middle.surplus_below), (&["NEW"][..], 1));

        let tail = plan("a\nb", &[false; 2], "a\nb\nmore", false);
        assert_eq!((tail.surplus.as_slice(), tail.surplus_below), (&["more"][..], 1));

        let head = plan("a\nb", &[false; 2], "new\na\nb", false);
        assert_eq!((head.surplus.as_slice(), head.surplus_below), (&["new"][..], 0));
    }

    #[test]
    fn the_plan_never_panics_on_odd_input() {
        let nothing_original = plan_paragraph_edit(&[], &[], &["x"], false);
        assert!(nothing_original.fates.is_empty());
        assert_eq!(nothing_original.surplus, ["x"]);
        // Nothing typed at all, and fewer flags than lines.
        let nothing_typed = plan_paragraph_edit(&["a", "b"], &[true], &[], false);
        assert_eq!(nothing_typed.fates, [Frozen, Removed]);
    }

    // -- the conversion to what the engine is told ---------------------------

    fn retype(first: usize, text: &str, remove: &[usize]) -> TextLineEdit {
        TextLineEdit::Retype {
            first,
            text: text.to_string(),
            style: TextStyle::default(),
            remove: remove.to_vec(),
            justify_to: None,
        }
    }

    /// **A retyped line replaces its pieces**: the first takes the words and
    /// the others are removed — for two pieces, and for three.
    #[test]
    fn a_retyped_line_puts_the_words_on_its_first_piece_and_removes_the_others() {
        let l = lines(&[&[10, 11], &[12], &[13, 14, 15]]);
        let edits = line_edits(&[Written("a"), Written("b"), Written("c")], &l, &TextStyle::default(), false);
        assert_eq!(edits, [retype(10, "a", &[11]), retype(12, "b", &[]), retype(13, "c", &[14, 15])]);
    }

    /// A line the person deleted comes off the page whole, every piece of it.
    #[test]
    fn a_removed_line_removes_every_one_of_its_pieces() {
        let l = lines(&[&[1], &[2, 3, 4]]);
        let edits = line_edits(&[Removed, Removed], &l, &TextStyle::default(), false);
        assert_eq!(
            edits,
            [
                TextLineEdit::Remove { objects: vec![1] },
                TextLineEdit::Remove { objects: vec![2, 3, 4] }
            ]
        );
    }

    /// Kept and frozen lines are not mentioned, so nothing about them can
    /// change: not their words, not their pieces.
    #[test]
    fn kept_and_frozen_lines_produce_nothing() {
        let l = lines(&[&[1, 2], &[], &[3, 4], &[5, 6, 7]]);
        let edits = line_edits(&[Kept, Frozen, Kept, Frozen], &l, &TextStyle::default(), false);
        assert!(edits.is_empty(), "{edits:?}");

        // And in a mixture only the lines that change are there, in order.
        let edits = line_edits(&[Kept, Frozen, Written("x"), Removed], &l, &TextStyle::default(), false);
        assert_eq!(edits, [retype(3, "x", &[4]), TextLineEdit::Remove { objects: vec![5, 6, 7] }]);
    }

    /// The font and the size ride along on every retype — and **nothing else
    /// does**: the engine refuses a colour or a position on a `Retype`, and
    /// whatever the caller's style holds, neither is passed on.
    #[test]
    fn only_the_font_and_the_size_reach_a_retype() {
        let full = TextStyle {
            size: Some(14.0),
            color: Some(Color { r: 200, g: 0, b: 0, a: 255 }),
            at: Some((10.0, 20.0)),
            face: Some("Fraunces".to_string()),
        };
        let l = lines(&[&[1, 2], &[3]]);
        let edits = line_edits(&[Written("a"), Written("b")], &l, &full, false);
        let wanted = TextStyle { size: Some(14.0), color: None, at: None, face: Some("Fraunces".to_string()) };
        for edit in &edits {
            let TextLineEdit::Retype { style, .. } = edit else { panic!("not a retype: {edit:?}") };
            assert_eq!(style, &wanted);
        }
        assert_eq!(edits.len(), 2);
    }

    /// A line with no object has nothing to write to or remove.
    #[test]
    fn a_line_with_no_object_produces_nothing_rather_than_panicking() {
        let l = lines(&[&[], &[]]);
        assert!(line_edits(&[Written("x"), Removed], &l, &TextStyle::default(), false).is_empty());
    }

    /// No object is named twice across a whole paragraph's edits — the engine
    /// refuses a batch that does.
    #[test]
    fn no_object_is_named_twice_in_one_paragraphs_edits() {
        let l = lines(&[&[1, 2, 3], &[4, 5], &[6], &[7, 8]]);
        let edits = line_edits(&[Written("a"), Removed, Kept, Written("d")], &l, &TextStyle::default(), false);
        let mut named = Vec::new();
        for edit in &edits {
            match edit {
                TextLineEdit::Retype { first, remove, .. } => {
                    named.push(*first);
                    named.extend(remove);
                }
                TextLineEdit::Remove { objects } => named.extend(objects),
            }
        }
        let mut sorted = named.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(named.len(), sorted.len(), "an object was named twice: {named:?}");
        assert_eq!(sorted, [1, 2, 3, 4, 5, 7, 8], "the kept line's object must not be named");
    }

    /// The pieces taken off the page are counted across every kind of edit —
    /// zero is what tells the apply that no object number moved.
    #[test]
    fn the_pieces_taken_off_the_page_are_counted_across_both_kinds_of_edit() {
        assert_eq!(removed_pieces(&[]), 0);
        assert_eq!(removed_pieces(&[retype(1, "a", &[]), retype(2, "b", &[])]), 0);
        assert_eq!(removed_pieces(&[retype(1, "a", &[2, 3]), TextLineEdit::Remove { objects: vec![4, 5, 6] }]), 5);
        let l = lines(&[&[1, 2], &[3], &[4, 5, 6]]);
        let edits = line_edits(&[Written("x"), Written("y"), Removed], &l, &TextStyle::default(), false);
        assert_eq!(removed_pieces(&edits), 1 + 0 + 3);
    }

    // -- faux-bold twins ------------------------------------------------------

    /// **A retyped line takes its twins off the page with its other pieces**, and
    /// a removed line with its pieces: left behind, a twin goes on drawing the old
    /// words over the new ones. The first piece's own twin is among them.
    #[test]
    fn a_retyped_or_removed_line_removes_its_twins_with_it() {
        let l = lines(&[&[10, 11], &[12], &[13, 14]]);
        let twins = vec![vec![20, 21], vec![22], vec![]];
        let edits = super::line_edits(
            &[Written("a"), Written("b"), Removed],
            &l,
            &twins,
            &TextStyle::default(),
            false,
        );
        assert_eq!(
            edits,
            [
                retype(10, "a", &[11, 20, 21]),
                retype(12, "b", &[22]),
                TextLineEdit::Remove { objects: vec![13, 14] },
            ]
        );

        let removed = super::line_edits(&[Removed, Kept, Kept], &l, &twins, &TextStyle::default(), false);
        assert_eq!(removed, [TextLineEdit::Remove { objects: vec![10, 11, 20, 21] }]);
    }

    /// **A twin of a line that is not written is never touched** — a kept line, and
    /// above all a frozen one, which is never written at all.
    #[test]
    fn the_twins_of_a_kept_or_frozen_line_are_left_where_they_are() {
        let l = lines(&[&[1, 2], &[], &[3], &[4]]);
        let twins = vec![vec![9], vec![8], vec![7], vec![6]];
        let edits = super::line_edits(&[Kept, Frozen, Kept, Written("x")], &l, &twins, &TextStyle::default(), false);
        assert_eq!(edits, [retype(4, "x", &[6])], "only the retyped line's twin may be named");
        for edit in &edits {
            let TextLineEdit::Retype { remove, .. } = edit else { panic!("{edit:?}") };
            assert!(![9, 8, 7].iter().any(|t| remove.contains(t)), "{remove:?}");
        }
    }

    /// A line of one piece with a twin still renumbers the page: the twin is a piece
    /// that comes off, and the apply has to know.
    #[test]
    fn a_twin_counts_among_the_pieces_taken_off_the_page() {
        let l = lines(&[&[1], &[2]]);
        let with = super::line_edits(&[Written("a"), Kept], &l, &[vec![5], vec![]], &TextStyle::default(), false);
        assert_eq!(removed_pieces(&with), 1);
        let without = super::line_edits(&[Written("a"), Kept], &l, &[], &TextStyle::default(), false);
        assert_eq!(removed_pieces(&without), 0);
    }

    /// **No object is named twice**, twins included: the engine refuses a batch that
    /// does, and a twin that is also a piece of a line would be written to and removed.
    #[test]
    fn a_twin_that_is_a_piece_or_is_listed_twice_is_refused_and_so_is_a_list_per_missing_line() {
        let l = lines(&[&[1], &[2, 3]]);
        let ok = |twins: &[Vec<usize>]| super::check_lines_up(2, &l, twins, &[false, false]);
        assert!(ok(&[]).is_ok(), "a paragraph without twins");
        assert!(ok(&[vec![9], vec![]]).is_ok());
        for (what, twins) in [
            ("a twin that is a piece of another line", vec![vec![3], vec![]]),
            ("a twin that is a piece of its own line", vec![vec![1], vec![]]),
            ("a twin listed on two lines", vec![vec![9], vec![9]]),
            ("a twin listed twice on a line", vec![vec![9, 9], vec![]]),
            ("a list of twins too few", vec![vec![9]]),
        ] {
            let refused = ok(&twins).err().unwrap_or_else(|| panic!("{what} was not refused"));
            assert!(refused.contains("no longer lines up") && refused.contains("nothing was changed"), "{what}: {refused}");
        }
    }

    /// The log says how many twins came off, by count, and says nothing of them
    /// for a paragraph that has none (its line is what it always was).
    #[test]
    fn the_log_line_counts_twins_only_when_there_are_some() {
        let l = lines(&[&[1, 2], &[3]]);
        let p = plan_paragraph_edit(&["a", "b"], &[false, false], &["A", "b"], false);
        assert!(!super::plan_log_line(2, &p, &l, &[]).contains("twins"));
        let said = super::plan_log_line(2, &p, &l, &[vec![7, 8], vec![9]]);
        assert!(said.contains("taken off the page (and 2 twins)"), "{said}");
    }

    // -- keeping a justified line as wide as it was --------------------------

    /// Lines of a given width: `(left, right)` per line.
    fn wide(edges: &[(f32, f32)]) -> Vec<(Vec<usize>, Rect)> {
        edges
            .iter()
            .enumerate()
            .map(|(i, (left, right))| (vec![i], Rect { left: *left, top: 0.0, right: *right, bottom: 1.0 }))
            .collect()
    }

    /// Justified is told from left-aligned by where the lines end: the real
    /// paragraphs' edges (the datasheet's, and CAMINO's with a few lines 3 pt
    /// short) are at one margin; a ragged-right paragraph's are not.
    #[test]
    fn a_justified_paragraph_ends_at_one_margin_and_a_ragged_one_does_not() {
        // The datasheet's 13-line paragraph: right edges within 0.7 pt, last line short.
        let datasheet = wide(&[
            (187.0, 305.58), (187.8, 305.9), (187.0, 305.81), (187.8, 305.29), (187.8, 305.59), (187.8, 305.42),
            (187.3, 305.84), (187.3, 305.83), (187.4, 305.32), (187.8, 305.72), (187.8, 305.59), (187.7, 305.95),
            (187.1, 234.91),
        ]);
        assert!(ends_at_one_margin(&datasheet));
        // CAMINO's: a few lines end 3.2 pt short of 313.67, the rest at it.
        let camino = wide(&[
            (194.6, 310.47), (195.4, 313.66), (194.6, 313.67), (195.4, 313.67), (195.4, 310.47), (195.4, 313.67),
            (194.9, 313.67), (194.9, 313.67), (195.0, 313.67), (195.4, 313.67), (195.4, 310.71), (195.3, 313.67),
            (194.7, 242.42),
        ]);
        assert!(ends_at_one_margin(&camino));
        // Left-aligned text: the same left edge, ends all over the place.
        let ragged = wide(&[(100.0, 300.0), (100.0, 240.0), (100.0, 285.0), (100.0, 210.0), (100.0, 260.0), (100.0, 150.0)]);
        assert!(!ends_at_one_margin(&ragged));
    }

    /// The tolerance is a few points, not none: justified lines end a point or
    /// three apart (a hyphen, a trailing space), and all of them still count.
    #[test]
    fn lines_that_end_a_few_points_apart_still_end_at_one_margin() {
        let near = wide(&[
            (0.0, 313.6), (0.0, 311.5), (0.0, 312.9), (0.0, 310.8), (0.0, 313.67), (0.0, 312.2), (0.0, 80.0),
        ]);
        assert!(ends_at_one_margin(&near));
        // Four points is the edge; five is not.
        assert!(ends_at_one_margin(&wide(&[(0.0, 100.0), (0.0, 96.0), (0.0, 100.0), (0.0, 20.0)])));
        assert!(!ends_at_one_margin(&wide(&[(0.0, 100.0), (0.0, 95.0), (0.0, 95.0), (0.0, 100.0), (0.0, 95.0), (0.0, 20.0)])));
    }

    /// Too little to go on: one line has no margin; one line above a last line
    /// counts as one.
    #[test]
    fn a_paragraph_of_one_or_two_lines_is_judged_on_what_there_is() {
        assert!(!ends_at_one_margin(&[]));
        assert!(!ends_at_one_margin(&wide(&[(0.0, 100.0)])));
        assert!(ends_at_one_margin(&wide(&[(0.0, 100.0), (0.0, 40.0)])));
    }

    /// **The width reaches the engine**: each retyped, non-last line of a
    /// justified paragraph carries the width its original had in `justify_to`;
    /// the last line, a kept line, a ragged paragraph's lines and a `Remove`
    /// carry none. (And `justified: false` asks for none anywhere.)
    #[test]
    fn a_retyped_line_carries_the_width_it_is_to_span_in_justify_to() {
        let l = wide(&[(100.0, 310.0), (100.0, 312.5), (100.0, 308.0), (100.0, 190.0)]);
        let fates = [Written("a"), Kept, Written("c"), Written("d")];
        let justify_of = |edits: &[TextLineEdit]| -> Vec<Option<f32>> {
            edits
                .iter()
                .map(|edit| match edit {
                    TextLineEdit::Retype { justify_to, .. } => *justify_to,
                    TextLineEdit::Remove { .. } => panic!("no removal here"),
                })
                .collect()
        };
        let justified = line_edits(&fates, &l, &TextStyle::default(), true);
        assert_eq!(justify_of(&justified), [Some(210.0), Some(208.0), None], "lines 0 and 2 are asked; the last line is not");
        let ragged = line_edits(&fates, &l, &TextStyle::default(), false);
        assert_eq!(justify_of(&ragged), [None, None, None]);

        // A removed line says nothing about width.
        let with_removal = line_edits(&[Removed, Written("b"), Kept, Kept], &l, &TextStyle::default(), true);
        assert!(matches!(&with_removal[0], TextLineEdit::Remove { .. }));
        assert!(matches!(&with_removal[1], TextLineEdit::Retype { justify_to: Some(w), .. } if (*w - 212.5).abs() < 1e-3));
    }

    /// Whether a refusal might be the stretching's doing is told from the commands
    /// themselves: a retype that asks for a width, and nothing else, does.
    #[test]
    fn only_a_command_that_asks_for_a_width_can_have_been_refused_for_it() {
        let l = wide(&[(100.0, 310.0), (100.0, 312.5), (100.0, 190.0)]);
        let fates = [Written("a"), Written("b"), Written("c")];
        let asking = commands_for(0, vec![], super::line_edits(&fates, &l, &[], &TextStyle::default(), true));
        assert!(asks_for_width(&asking));
        let ragged = commands_for(0, vec![], super::line_edits(&fates, &l, &[], &TextStyle::default(), false));
        assert!(!asks_for_width(&ragged));
        // The closing line alone is never asked, so a paragraph whose only change is
        // there asks for nothing.
        let last_only = commands_for(
            0,
            vec![],
            super::line_edits(&[Kept, Kept, Written("c")], &l, &[], &TextStyle::default(), true),
        );
        assert!(!asks_for_width(&last_only));
        assert!(!asks_for_width(&[]));
    }

    /// A retyped line of a justified paragraph is asked for the width the
    /// original line had — right minus left of its box — and nothing else is.
    #[test]
    fn a_retyped_line_of_a_justified_paragraph_is_asked_for_the_width_it_had() {
        let l = wide(&[(100.0, 310.0), (100.0, 312.5), (100.0, 308.0), (100.0, 190.0)]);
        let fates = [Kept, Written("x"), Written("y"), Written("z")];
        assert_eq!(justify_targets(&fates, &l, true), [None, Some(212.5), Some(208.0), None]);
    }

    /// The paragraph's last line is never stretched, a ragged paragraph's lines
    /// never are, and a line that is not retyped is not mentioned.
    #[test]
    fn only_a_retyped_line_that_is_not_the_last_of_a_justified_paragraph_is_stretched() {
        let l = wide(&[(0.0, 100.0), (0.0, 100.0), (0.0, 60.0)]);
        // The last line, retyped: no.
        assert_eq!(justify_targets(&[Kept, Kept, Written("a")], &l, true), [None, None, None]);
        // A ragged paragraph: no.
        assert_eq!(justify_targets(&[Written("a"), Written("b"), Written("c")], &l, false), [None, None, None]);
        // Removed, frozen and kept lines: no.
        assert_eq!(justify_targets(&[Removed, Frozen, Kept], &l, true), [None, None, None]);
        // A line retyped in a justified paragraph: yes.
        assert_eq!(justify_targets(&[Written("a"), Kept, Kept], &l, true), [Some(100.0), None, None]);
    }

    /// "Last" is the last of the original lines: when it is removed the line
    /// above it was full width and is kept full width.
    #[test]
    fn a_removed_last_line_does_not_make_the_one_above_it_a_last_line() {
        let l = wide(&[(0.0, 100.0), (0.0, 100.0), (0.0, 50.0)]);
        assert_eq!(justify_targets(&[Kept, Written("b"), Removed], &l, true), [None, Some(100.0), None]);
    }

    /// A single line is its own last line; and the width is a width whichever
    /// way round the box's edges were given.
    #[test]
    fn a_single_line_is_never_stretched_and_the_width_is_never_negative() {
        assert_eq!(justify_targets(&[Written("a")], &wide(&[(0.0, 100.0)]), true), [None]);
        let flipped = wide(&[(100.0, 0.0), (0.0, 100.0)]);
        assert_eq!(justify_targets(&[Written("a"), Kept], &flipped, true), [Some(100.0), None]);
        assert!(justify_targets(&[], &[], true).is_empty());
    }

    // -- the paragraph's colour ----------------------------------------------

    /// A new colour for the whole paragraph reaches every piece that stays:
    /// all of a kept line, the first piece of a retyped one — not the pieces
    /// about to be removed, a removed line, or a line drawn as shapes.
    #[test]
    fn a_new_colour_reaches_every_piece_that_stays_and_none_that_goes() {
        let l = lines(&[&[1, 2], &[3, 4], &[5, 6], &[7], &[8, 9]]);
        let fates = [Kept, Written("x"), Removed, Frozen, Kept];
        assert_eq!(recolour_targets(&fates, &l), [1, 2, 3, 8, 9]);
        assert!(recolour_targets(&[Frozen, Removed], &lines(&[&[1], &[2]])).is_empty());
    }

    // -- what is executed, and in which order --------------------------------

    fn colour_edit(object: usize) -> (usize, String, TextStyle) {
        (
            object,
            "words".to_string(),
            TextStyle { color: Some(Color { r: 1, g: 2, b: 3, a: 255 }), ..Default::default() },
        )
    }

    /// **The colour first, then the replace**: the replace renumbers the page,
    /// so a colour that named objects after it would land on the wrong ones.
    #[test]
    fn the_colour_is_executed_before_the_replace_which_renumbers_the_page() {
        let both = commands_for(2, vec![colour_edit(5)], vec![TextLineEdit::Remove { objects: vec![7] }]);
        assert_eq!(both.len(), 2);
        assert!(matches!(&both[0], Command::SetTextRuns { page_index: 2, edits } if edits.len() == 1));
        assert!(matches!(&both[1], Command::ReplaceTextLines { page_index: 2, edits } if edits.len() == 1));
    }

    #[test]
    fn with_nothing_to_colour_or_replace_nothing_is_executed() {
        assert!(commands_for(0, vec![], vec![]).is_empty());
        let only_colour = commands_for(0, vec![colour_edit(1)], vec![]);
        assert!(matches!(only_colour.as_slice(), [Command::SetTextRuns { .. }]));
        let only_replace = commands_for(0, vec![], vec![TextLineEdit::Remove { objects: vec![1] }]);
        assert!(matches!(only_replace.as_slice(), [Command::ReplaceTextLines { .. }]));
    }

    // -- refusing a paragraph that does not line up --------------------------

    #[test]
    fn a_paragraph_that_lines_up_is_accepted() {
        assert!(check_lines_up(3, &lines(&[&[1], &[2, 3], &[]]), &[false, false, true]).is_ok());
    }

    #[test]
    fn a_paragraph_that_no_longer_lines_up_is_refused_for_each_way_of_not_lining_up() {
        let ok = lines(&[&[1], &[2]]);
        for (what, text_lines, ls, frozen) in [
            ("no lines at all", 0, lines(&[]), vec![]),
            ("a line of text too few", 1, ok.clone(), vec![false, false]),
            ("a line of text too many", 3, ok.clone(), vec![false, false]),
            ("a frozen flag too few", 2, ok.clone(), vec![false]),
            ("a line that is not frozen and has no object", 2, lines(&[&[1], &[]]), vec![false, false]),
            ("an object on two lines", 2, lines(&[&[1, 2], &[2]]), vec![false, false]),
        ] {
            let refused = check_lines_up(text_lines, &ls, &frozen)
                .err()
                .unwrap_or_else(|| panic!("{what} was not refused"));
            assert!(refused.contains("no longer lines up"), "{what}: {refused}");
            assert!(refused.contains("nothing was changed"), "{what}: {refused}");
        }
    }

    // -- what is said --------------------------------------------------------

    #[test]
    fn the_log_line_names_lines_by_index_and_never_by_their_words() {
        let original = ["secret price 100", "second line", "third line", "drawn", "kept"];
        let typed = ["secret price 999", "second line", "third line", "drawn", "kept"];
        let l = lines(&[&[1, 2, 3], &[4], &[5], &[], &[6, 7]]);
        let p = plan_paragraph_edit(&original, &[false, false, false, true, false], &typed, false);
        let line = plan_log_line(typed.len(), &p, &l);
        assert_eq!(
            line,
            "paragraph diff: 5 original lines, 5 typed lines, prefix 0, suffix 4, retyped lines [0], \
             removed lines [], 2 pieces taken off the page, frozen lines [3] (0 of them typed over), \
             unchanged lines 3"
        );
        for word in ["secret", "price", "999", "second", "third", "drawn"] {
            assert!(!line.contains(word), "the log line carries page text {word:?}: {line}");
        }
    }

    #[test]
    fn the_log_line_counts_the_pieces_a_removed_line_loses() {
        let l = lines(&[&[1, 2], &[3, 4, 5]]);
        let p = plan_paragraph_edit(&["a", "b"], &[false, false], &["a"], false);
        let line = plan_log_line(1, &p, &l);
        assert!(line.contains("removed lines [1], 3 pieces taken off the page"), "{line}");
    }

    #[test]
    fn the_apply_message_names_frozen_lines_only_when_they_were_typed_over() {
        assert_eq!(paragraph_applied_message(false, 0, false), "paragraph changed.");
        assert_eq!(paragraph_applied_message(true, 0, false), "unchanged.");
        assert_eq!(
            paragraph_applied_message(false, 1, false),
            "paragraph changed; 1 line drawn as shapes was left as it is."
        );
        assert_eq!(
            paragraph_applied_message(false, 3, false),
            "paragraph changed; 3 lines drawn as shapes were left as they are."
        );
        assert_eq!(
            paragraph_applied_message(true, 2, false),
            "unchanged; 2 lines drawn as shapes were left as they are."
        );
    }

    /// A position asked for is never silently dropped.
    #[test]
    fn a_position_that_could_not_be_applied_is_said() {
        assert_eq!(
            paragraph_applied_message(false, 0, true),
            "paragraph changed; the position was not changed — a paragraph has no single place to move to."
        );
        assert!(paragraph_applied_message(true, 1, true).contains("1 line drawn as shapes"));
        assert!(paragraph_applied_message(true, 1, true).contains("the position was not changed"));
    }
}
/// Whether a picked paragraph's non-last lines should be stretched to the
/// box's own width — see [`PagifyApp::draw_run_editor`]'s own call site.
///
/// Not simply "more than one line": a label-above-an-indented-value field —
/// "Current Input: ..." over an indented "1050mA" — merges into a 2-line
/// "paragraph" by the same geometry a real wrapped paragraph does, but its
/// second line starts well to the right of the first, not flush against the
/// margin every line of a real wrap shares. Stretching its one real line to
/// the box's own width invents a look the page never had. **Reported from
/// use**, alongside the paragraph misdetection itself: a field like this one
/// showed visibly gapped spacing ("Respectively  for  power") that the real
/// page never had. The tolerance absorbs ordinary floating-point noise
/// between runs extracted from the same left-aligned block, not a real
/// indent.
pub fn paragraph_should_justify(lines: &[(Vec<usize>, pdf_core::document::Rect)]) -> bool {
    lines.len() > 1
        && lines
            .windows(2)
            .all(|w| (w[0].1.left.min(w[0].1.right) - w[1].1.left.min(w[1].1.right)).abs() <= 2.0)
}

/// How much extra space to insert before each word of a justified line, so
/// its natural width stretches to fill `target_width` — real justification
/// (every gap gets an equal share of the shortfall), not an approximation.
///
/// `word_widths` is each word's own already-measured width, left to right,
/// in the same units as `target_width`. A line of fewer than two words has
/// no gap to stretch and is returned unchanged (every entry `0.0`); a line
/// that already reaches or exceeds the target is left alone too — this
/// only ever adds space, never removes it by compressing a word.
///
/// **Reported from use, twice: the run editor's own paragraph box showed a
/// ragged right edge where the real page showed the same paragraph fully
/// justified**, on top of everything else about the box that had already
/// been made to match. This is the one piece of that look `egui::TextEdit`
/// has no setting for — `LayoutJob::justify` exists, but it only stretches
/// rows *it* wrapped, and every one of this editor's lines already ends in
/// an explicit `\n` (one object per line, not a reflowed paragraph), which
/// is exactly the case that built-in flag deliberately leaves alone. So the
/// stretch is computed by hand instead, one line at a time, and applied as
/// `leading_space` — see `draw_run_editor`'s own layouter.
pub fn justify_gaps(word_widths: &[f32], target_width: f32) -> Vec<f32> {
    if word_widths.len() < 2 {
        return vec![0.0; word_widths.len()];
    }
    let natural: f32 = word_widths.iter().sum();
    let deficit = (target_width - natural).max(0.0);
    let extra_per_gap = deficit / (word_widths.len() - 1) as f32;
    std::iter::once(0.0).chain(std::iter::repeat(extra_per_gap).take(word_widths.len() - 1)).collect()
}

/// Joins a paragraph's own lines back into one string, putting back the
/// hyphen a wrapped word shows on the page but the text layer does not carry.
///
/// `hyphen_after[i]` says the page draws a hyphen mark at the end of line `i`
/// — see [`wrap_hyphen_marks`]; an entry that is missing reads as `false`.
///
/// **Reported from use, with a screenshot: the real page reads
/// "light-\ning" and "dis-\nsipation", the editor read "light\ning" and
/// "dis\nsipation" — no hyphen at all, not even a broken one.** Checked
/// directly against the file: the run before the break is `"...light"`,
/// the run after is `"ing..."`, with nothing — not a character, not a
/// control code — between them in the extracted text. `fix_extracted_text`
/// only ever repairs a character that is *there*; this producer draws its
/// wrap-hyphen as its own small mark rather than a glyph, so the text layer
/// never carried one to repair.
///
/// **Only where the page shows one — never because two lines happen to
/// meet mid-word.** The first fix judged by the shape of the break alone:
/// letters touching on both sides, so a hyphen goes in. That is true of this
/// producer's hyphenated wraps and equally of every ordinary wrap on a page
/// whose lines never end in a space — an Illustrator export's justified
/// columns end every line on a letter, and every such join grew a hyphen the
/// page never drew (764 of 3,930 joins in a census of real pages). Worse than
/// the look of the box: applying an edit rewrites every line of the paragraph
/// from the buffer, so each invented "-" was typed into the page as real
/// text, on lines nobody had touched — see `wrap_hyphen_tests`.
///
/// So the evidence has to be on the page: the line's own text already
/// carries the hyphen (a real one, or the U+0002 PDFium reads one back as —
/// `fix_extracted_text` turns that into "-"), or a hyphen mark is drawn right
/// after the line and `hyphen_after` says so. Letters on both sides of the
/// break are still required of a *drawn* mark: it has to be splitting a word.
pub fn join_paragraph_lines(lines: &[String], hyphen_after: &[bool]) -> String {
    let mut combined = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            // `alphanumeric` on the line above also means "does not already
            // end in a hyphen or a U+0002", so a mark drawn after a hyphen the
            // text carries cannot double it.
            let drawn_hyphen = hyphen_after.get(i - 1).copied().unwrap_or(false)
                && combined.chars().next_back().is_some_and(char::is_alphanumeric)
                && line.chars().next().is_some_and(char::is_alphanumeric);
            if drawn_hyphen {
                combined.push('-');
            }
            combined.push('\n');
        }
        combined.push_str(line);
    }
    combined
}

/// Where a wrapped line ends, as far as spotting a hyphen drawn after it
/// goes: the right edge of its last glyph, its baseline (page points, top-left
/// origin, so `y` grows downward) and the size it is set in. A hyphen mark is
/// described in ems of that size, not in points, so one rule serves a 7 pt
/// datasheet and a 40 pt heading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineEnd {
    pub right: f32,
    pub baseline: f32,
    pub em: f32,
}

/// Whether `shape` is a hyphen mark standing at the end of the line `end`
/// describes: a short, thin, horizontal drawn shape, at about the height a
/// hyphen sits, starting just right of the line's last glyph.
///
/// Every number is in ems and deliberately loose — a mark is whatever the
/// producer's own hyphen glyph looks like once converted to a path — while
/// still ruling out what else lives at the end of a line: an underline is
/// below the baseline, a rule or a strike-through is long or starts inside the
/// last word, a full stop is as tall as it is wide, an outlined letter is far
/// taller than 0.2 em.
pub fn is_hyphen_mark(shape: &pdf_core::document::DrawnObject, end: LineEnd) -> bool {
    if shape.kind != pdf_core::document::DrawnKind::Shape || end.em <= 0.0 {
        return false;
    }
    let r = &shape.rect;
    let (left, right) = (r.left.min(r.right), r.left.max(r.right));
    let (top, bottom) = (r.top.min(r.bottom), r.top.max(r.bottom));
    let (width, height) = (right - left, bottom - top);
    let em = end.em;
    // Up from the baseline — the page's `y` grows the other way.
    let centre_above_baseline = end.baseline - (top + bottom) / 2.0;
    let gap_after_last_glyph = left - end.right;
    (0.1 * em..=0.6 * em).contains(&width)
        && height <= 0.2 * em
        && width >= 1.5 * height
        && (0.1 * em..=0.65 * em).contains(&centre_above_baseline)
        && (-0.15 * em..=0.4 * em).contains(&gap_after_last_glyph)
}

/// For every line of a paragraph, whether the page draws a hyphen mark right
/// after it — the evidence [`join_paragraph_lines`] needs before it puts a
/// "-" at that wrap. One entry per line; `None` for a line whose geometry is
/// not known, which has no mark to find. The last line's entry is never
/// consulted, there being no wrap after it.
pub fn wrap_hyphen_marks(ends: &[Option<LineEnd>], shapes: &[pdf_core::document::DrawnObject]) -> Vec<bool> {
    ends.iter()
        .map(|end| end.is_some_and(|end| shapes.iter().any(|shape| is_hyphen_mark(shape, end))))
        .collect()
}

/// Repair characters a font has no glyph for and no font ever will — real
/// control codes, not real text — before they ever reach the run editor's
/// buffer.
///
/// **Reported from use, with a screenshot: a word mid-paragraph rendered
/// with what looked like the font suddenly changing.** It was a `\u{2}`
/// (STX) sitting where the source page draws a hyphen — this PDF's own
/// `ToUnicode` mapping for its hyphen glyph resolves to a control code
/// rather than `-`, a defect in the file's own text. No installed font has
/// a real glyph for a control character, so egui fell back to a
/// *different* font's own placeholder box for that one character — which
/// is exactly what "the font changed" looks like from the outside.
///
/// **A control character sitting between two letters is put back as a
/// hyphen, not dropped.** Reported a second time, with a screenshot: the
/// first fix dropped the character outright, which fixed the tofu box but
/// silently turned "elitee-plus" into "eliteeplus" wherever that same
/// mapping bug landed on the product name's own hyphen rather than on a
/// line-wrap. A hyphen is overwhelmingly the most common glyph a broken
/// `ToUnicode` table mismaps this way, and a letter on both sides is
/// exactly the shape a real hyphen — not an en dash, not a bullet, not
/// nothing — leaves. Anywhere else (start of a line, next to a digit,
/// next to another control character), there is no such signal, and the
/// character is dropped rather than guessed at.
///
/// **Looks past a `\n` on either side, not just the immediately adjacent
/// character.** A hyphen can fall exactly on a line wrap — the ordinary
/// place one occurs — where the character actually touching it is the
/// newline itself and the letter is one further away; a paragraph's own
/// lines are expected to already be joined into one string by the time
/// this runs, for exactly this reason.
pub fn fix_extracted_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        if c.is_control() && c != '\n' && c != '\t' {
            let prev = chars[..i].iter().rev().find(|p| **p != '\n');
            let next = chars[i + 1..].iter().find(|n| **n != '\n');
            let between_letters =
                prev.is_some_and(|p| p.is_alphabetic()) && next.is_some_and(|n| n.is_alphabetic());
            if between_letters {
                out.push('-');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// **A line-end hyphen the page's text carries as a control code stays a
/// hyphen when the line after it is one the page draws as shapes.**
/// [`fix_extracted_text`] puts such a code back as "-" only between two
/// letters, looking past the line break, and drops it anywhere else; before a
/// drawn line the next character is the placeholder's "[" (or, before a line
/// with a drawn word at its start, whatever text follows the gap), so the hyphen
/// the page visibly draws was dropped from the buffer — and retyping the line
/// then took it off the page. Reported by the pick-path census on the datasheet's
/// first paragraph ("driv" + U+0002, line 1, a drawn line after it).
///
/// What is known here is better than a letter test: the code is *the page's own
/// hyphen glyph*, it follows a letter, and the word it cuts goes on in words
/// nobody can read. So when line `i` is drawn (`drawn[i]`: a placeholder or a
/// line with a drawn word in it), a control code ending line `i - 1` after a
/// letter becomes "-" there. A code after anything but a letter, or before a line
/// that is written, is left to [`fix_extracted_text`] as before.
pub fn hyphens_before_drawn_lines(texts: &mut [String], drawn: &[bool]) {
    for i in 1..texts.len() {
        if !drawn.get(i).copied().unwrap_or(false) {
            continue;
        }
        let above = &mut texts[i - 1];
        let mut tail = above.char_indices().rev();
        let Some((at, last)) = tail.next() else { continue };
        let is_marker = last.is_control() && last != '\n' && last != '\t';
        if is_marker && tail.next().is_some_and(|(_, before)| before.is_alphabetic()) {
            above.replace_range(at.., "-");
        }
    }
}

/// Which of a paragraph's lines its *look* — the one font/size the run
/// editor's single `TextEdit` renders every line in — should be taken from.
///
/// Each entry is `(object, look, weight)` — one per text fragment, `look`
/// being whatever the caller decides makes two fragments look alike (so far
/// `(face name, size bits)`; a font identity once the caller has one) and
/// `weight` how much that fragment should count for. The winner is the look
/// with the largest total weight, ties broken by whichever appeared first;
/// the object returned is the first fragment that has that look. `None` only
/// when `fragments` is empty.
///
/// **Reported from use, with a screenshot**: a paragraph whose first line
/// was a bold "Description:" heading opened with its entire multi-line body
/// rendered in that same bold, oversized face, even though every line
/// beneath it was ordinary body text. `pick_paragraph` used to seed the
/// whole editor from `lines[0]` alone; this is what replaced it.
///
/// **Weighed by ink, not counted by fragment.** One vote per fragment let
/// a producer's chopping decide: a heading cut into five scraps outvotes a
/// body of four long lines, and a three-letter "HSI" in another weight
/// inside a paragraph is a fragment like any other. The caller passes each
/// fragment's non-space character count, so the look that most of the
/// *text* has wins, however many pieces it was written in. And when every
/// fragment shares one look — which is what a font name reported
/// identically for five different weights makes of a whole page — the
/// first-seen rule still gives the first fragment, as it always did.
pub fn majority_look<K: PartialEq>(fragments: &[(usize, K, usize)]) -> Option<usize> {
    // (look, total weight, first object with it)
    let mut tally: Vec<(&K, usize, usize)> = Vec::new();
    for (object, look, weight) in fragments {
        match tally.iter_mut().find(|(seen, ..)| *seen == look) {
            Some((_, total, _)) => *total += weight,
            None => tally.push((look, *weight, *object)),
        }
    }
    // Not `Iterator::max_by_key`: on a tie it keeps the *last* maximum, and
    // first-seen order is what makes `a_tie_resolves_to_whichever_look_
    // appeared_first` (and, in practice, a paragraph with no real majority)
    // deterministic in the more expected direction.
    let mut best: Option<(usize, usize)> = None; // (total weight, first object)
    for (_, total, first) in &tally {
        if best.map_or(true, |(most, _)| *total > most) {
            best = Some((*total, *first));
        }
    }
    best.map(|(_, first)| first)
}
