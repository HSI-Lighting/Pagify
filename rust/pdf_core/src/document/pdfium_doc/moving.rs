//! Moving or resizing many objects of one page at once: one read of the page,
//! one write.

use super::*;

/// One stream edit: the bytes to replace, and what goes there.
type Edit = (std::ops::Range<usize>, Vec<u8>);

/// What a group edit does to every member.
#[derive(Debug, Clone, Copy)]
pub(super) enum Change {
    /// Move by this much (page points, y down).
    Move(Point),
    /// Resize about `anchor` (page points, y down) by `sx` across, `sy` down.
    Scale { anchor: Point, sx: f32, sy: f32 },
}

impl PdfiumDocument {
    /// See [`crate::document::DocumentMut::move_objects_mut`].
    pub(super) fn move_objects_in_stream(&mut self, page_index: usize, objects: &[usize], by: Point) -> Result<()> {
        self.change_objects_in_stream(page_index, objects, Change::Move(by))
    }

    /// See [`crate::document::DocumentMut::scale_objects_mut`].
    pub(super) fn scale_objects_in_stream(
        &mut self,
        page_index: usize,
        objects: &[usize],
        anchor: Point,
        sx: f32,
        sy: f32,
    ) -> Result<()> {
        if !(sx.is_finite() && sy.is_finite()) || sx.abs() < 1e-3 || sy.abs() < 1e-3 {
            return Err(PdfError::InvalidArgument("that would resize it to nothing".into()));
        }
        self.change_objects_in_stream(page_index, objects, Change::Scale { anchor, sx, sy })
    }

