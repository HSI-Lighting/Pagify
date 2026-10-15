//! Lifting drawn objects off a page, and pasting them into another as lines of its own.
//!
//! A child of `pdfium_doc`, so the stream-following helpers that restack and
//! remove already trust are used as they are, without widening them. The
//! pure half (resources, graph copy, the form itself) is `crate::pdf::lift`.

use super::*;

impl PdfiumDocument {
    /// The matrix taking the engine's top-left page space (census rects and
    /// `Point`s) to PDF user space, and the page's `/Rotate` in quarter turns.
    ///
    /// Page space is the crop box's top-left with y down (`PageSpace`), and
    /// does not include `/Rotate`: the turn is returned beside it.
    pub(super) fn page_to_user(&self, page_index: usize) -> Result<([f32; 6], u8)> {
        let page_number = i32::try_from(page_index)
            .map_err(|_| PdfError::PageOutOfRange { index: page_index, count: self.page_count })?;
        let space = RawPage::open(self.document.handle(), page_number)?.space()?;
        Ok(([1.0, 0.0, 0.0, -1.0, space.left, space.top], self.page_rotation(page_index)?))
    }

    #[doc(hidden)]
    pub fn page_to_user_for_probe(&self, page_index: usize) -> Result<([f32; 6], u8)> {
        self.page_to_user(page_index)
    }

    /// A rectangle in page space where the page shows it: in points from the
    /// top-left of the page as rendered, turned by its `/Rotate`.
    ///
    /// Through PDF space and PDFium's own page box, which follows inheritance,
    /// rather than from `PageSpace` alone: a turned page whose MediaBox is on
    /// its page tree has a `PageSpace` measured from its turned height (see
    /// `crate::pdf::lift::one_page_file`), and this has to find the pixels
    /// either way.
    pub(super) fn displayed(&self, page_index: usize, r: Rect) -> Result<Rect> {
        let (to_user, turns) = self.page_to_user(page_index)?;
        let page_number = i32::try_from(page_index)
            .map_err(|_| PdfError::PageOutOfRange { index: page_index, count: self.page_count })?;
        let raw = RawPage::open(self.document.handle(), page_number)?;
        let mut b = FS_RECTF { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };
        if unsafe { pdfium()?.bindings().FPDF_GetPageBoundingBox(raw.handle, &mut b) } == 0 {
            return Err(PdfError::Unsupported("this page's box cannot be read"));
        }
        let (w, h) = (b.right - b.left, b.top - b.bottom);
        let shown = |x: f32, y: f32| {
            let (ux, uy) = (to_user[0] * x + to_user[2] * y + to_user[4], to_user[1] * x + to_user[3] * y + to_user[5]);
            let (u, v) = (ux - b.left, b.top - uy);
            match turns % 4 {
                0 => (u, v),
                1 => (h - v, u),
                2 => (w - u, h - v),
                _ => (v, w - u),
            }
        };
        let (p, q) = (shown(r.left, r.top), shown(r.right, r.bottom));
        Ok(Rect { left: p.0.min(q.0), top: p.1.min(q.1), right: p.0.max(q.0), bottom: p.1.max(q.1) })
    }

    /// See [`crate::document::Document::lift_objects`].
    ///
    /// Each object's operators are found the way restack finds them (a
    /// picture with its frame and placeholder, a shape with whether it also
    /// clips), carried with the state and clips in force (`carried_block`)
    /// into a page of their own, and **that page is checked before it is
    /// offered**: PDFium must list exactly the objects whose operators were
    /// taken, same kind, bounds and colours. Anything else is a refusal, and
    /// the app copies a picture as before. Reads only; the document is never
    /// changed.
    pub(super) fn lift(&self, page_index: usize, objects: &[usize]) -> Result<crate::document::Lifted> {
        use crate::pdf::{content, lift, File};

        if objects.is_empty() {
            return Err(PdfError::InvalidArgument("nothing to lift".into()));
        }
        let census = self.object_census(page_index)?;
        let bytes = self.readable_bytes()?;
        let file = File::parse(&bytes)?;
        let page = self.page_object(&file, page_index)?;
        let (stream, _) = self.page_content(&file, &page)?;
        let operations = content::parse(&stream)?;
        let states = content::states(&operations);
        let spans = drawing_spans(&census, &operations, &self.image_names(&file, &page)?);

        let mut chosen = Vec::with_capacity(objects.len());
        for &object in objects {
            let drawn = census
                .get(object)
                .ok_or_else(|| PdfError::InvalidArgument("that object is not on this page".into()))?;
            if drawn.kind as u32 == pdfium_render::prelude::FPDF_PAGEOBJ_TEXT {
                return Err(PdfError::Unsupported("words are copied as words, not as lines"));
            }
            chosen.push(spans[object].ok_or(PdfError::Unsupported(
                "this is drawn in a way this cannot follow through the drawing order",
            ))?);
        }
        // In stream order, outermost first, and one inside another only once:
        // a picture's placeholder chosen with the picture is already in its unit.
        chosen.sort_by_key(|&(first, last, _)| (first, std::cmp::Reverse(last)));
        let mut kept: Vec<(usize, usize, bool)> = Vec::with_capacity(chosen.len());
        for span in chosen {
            match kept.last() {
                Some(&(_, end, _)) if span.0 <= end && span.1 <= end => {}
                Some(&(_, end, _)) if span.0 <= end => {
                    return Err(PdfError::Unsupported("two of these are drawn into one another"));
                }
                _ => kept.push(span),
            }
        }

        let mut blocks = Vec::new();
        for &(first, last, self_clips) in &kept {
            blocks.extend(self.carried_block(
                &stream, &operations, &states, first, last, false, self_clips, IDENTITY_MATRIX,
            )?);
        }
        let blocks = lift::without_text_state(&lift::without_marked_content(&blocks)?)?;

        let resources = self
            .inherited(&file, &page, b"Resources")?
            .and_then(|r| r.as_dict().cloned())
            .unwrap_or_default();
        let resources = lift::prune_resources(&file, &resources, &lift::used_resource_names(&blocks)?)?;
        let (own, inherited) = self.geometry(&file, &page)?;
        let pdf = lift::one_page_file(&file, own, inherited, &blocks, &resources)?;

        // What the clip must draw: every object whose operators lie inside
        // one of the lifted spans, so a picture brings its frame.
        let expected: Vec<usize> = (0..census.len())
            .filter(|&i| {
                spans[i].is_some_and(|(f, l, _)| kept.iter().any(|&(first, last, _)| first <= f && l <= last))
            })
            .collect();
        let drawn = Self::open_bytes(pdf.clone(), None)?.object_census(0)?;
        let near = |a: Rect, b: Rect| {
            [(a.left, b.left), (a.top, b.top), (a.right, b.right), (a.bottom, b.bottom)]
                .iter()
                .all(|(p, q)| (p - q).abs() <= 0.5)
        };
        let same = drawn.len() == expected.len()
            && drawn.iter().zip(&expected).all(|(d, &i)| {
                let s = &census[i];
                d.kind == s.kind && near(d.rect, s.rect) && d.fill == s.fill && d.stroke == s.stroke
            });
        if !same {
            return Err(PdfError::Unsupported("the copied lines would not draw what was selected"));
        }

        let rect = expected.iter().map(|&i| census[i].rect).reduce(|a, b| Rect {
            left: a.left.min(b.left),
            top: a.top.min(b.top),
            right: a.right.max(b.right),
            bottom: a.bottom.max(b.bottom),
        });
        Ok(crate::document::Lifted {
            pdf,
            rect: rect.expect("at least one object was chosen"),
            objects: expected.len(),
        })
    }
}