    /// **Reported from use (0.1.78):** 96 things moved together froze the
    /// app, and on a low-spec machine it was closed as not responding. A group
    /// move was one `MoveObject` per member (a group resize one `ScaleObject`),
    /// and each of them serialised the whole document, parsed it, rewrote it
    /// and loaded it back into PDFium, with a few page-wide scans besides:
    /// measured on the VIVO lens, 43 ms a member, 4 s for 93 of them on the UI
    /// thread, and its undo as long again.
    ///
    /// Here every member's edits are planned against **one** read of the page,
    /// by the same code a single edit uses (`transform_edits` for pictures and
    /// shapes; `run_move_edits` and `resize_run_edits` for words), and written
    /// in one splice. **Checked before it is kept**: every member is where the
    /// edit puts it, everything else is where it was, and the words are the
    /// same. Anything that cannot be planned or checked is a refusal that
    /// changes nothing, and the command then edits the members one at a time,
    /// as a group always was.
    fn change_objects_in_stream(&mut self, page_index: usize, objects: &[usize], change: Change) -> Result<()> {
        use crate::pdf::content;
        use pdfium_render::prelude::{FPDF_PAGEOBJ_IMAGE, FPDF_PAGEOBJ_PATH, FPDF_PAGEOBJ_TEXT};

        let mut objects = objects.to_vec();
        objects.sort_unstable();
        objects.dedup();
        if objects.is_empty() {
            return Ok(());
        }
        self.validate_page_index(page_index)?;

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let file = crate::pdf::File::parse(&base.bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, streams) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let states = content::states(&operations);
        let placed = content::placed(&operations);
        let census = self.object_census(page_index)?;
        // Read once: what the words must still say afterwards, and each run's
        // place for planning its edit (`text_run_at` per run loads the page's
        // text again every time).
        let runs = self.text_runs(page_index)?;
        let words = words_of(&runs);
        let height = self.page_size(page_index)?.height_pt;

        // The same edit a single move or resize makes, as a matrix on the page
        // (y up) for pictures and shapes.
        let page_matrix = match change {
            Change::Move(by) => [1.0, 0.0, 0.0, 1.0, by.x, -by.y],
            Change::Scale { anchor, sx, sy } => {
                let (ax, ay) = (anchor.x, height - anchor.y);
                [sx, 0.0, 0.0, sy, ax * (1.0 - sx), ay * (1.0 - sy)]
            }
        };

        // **A clipped scope whose every object is moving moves whole**: one
        // `cm` just inside its `q`, before its clip, so the clip travels with
        // what it holds — the way a frame moves with its picture. Reported
        // from use (0.1.81, three screenshots): VIVO draws its lens inside
        // `q <frame> re W n … Q`, and a shift written per line, after the
        // clip, left the clip behind and everything taken out of the frame
        // was clipped away. Its members are not planned again below.
        let mut covered = std::collections::BTreeSet::new();
        let mut members = Vec::with_capacity(objects.len());
        if let Some(starts) = object_starts(&census, &operations, &self.image_names(&file, &page)?) {
            for (open, close) in clipping_scopes(&operations) {
                let inside: Vec<usize> = (0..census.len()).filter(|&i| open < starts[i] && starts[i] < close).collect();
                let all_moving = !inside.is_empty() && inside.iter().all(|i| objects.binary_search(i).is_ok());
                if !all_moving || inside.iter().any(|i| covered.contains(i)) {
                    continue;
                }
                let ctm = states[open].ctm;
                let back = inverse(ctm).ok_or(PdfError::Unsupported("this is placed by a transform this cannot invert"))?;
                let local = matrices(matrices(ctm, page_matrix), back);
                let at = operations[open].span.end;
                members.push(vec![(at..at, concat_matrix(&local)?)]);
                covered.extend(inside);
            }
        }

        // Each member planned by the kind the census already says it is:
        // asking PDFium "is this words?" for every member loaded the page's
        // text each time, 7 ms a member. Anything that cannot be planned goes
        // the slow way, which says why.
        let facts = PageFacts::new(self, page_index);
        for &object in objects.iter().filter(|o| !covered.contains(*o)) {
            let planned = match census.get(object).map(|d| d.kind as u32) {
                Some(FPDF_PAGEOBJ_IMAGE | FPDF_PAGEOBJ_PATH) => {
                    self.transform_edits(&facts, page_index, object, page_matrix, &file, &page, &operations, &states, &placed)
                }
                Some(FPDF_PAGEOBJ_TEXT) => {
                    // A run `text_runs` leaves out is asked for on its own.
                    let run = match runs.iter().find(|run| run.object == object) {
                        Some(run) => Some(run.clone()),
                        None => self.text_run_at(page_index, object)?,
                    };
                    match run {
                        Some(run) => match change {
                            Change::Move(by) => self.run_move_edits(
                                &facts, page_index, &run, by, &file, &page, &stream, &operations, &states, &placed,
                            ),
                            Change::Scale { anchor, sx, sy } => self.run_scale_edits(
                                &facts, page_index, &run, anchor, sx, sy, &file, &page, &stream, &operations, &states,
                                &placed,
                            ),
                        },
                        None => Err(PdfError::Unsupported("")),
                    }
                }
                _ => Err(PdfError::Unsupported("")),
            };
            members.push(planned.map_err(|_| {
                PdfError::Unsupported("one of these is drawn in a way that cannot be changed with the others at once")
            })?);
        }
        let edited = content::splice(&stream, &in_stream_order(members)?);
        let rewritten = Self::write_edit(&base, &file, &stream_replacements(&streams, &edited)?, &[])?;

        let after = Self::open_bytes(rewritten.clone(), None)?;
        let now = after.object_census(page_index)?;
        let near = |a: f32, b: f32| (a - b).abs() <= 0.5;
        let each_where_it_should_be = now.len() == census.len()
            && census.iter().zip(&now).enumerate().all(|(index, (was, is))| {
                let same_look = was.kind == is.kind && was.fill == is.fill && was.stroke == is.stroke;
                let (r, n) = (was.rect, is.rect);
                let placed_right = if objects.binary_search(&index).is_err() {
                    near(n.left, r.left) && near(n.top, r.top) && near(n.right, r.right) && near(n.bottom, r.bottom)
                } else {
                    match change {
                        Change::Move(by) => {
                            near(n.left, r.left + by.x)
                                && near(n.top, r.top + by.y)
                                && near(n.right, r.right + by.x)
                                && near(n.bottom, r.bottom + by.y)
                        }
                        Change::Scale { anchor, sx, sy } => {
                            let x = |v: f32| anchor.x + (v - anchor.x) * sx;
                            let y = |v: f32| anchor.y + (v - anchor.y) * sy;
                            // Words are resized by their font size, not
                            // geometrically: where they start is what a
                            // single resize puts right, not their far side.
                            let corner = near(n.left, x(r.left)) && near(n.top, y(r.top));
                            corner
                                && (was.kind as u32 == FPDF_PAGEOBJ_TEXT
                                    || (near(n.right, x(r.right)) && near(n.bottom, y(r.bottom))))
                        }
                    }
                };
                same_look && placed_right
            });
        if !each_where_it_should_be {
            return Err(PdfError::Unsupported("changing these together would not change each of them as changing it alone does"));
        }
        if words_of(&after.text_runs(page_index)?) != words {
            return Err(PdfError::Unsupported("changing these together would change words on the page"));
        }
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    /// The edits a single resize makes to one run (`scale_object`): its size
    /// changed by a `Tf`/`Tz` (`resize_run_edits`), then moved so it sits
    /// where a resize about `anchor` puts it.
    ///
    /// A single resize measured the run after resizing it and moved it by the
    /// difference — a second whole-document rewrite. Measuring is not needed:
    /// a font size and a horizontal scale grow the run about its own origin,
    /// and scaling about the origin then moving by `(anchor − origin)(1 − s)`
    /// is scaling about the anchor.
    #[allow(clippy::too_many_arguments)]
    fn run_scale_edits(
        &self,
        facts: &PageFacts<'_>,
        page_index: usize,
        run: &crate::document::TextRun,
        anchor: Point,
        sx: f32,
        sy: f32,
        file: &crate::pdf::File<'_>,
        page: &crate::pdf::Object,
        stream: &[u8],
        operations: &[crate::pdf::content::Operation],
        states: &[crate::pdf::content::State],
        placed: &[crate::pdf::content::Placed],
    ) -> Result<Vec<Edit>> {
        let resize = self.resize_run_edits(facts, page_index, run, sx, sy, file, page, operations, states, placed)?;
        let shift = Point { x: (anchor.x - run.origin.x) * (1.0 - sx), y: (anchor.y - run.origin.y) * (1.0 - sy) };
        if shift.x.abs() <= 0.01 && shift.y.abs() <= 0.01 {
            return Ok(resize);
        }
        let shifted = self.run_move_edits(facts, page_index, run, shift, file, page, stream, operations, states, placed)?;

        // Where the two write at the same place, the move's go outside the
        // resize's: in front of the run before the new size is set, after it
        // once the old size is back — a pen-moving kern is measured in the
        // run's own size, and must be drawn at it.
        let start = resize.iter().map(|(span, _)| span.start).min().unwrap_or(0);
        let mut ranked: Vec<(usize, u8, Edit)> = Vec::new();
        for edit in shifted {
            let rank = if edit.0.start <= start { 0 } else { 1 };
            ranked.push((edit.0.start, rank, edit));
        }
        for edit in resize {
            let rank = if edit.0.start <= start { 1 } else { 0 };
            ranked.push((edit.0.start, rank, edit));
        }
        ranked.sort_by_key(|(at, rank, _)| (*at, *rank));
        Ok(ranked.into_iter().map(|(_, _, edit)| edit).collect())
    }
}

/// The words of a page's runs, in order, as the check compares them.
///
/// **A line-end hyphen is a hyphen.** PDFium reads a hyphen at the end of a
/// line as U+0002, and once moved with its words it is no longer at a line
/// end and reads as "-": the same mark on the page, and the check refused
/// every group move that held one — reported from use as 234 things taking
/// 11 s on 0.1.79. And spaces are not compared: PDFium infers them from the
/// gaps between letters, which a move of some words and not others changes.
fn words_of(runs: &[crate::document::TextRun]) -> Vec<String> {
    runs.iter().map(|run| word(&run.text)).collect()
}

fn word(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).map(|c| if c == '\u{2}' { '-' } else { c }).collect()
}