/// Where each picture, shape, group and shading of a page is drawn, as first
/// and last operation inclusive and whether a shape also sets a clip; `None`
/// for words and for any kind whose count in the stream is not PDFium's.
///
/// The *n*th of a kind in the stream is the *n*th of that kind PDFium lists,
/// once the counts agree — as `object_spans` matches them, without its words.
pub(super) fn drawing_spans(
    census: &[Drawn],
    operations: &[crate::pdf::content::Operation],
    images: &std::collections::BTreeSet<Vec<u8>>,
) -> Vec<Option<(usize, usize, bool)>> {
    use crate::pdf::Object;
    use pdfium_render::prelude::{FPDF_PAGEOBJ_FORM, FPDF_PAGEOBJ_IMAGE, FPDF_PAGEOBJ_PATH, FPDF_PAGEOBJ_SHADING};

    fn by_kind(census: &[Drawn], spans: &mut [Option<(usize, usize, bool)>], kind: u32, found: Vec<(usize, usize, bool)>) {
        let members: Vec<usize> = (0..census.len()).filter(|&i| census[i].kind as u32 == kind).collect();
        if members.len() == found.len() {
            for (object, span) in members.into_iter().zip(found) {
                spans[object] = Some(span);
            }
        }
    }
    let single = |wanted: &dyn Fn(&crate::pdf::content::Operation) -> bool| -> Vec<(usize, usize, bool)> {
        operations.iter().enumerate().filter(|(_, op)| wanted(op)).map(|(i, _)| (i, i, false)).collect()
    };

    let mut spans = vec![None; census.len()];
    let pictures = image_operators(operations, images)
        .into_iter()
        .map(|at| picture_unit(operations, at))
        .map(|unit| (unit.start, unit.end - 1, false))
        .collect();
    by_kind(census, &mut spans, FPDF_PAGEOBJ_IMAGE, pictures);
    let shapes = path_operators(operations).into_iter().map(|(r, clips)| (r.start, r.end - 1, clips)).collect();
    by_kind(census, &mut spans, FPDF_PAGEOBJ_PATH, shapes);
    let forms = single(&|op| {
        op.operator == b"Do" && matches!(op.operands.first(), Some(Object::Name(n)) if !images.contains(n))
    });
    by_kind(census, &mut spans, FPDF_PAGEOBJ_FORM, forms);
    by_kind(census, &mut spans, FPDF_PAGEOBJ_SHADING, single(&|op| op.operator == b"sh"));
    spans
}

/// Graphics state pasted lines start from, whatever the page left in force:
/// they are drawn after everything else on the page, and a page that ends
/// with `1 1 1 rg` or half opacity at its top level would paint a shape that
/// relied on the default black white, or faded.
const RESET: &[u8] = b"0 g 0 G 1 w 0 J 0 j 10 M [] 0 d\n";

impl PdfiumDocument {
    /// See [`crate::document::DocumentMut::paste_lifted_mut`].
    ///
    /// **Pasted lines are lines of the page's own.** Reported from use on
    /// 0.1.70, which pasted them as one group: the group could be selected
    /// but not deleted or resized, and nothing in it could be picked. Now the
    /// clip's operators are written into the page's own drawing, so each
    /// pasted line is a shape like the original, edited the same way.
    ///
    /// One rewrite through the engine's own writer: the resources the lines
    /// use are deep-copied into fresh objects and put in the page's **own**
    /// resources (never the dictionary it inherits or shares), under their own
    /// names unless the page already uses a name, when the lines are written
    /// with a fresh one; and one content stream is appended that closes
    /// whatever the page left open, puts the graphics state back, and draws
    /// the lines through the matrix that puts them at `at` (turned with the
    /// pages, so they look as they did). **Checked before it is kept** (see
    /// [`Self::paste_harm`]); a refusal leaves the document exactly as it was,
    /// because nothing is adopted until the check passes.
    pub(super) fn paste_lifted(&mut self, page_index: usize, lifted: &[u8], at: Point) -> Result<()> {
        use crate::pdf::{content, lift, write_object, write_stream, Dict, File, Object};

        self.validate_page_index(page_index)?;

        // What the clip draws, where, and how its page was turned.
        let clip = Self::open_bytes(lifted.to_vec(), None)?;
        if clip.page_count != 1 {
            return Err(PdfError::Unsupported("a copied drawing is one page"));
        }
        let drawn = clip.object_census(0)?;
        let rect = drawn
            .iter()
            .map(|d| d.rect)
            .reduce(|a, b| Rect {
                left: a.left.min(b.left),
                top: a.top.min(b.top),
                right: a.right.max(b.right),
                bottom: a.bottom.max(b.bottom),
            })
            .ok_or(PdfError::Unsupported("the copied drawing draws nothing"))?;
        let (clip_to_user, clip_turns) = clip.page_to_user(0)?;
        let (target_to_user, target_turns) = self.page_to_user(page_index)?;
        let turns = (clip_turns + 4 - target_turns) % 4;
        let matrix = paste_matrix(clip_to_user, rect, turns, at, target_to_user)?;
        // Where each line should land, and the whole paste.
        let expected: Vec<Drawn> = drawn
            .iter()
            .map(|d| Drawn { kind: d.kind, rect: landed(d.rect, rect, turns, at), fill: d.fill, stroke: d.stroke })
            .collect();
        let area = landed(rect, rect, turns, at);

        // The clip's content and only the resources it names; one it names
        // and does not hold is a refusal here, before anything is written.
        let clip_file = File::parse(lifted)?;
        let clip_page = self.page_object(&clip_file, 0)?;
        let (clip_content, _) = self.page_content(&clip_file, &clip_page)?;
        let resources = self
            .inherited(&clip_file, &clip_page, b"Resources")?
            .and_then(|r| r.as_dict().cloned())
            .unwrap_or_default();
        let mut resources = lift::prune_resources(&clip_file, &resources, &lift::used_resource_names(&clip_content)?)?;
        let reset_name = with_reset_state(&mut resources);
        let mut lines = RESET.to_vec();
        lines.extend_from_slice(format!("/{reset_name} gs\n").as_bytes());
        lines.extend_from_slice(&concat_matrix(&matrix)?);
        lines.push(b'\n');
        lines.extend_from_slice(&clip_content);

        let was_secured = self.already_secured;
        let plus = self.secure_plus;
        let permissions = self.permissions();
        let base = self.edit_base()?;
        let file = File::parse(&base.bytes)?;
        let first = file.next_object_number()?;
        let (copied, copies) = lift::copy_graph(&clip_file, &Object::Dict(resources.clone()), first + 1)?;
        let Object::Dict(copied) = copied else {
            return Err(PdfError::Unsupported("the copied drawing's resources are not a dictionary"));
        };

        // The page: its resources made its own, the lines' resources in them,
        // and one more stream.
        let page_number = self.page_object_number(&file, page_index)?;
        let page = file.object(page_number)?;
        // A page with no content at all (a blank one) still takes a paste.
        let stream = match page.as_dict().and_then(|d| d.get(b"Contents")) {
            Some(_) => self.page_content(&file, &page)?.0,
            None => Vec::new(),
        };
        let closing = closing_state(&content::parse(&stream)?)
            .ok_or(PdfError::Unsupported("this page's content ends inside a text object"))?;
        let mut own = self.inherited(&file, &page, b"Resources")?.and_then(|r| r.as_dict().cloned()).unwrap_or_default();
        let mut renames = lift::Renames::new();
        for (category, table) in copied.0 {
            let Object::Dict(table) = table else { continue };
            let mut mine = own
                .get(&category)
                .map(|x| file.resolve(x))
                .transpose()?
                .and_then(|x| x.as_dict().cloned())
                .unwrap_or_default();
            for (name, value) in table.0 {
                let fresh = (0u32..)
                    .map(|n| if n == 0 { name.clone() } else { [&name[..], format!("_{n}").as_bytes()].concat() })
                    .find(|n| mine.get(n).is_none())
                    .expect("a name not yet taken");
                if fresh != name {
                    renames.insert((category.clone(), name), fresh.clone());
                }
                mine.set(&fresh, value);
            }
            own.set(&category, Object::Dict(mine));
        }
        let mut contents = match page.as_dict().and_then(|d| d.get(b"Contents")) {
            Some(Object::Array(items)) => items.clone(),
            Some(reference @ Object::Reference(number, _)) => match file.object(*number)? {
                Object::Array(items) => items,
                _ => vec![reference.clone()],
            },
            _ => Vec::new(),
        };
        contents.push(Object::Reference(first, 0));
        let mut dict = page.as_dict().cloned().ok_or(PdfError::Unsupported("that page is not a dictionary"))?;
        dict.set(b"Resources", Object::Dict(own));
        dict.set(b"Contents", Object::Array(contents));
        let mut page_body = Vec::new();
        write_object(&mut page_body, &Object::Dict(dict));

        // Close what the page left open, undo the transform it left, draw.
        let mut appended = b"\n".to_vec();
        for _ in 0..closing.0 {
            appended.extend_from_slice(b"Q\n");
        }
        appended.extend_from_slice(b"q\n");
        if closing.1 != IDENTITY_MATRIX {
            let undo = inverse(closing.1)
                .ok_or(PdfError::Unsupported("this page's content leaves a transform that cannot be undone"))?;
            appended.extend_from_slice(&concat_matrix(&undo)?);
            appended.push(b'\n');
        }
        appended.extend_from_slice(&lift::rename_resources(&lines, &renames)?);
        appended.extend_from_slice(b"\nQ\n");
        let flate = Dict(vec![(b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec()))]);