/// The `q … Q` scopes that set a clipping path at their own level, as the
/// operations that open and close them, outermost first.
fn clipping_scopes(operations: &[crate::pdf::content::Operation]) -> Vec<(usize, usize)> {
    let mut open: Vec<(usize, bool)> = Vec::new();
    let mut scopes = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        match operation.operator.as_slice() {
            b"q" => open.push((index, false)),
            b"Q" => {
                if let Some((at, clips)) = open.pop() {
                    if clips {
                        scopes.push((at, index));
                    }
                }
            }
            b"W" | b"W*" => {
                if let Some(level) = open.last_mut() {
                    level.1 = true;
                }
            }
            _ => {}
        }
    }
    scopes.sort_by_key(|&(at, end)| (at, std::cmp::Reverse(end)));
    scopes
}

/// The operation each object of the page starts at, in census order — or
/// `None` where that cannot be told for every one of them. Pictures, shapes,
/// groups and shadings as `drawing_spans` matches them; words as the *n*th
/// operation that draws glyphs, once there are as many as text objects.
fn object_starts(
    census: &[Drawn],
    operations: &[crate::pdf::content::Operation],
    images: &std::collections::BTreeSet<Vec<u8>>,
) -> Option<Vec<usize>> {
    use pdfium_render::prelude::FPDF_PAGEOBJ_TEXT;
    let spans = super::lift::drawing_spans(census, operations, images);
    let shows: Vec<usize> =
        operations.iter().enumerate().filter(|(_, op)| op.draws_glyphs()).map(|(index, _)| index).collect();
    let words = census.iter().filter(|d| d.kind as u32 == FPDF_PAGEOBJ_TEXT).count();
    if words != shows.len() {
        return None;
    }
    let mut shows = shows.into_iter();
    census
        .iter()
        .enumerate()
        .map(|(index, drawn)| {
            if drawn.kind as u32 == FPDF_PAGEOBJ_TEXT { shows.next() } else { spans[index].map(|(first, _, _)| first) }
        })
        .collect()
}