        let mut extra = vec![(first, write_stream(&flate, &content::encode(&appended)?))];
        extra.extend(copies);
        let rewritten = Self::write_edit(&base, &file, &[(page_number, page_body)], &extra)?;

        // What the lines should look like on paper with this page's geometry:
        // the same lines through the same matrix on a blank page.
        let (own, inherited) = self.geometry(&file, &page)?;
        let mut alone = b"q\n".to_vec();
        alone.extend_from_slice(&lines);
        alone.extend_from_slice(b"\nQ\n");
        let alone = Self::open_bytes(lift::one_page_file(&clip_file, own, inherited, &alone, &resources)?, None)?;

        let after = Self::open_bytes(rewritten.clone(), None)?;
        if let Some(reason) = self.paste_harm(&after, &alone, page_index, &expected, area)? {
            return Err(PdfError::Unsupported(reason));
        }
        self.adopt_edit(&base, rewritten, was_secured, plus, permissions)
    }

    /// A page's `/MediaBox`, `/CropBox` and `/Rotate`: those on the page
    /// itself, and those it inherits, kept apart — see
    /// [`crate::pdf::lift::one_page_file`].
    fn geometry(
        &self,
        file: &crate::pdf::File<'_>,
        page: &crate::pdf::Object,
    ) -> Result<(crate::pdf::Dict, crate::pdf::Dict)> {
        let (mut own, mut inherited) = (crate::pdf::Dict::default(), crate::pdf::Dict::default());
        for key in [&b"MediaBox"[..], b"CropBox", b"Rotate"] {
            let on_page = page.as_dict().is_some_and(|d| d.get(key).is_some());
            if let Some(value) = self.inherited(file, page, key)? {
                if on_page { &mut own } else { &mut inherited }.set(key, value);
            }
        }
        Ok((own, inherited))
    }

    /// Why a pasted page must not be kept, or `None`.
    ///
    /// The page count is the same; every object already on the page is, in
    /// order, the same kind, place and colour; after them come exactly the
    /// pasted lines, each where it was put, of its kind and colours; the
    /// words on the page are the same; nothing changed outside the paste; and
    /// the paste draws what the lines alone draw (`alone`) wherever it landed
    /// on paper — which a clip or a transform the page left in force would not.
    fn paste_harm(
        &self,
        after: &PdfiumDocument,
        alone: &PdfiumDocument,
        page_index: usize,
        expected: &[Drawn],
        area: Rect,
    ) -> Result<Option<&'static str>> {
        if after.page_count != self.page_count {
            return Ok(Some("pasting here would change the number of pages"));
        }
        let near = |a: Rect, b: Rect, within: f32| {
            [(a.left, b.left), (a.top, b.top), (a.right, b.right), (a.bottom, b.bottom)]
                .iter()
                .all(|(p, q)| (p - q).abs() <= within)
        };
        let same = |a: &Drawn, b: &Drawn, within: f32| {
            a.kind == b.kind && near(a.rect, b.rect, within) && a.fill == b.fill && a.stroke == b.stroke
        };
        let (was, now) = (self.object_census(page_index)?, after.object_census(page_index)?);
        if now.len() != was.len() + expected.len() {
            return Ok(Some("pasting here would not add exactly the copied lines"));
        }
        if !was.iter().zip(&now).all(|(a, b)| same(a, b, 0.5)) {
            return Ok(Some("pasting here would change what is already on the page"));
        }
        if !now[was.len()..].iter().zip(expected).all(|(a, b)| same(a, b, 1.0)) {
            return Ok(Some("the lines would not land where they were put"));
        }
        if self.text_runs(page_index)? != after.text_runs(page_index)? {
            return Ok(Some("pasting here would change words on the page"));
        }
        if changed_outside(self, after, page_index, area)?.is_some() {
            return Ok(Some("pasting here would change the page outside the paste"));
        }

        // Where the drawing alone has ink and the page had paper, the pasted
        // page must show that ink. Same page geometry and the same matrix, so
        // pixel for pixel, at any fraction of a point.
        let size = self.page_size(page_index)?;
        let scale = check_scale(size.width_pt, size.height_pt);
        let render = |doc: &PdfiumDocument, page: usize| -> Result<crate::render::Bitmap> {
            let mut b = doc.page(page)?.render_region(&RegionRequest {
                crop: Rect { left: 0.0, top: 0.0, right: size.width_pt, bottom: size.height_pt },
                scale,
                ..RegionRequest::default()
            })?;
            b.convert_to(crate::render::PixelOrder::Rgba);
            Ok(b)
        };
        let (paper, ink, pasted) = (render(self, page_index)?, render(alone, 0)?, render(after, page_index)?);
        if (paper.width, paper.height) != (ink.width, ink.height) || (paper.width, paper.height) != (pasted.width, pasted.height) {
            return Ok(Some("the drawing could not be compared with the page"));
        }
        let luminance = |b: &crate::render::Bitmap, at: usize| {
            0.299 * f32::from(b.data[at]) + 0.587 * f32::from(b.data[at + 1]) + 0.114 * f32::from(b.data[at + 2])
        };
        let (mut on_paper, mut matching) = (0usize, 0usize);
        for y in 0..paper.height as usize {
            for x in 0..paper.width as usize {
                let at = y * paper.stride + x * 4;
                let want = luminance(&ink, at);
                if want < 200.0 && luminance(&paper, at) >= 250.0 {
                    on_paper += 1;
                    matching += usize::from((luminance(&pasted, at) - want).abs() <= 40.0);
                }
            }
        }
        if on_paper > 0 && matching * 100 < on_paper * 97 {
            return Ok(Some("the pasted lines would not look like the copied ones here"));
        }
        Ok(None)
    }
}