/// Every member's edits, members in stream order, as one list for one splice.
///
/// Two members whose edits overlap refuse: one is drawn inside the other's
/// unit (a picture's placeholder with the picture), and changing them one at
/// a time is what keeps that right.
fn in_stream_order(members: Vec<Vec<Edit>>) -> Result<Vec<Edit>> {
    let mut members: Vec<(std::ops::Range<usize>, Vec<Edit>)> = members
        .into_iter()
        .filter(|edits| !edits.is_empty())
        .map(|edits| {
            let start = edits.iter().map(|(span, _)| span.start).min().unwrap_or(0);
            let end = edits.iter().map(|(span, _)| span.end).max().unwrap_or(0);
            (start..end, edits)
        })
        .collect();
    members.sort_by_key(|(extent, _)| extent.start);

    let mut out = Vec::new();
    let mut reach = 0;
    for (index, (extent, edits)) in members.into_iter().enumerate() {
        if index > 0 && extent.start < reach {
            return Err(PdfError::Unsupported("two of these are drawn into one another"));
        }
        out.extend(edits);
        reach = extent.end;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{in_stream_order, word};

    fn insert(at: usize, what: &str) -> super::Edit {
        (at..at, what.as_bytes().to_vec())
    }

    #[test]
    fn members_are_put_in_stream_order() {
        let first = vec![insert(10, "q"), insert(50, "Q")];
        let later = vec![insert(60, "q"), insert(70, "Q")];
        let order = in_stream_order(vec![later.clone(), first.clone()]).expect("order");
        assert_eq!(order, [first, later].concat());
    }

    #[test]
    fn members_that_overlap_refuse() {
        let outer = vec![insert(10, "q"), insert(50, "Q")];
        let inner = vec![insert(20, "q"), insert(30, "Q")];
        let crossing = vec![insert(40, "q"), insert(60, "Q")];
        assert!(in_stream_order(vec![outer.clone(), inner]).is_err());
        assert!(in_stream_order(vec![outer, crossing]).is_err());
    }

    /// One member's end and the next one's start at the same byte: the end
    /// comes first, or the next member's `q` would open before this one's `Q`.
    #[test]
    fn a_member_ending_where_the_next_begins_is_closed_first() {
        let a = vec![insert(10, "q"), insert(20, "Q")];
        let b = vec![insert(20, "q2"), insert(30, "Q2")];
        let order = in_stream_order(vec![b.clone(), a.clone()]).expect("order");
        assert_eq!(order, [a, b].concat());
    }

    #[test]
    fn a_line_end_hyphen_and_spaces_are_not_a_change_of_words() {
        assert_eq!(word("driv\u{2}"), word("driv-"));
        assert_eq!(word("Power  Input: "), word("PowerInput:"));
        assert_ne!(word("Power"), word("Tower"));
    }
}