/// Clip user space to target user space, so the paste looks as it did:
/// to the clip's page space, turned `turns` quarter turns clockwise about the
/// drawing's centre, moved so that centre is `at`, then to the target's user
/// space. Page spaces have y down, so a clockwise quarter turn takes
/// `(dx, dy)` to `(-dy, dx)`.
fn paste_matrix(clip_to_user: [f32; 6], rect: Rect, turns: u8, at: Point, target_to_user: [f32; 6]) -> Result<[f32; 6]> {
    let to_page = inverse(clip_to_user).ok_or(PdfError::Unsupported("the copied drawing's page cannot be measured"))?;
    let (cx, cy) = ((rect.left + rect.right) / 2.0, (rect.top + rect.bottom) / 2.0);
    let turn = match turns % 4 {
        0 => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        1 => [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
        2 => [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
        _ => [0.0, -1.0, 1.0, 0.0, 0.0, 0.0],
    };
    Ok([
        to_page,
        [1.0, 0.0, 0.0, 1.0, -cx, -cy],
        turn,
        [1.0, 0.0, 0.0, 1.0, at.x, at.y],
        target_to_user,
    ]
    .into_iter()
    .reduce(matrices)
    .expect("five matrices"))
}

/// Where `r` (clip page space) lands in the target's page space: turned
/// `turns` quarter turns clockwise about the centre of `around`, which is
/// put at `at` — the same move as [`paste_matrix`], for checking it.
fn landed(r: Rect, around: Rect, turns: u8, at: Point) -> Rect {
    let (cx, cy) = ((around.left + around.right) / 2.0, (around.top + around.bottom) / 2.0);
    let turn = |x: f32, y: f32| {
        let (dx, dy) = (x - cx, y - cy);
        let (dx, dy) = match turns % 4 {
            0 => (dx, dy),
            1 => (-dy, dx),
            2 => (-dx, -dy),
            _ => (dy, -dx),
        };
        (at.x + dx, at.y + dy)
    };
    let (a, b) = (turn(r.left, r.top), turn(r.right, r.bottom));
    Rect { left: a.0.min(b.0), top: a.1.min(b.1), right: a.0.max(b.0), bottom: a.1.max(b.1) }
}

/// Add an ExtGState that puts opacity, blending and soft masks back to their
/// defaults, under a name the resources do not use, and return the name.
fn with_reset_state(resources: &mut crate::pdf::Dict) -> String {
    use crate::pdf::{Dict, Object};
    let mut states = resources.get(b"ExtGState").and_then(Object::as_dict).cloned().unwrap_or_default();
    let name = (0u32..)
        .map(|n| if n == 0 { "PagifyReset".to_string() } else { format!("PagifyReset{n}") })
        .find(|n| states.get(n.as_bytes()).is_none())
        .expect("a name not yet taken");
    let number = |v: &str| Object::Number(v.as_bytes().to_vec());
    let reset = Dict(vec![
        (b"Type".to_vec(), Object::Name(b"ExtGState".to_vec())),
        (b"CA".to_vec(), number("1")),
        (b"ca".to_vec(), number("1")),
        (b"BM".to_vec(), Object::Name(b"Normal".to_vec())),
        (b"SMask".to_vec(), Object::Name(b"None".to_vec())),
    ]);
    states.set(name.as_bytes(), Object::Dict(reset));
    resources.set(b"ExtGState", Object::Dict(states));
    name
}

/// The scale a paste's ink check renders a page at: 2, or less on a page so
/// big that three renders at 2 would hold the paste up — never more than four
/// million pixels a render.
fn check_scale(width_pt: f32, height_pt: f32) -> f32 {
    const MOST_PIXELS: f32 = 4_000_000.0;
    2.0f32.min((MOST_PIXELS / (width_pt * height_pt).max(1.0)).sqrt())
}

#[cfg(test)]
mod tests {
    use super::check_scale;

    /// Three whole-page renders per paste: at scale 2 an A0 drawing is 32
    /// million pixels each, 128 MB, and the paste stops for seconds.
    #[test]
    fn the_ink_check_never_renders_more_than_four_million_pixels() {
        assert_eq!(check_scale(595.0, 842.0), 2.0, "an A4 page is checked as sharply as asked");
        let s = check_scale(2384.0, 3370.0);
        assert!(s < 2.0 && 2384.0 * 3370.0 * s * s <= 4_000_001.0, "an A0 page at {s}");
    }
}
