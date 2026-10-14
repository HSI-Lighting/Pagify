//! Baking a page's visible annotations into its content, for a copy that is
//! sent to somebody rather than edited.
//!
//! # Why
//!
//! A picture placed in Pagify is a Stamp *annotation*: a separate object laid
//! over the page. Readers draw annotations; thumbnail and preview renderers —
//! WhatsApp's among them — often draw only the page, so the picture shows as the
//! grey box under it. This puts the annotation's appearance into the page itself.
//!
//! # How a picture is written
//!
//! **As a plain image placement, not a form.** `q a b c d e f cm /Name Do Q`
//! naming the image itself, so the engine lists it as a picture and can move and
//! remove it. A form XObject (what `qpdf --flatten-annotations` writes) is
//! "a group" to the engine, which moves and removes only pictures, shapes and
//! words — measured on a real datasheet, a form there is refused both ways.
//! Anything that is not one image in its own box goes down as a form, which is
//! faithful and needs no editing.
//!
//! Written without PDFium: the file is read, the page objects that change are
//! replaced and new ones added, and every other byte is copied through — the
//! reason this module exists next to [`super::File`] and not on
//! `FPDFPage_GenerateContent`, which re-emits whole pages.

use std::collections::BTreeSet;

use super::{content, write_object, write_stream, Dict, File, Object};
use crate::error::{PdfError, Result};

/// What a flatten did, and the file it produced.
#[derive(Debug)]
pub struct Flattened {
    /// The whole file. Byte for byte the input when nothing was flattened.
    pub bytes: Vec<u8>,
    /// Annotations put down as plain image placements.
    pub pictures: usize,
    /// Annotations put down as forms.
    pub forms: usize,
    /// Marks left as annotations because they carry **no appearance to bake**:
    /// the highlights, underlines and ink PDFium writes are a colour and some
    /// coordinates, and every reader draws them from those. Counted apart from
    /// `left` because these are the ones a preview will not show.
    pub bare: usize,
    /// Everything else left as it was: links, form fields, hidden ones, notes.
    pub left: usize,
}

/// What a flattened annotation's XObject is called on the page: this and the
/// first number not already taken.
const NAME_PREFIX: &str = "PgFlat";

/// The kinds of annotation that are marks on the page. Everything else — links,
/// fields, notes, popups, attachments — is something a reader does, not draws.
const MARKS: &[&[u8]] = &[
    b"Stamp", b"Highlight", b"Underline", b"StrikeOut", b"Squiggly", b"Ink", b"FreeText", b"Square", b"Circle", b"Line",
    b"Polygon", b"PolyLine",
];

/// Annotation flags (ISO 32000-1 table 165) that mean it is not drawn on screen.
const HIDDEN: i64 = 2;
const NO_VIEW: i64 = 32;

/// How far a picture may lie outside its form's box and still count as inside
/// it, in points — the width of a rounding error, not of a clip.
const SLACK: f64 = 0.5;

type Matrix = [f64; 6];

/// How one annotation goes down.
struct Plan {
    /// A picture in its own box (a plain image placement) or anything else (a form).
    picture: bool,
    /// Places the XObject: the form's box onto the annotation's rectangle, and
    /// for a picture its own placement through that.
    matrix: Matrix,
    /// What `Do` draws: the image for a picture, the appearance for a form.
    target: Object,
}

/// `first` and then `second`, as PDF concatenates matrices.
fn then(first: Matrix, second: Matrix) -> Matrix {
    let [a1, b1, c1, d1, e1, f1] = first;
    let [a2, b2, c2, d2, e2, f2] = second;
    [
        a1 * a2 + b1 * c2,
        a1 * b2 + b1 * d2,
        c1 * a2 + d1 * c2,
        c1 * b2 + d1 * d2,
        e1 * a2 + f1 * c2 + e2,
        e1 * b2 + f1 * d2 + f2,
    ]
}

fn at(m: Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// A direct array of exactly `count` numbers.
fn numbers(object: Option<&Object>, count: usize) -> Option<Vec<f64>> {
    let Some(Object::Array(items)) = object else { return None };
    let values: Option<Vec<f64>> = items.iter().map(Object::as_f64).collect();
    values.filter(|v| v.len() == count)
}

fn dict_of(file: &File<'_>, object: Option<&Object>) -> Option<Dict> {
    file.resolve(object?).ok()?.as_dict().cloned()
}

/// `(left, bottom, right, top)` of four numbers in any corner order.
fn corners(values: &[f64]) -> (f64, f64, f64, f64) {
    (values[0].min(values[2]), values[1].min(values[3]), values[0].max(values[2]), values[1].max(values[3]))
}

/// Why an annotation is left as it is.
enum Skip {
    /// Not a mark, or not shown: a link, a field, a note, a hidden one.
    Other,
    /// A mark somebody sees, with no appearance to bake — see [`Flattened::bare`].
    Bare,
}

/// How `annot` should be flattened, or why not.
fn plan(file: &File<'_>, annot: &Dict) -> std::result::Result<Plan, Skip> {
    let Some(Object::Name(kind)) = annot.get(b"Subtype") else { return Err(Skip::Other) };
    if !MARKS.contains(&kind.as_slice()) {
        return Err(Skip::Other);
    }
    let flags = annot.get(b"F").and_then(Object::as_i64).unwrap_or(0);
    if flags & (HIDDEN | NO_VIEW) != 0 {
        return Err(Skip::Other);
    }
    appearance(file, annot).ok_or(Skip::Bare)
}

/// The placement of `annot`'s own appearance, when it has one that can be read.
fn appearance(file: &File<'_>, annot: &Dict) -> Option<Plan> {
    let (left, bottom, right, top) = corners(&numbers(annot.get(b"Rect"), 4)?);

    let appearance = dict_of(file, annot.get(b"AP"))?;
    let normal = appearance.get(b"N")?.clone();
    let Object::Stream(form, range) = file.resolve(&normal).ok()? else { return None };
    let (bx0, by0, bx1, by1) = corners(&numbers(form.get(b"BBox"), 4)?);
    let form_matrix: Matrix = match form.get(b"Matrix") {
        Some(m) => numbers(Some(m), 6)?.try_into().ok()?,
        None => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };

    // ISO 32000-1 12.5.5: the form's box, through its own matrix, is mapped onto
    // the rectangle.
    let seen: Vec<(f64, f64)> =
        [(bx0, by0), (bx1, by0), (bx0, by1), (bx1, by1)].iter().map(|&(x, y)| at(form_matrix, x, y)).collect();
    let (tx0, tx1) = seen.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
    let (ty0, ty1) = seen.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    if tx1 - tx0 <= 0.0 || ty1 - ty0 <= 0.0 || right - left <= 0.0 || top - bottom <= 0.0 {
        return None;
    }
    let (sx, sy) = ((right - left) / (tx1 - tx0), (top - bottom) / (ty1 - ty0));
    let map: Matrix = [sx, 0.0, 0.0, sy, left - sx * tx0, bottom - sy * ty0];
    let as_form = Plan { picture: false, matrix: map, target: normal };

    // A picture is one image drawn inside its own box, in a form that adds
    // nothing of its own: `q a b c d e f cm /Name Do Q`.
    if form_matrix != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] {
        return Some(as_form);
    }
    let Some(drawing) = content::decode(&form, &file.bytes()[range]) else { return Some(as_form) };
    let Ok(ops) = content::parse(&drawing) else { return Some(as_form) };
    let operators: Vec<&[u8]> = ops.iter().map(|o| o.operator.as_slice()).collect();
    if operators != [b"q".as_slice(), b"cm", b"Do", b"Q"] {
        return Some(as_form);
    }
    let Some(placed) = ops[1].operands.iter().map(Object::as_f64).collect::<Option<Vec<f64>>>().filter(|v| v.len() == 6) else {
        return Some(as_form);
    };
    let placed: Matrix = placed.try_into().ok()?;
    let Some(Object::Name(name)) = ops[2].operands.first() else { return Some(as_form) };
    let image = dict_of(file, form.get(b"Resources"))
        .and_then(|r| dict_of(file, r.get(b"XObject")))
        .and_then(|x| x.get(name).cloned());
    let Some(image @ Object::Reference(..)) = image else { return Some(as_form) };
    let is_image = matches!(
        file.resolve(&image),
        Ok(Object::Stream(d, _)) if d.get(b"Subtype") == Some(&Object::Name(b"Image".to_vec()))
    );
    // Drawn outside the box, the form clips it and a bare placement would not.
    let inside = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].iter().all(|&(x, y)| {
        let (px, py) = at(placed, x, y);
        px >= bx0 - SLACK && px <= bx1 + SLACK && py >= by0 - SLACK && py <= by1 + SLACK
    });
    if !is_image || !inside {
        return Some(as_form);
    }
    Some(Plan { picture: true, matrix: then(placed, map), target: image })
}

/// Numbers as a PDF writes them: no exponent, no trailing zeros, no `-0`.
fn number(value: f64) -> String {
    let text = format!("{value:.5}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" { "0".into() } else { text.into() }
}

/// A page, where it is, and the resources it inherits if it has none.
struct PageAt {
    number: u32,
    dict: Dict,
    resources: Option<Object>,
}

fn collect_pages(
    file: &File<'_>,
    node: &Object,
    inherited: Option<Object>,
    depth: usize,
    seen: &mut BTreeSet<u32>,
    out: &mut Vec<PageAt>,
) -> Result<()> {
    let Some((number, _)) = node.as_reference() else { return Ok(()) };
    if depth > 64 || !seen.insert(number) {
        return Err(PdfError::InvalidArgument("the page tree loops or is too deep".into()));
    }
    let Object::Dict(dict) = file.object(number)? else { return Ok(()) };
    let resources = dict.get(b"Resources").cloned().or(inherited);
    match dict.get(b"Kids") {
        Some(Object::Array(kids)) => {
            for kid in kids {
                collect_pages(file, kid, resources.clone(), depth + 1, seen, out)?;
            }
        }
        _ => out.push(PageAt { number, dict, resources }),
    }
    Ok(())
}

/// Flatten every page's visible markup annotations into its content.
pub fn flatten_annotations(bytes: &[u8]) -> Result<Flattened> {
    let file = File::parse(bytes)?;
    let root = dict_of(&file, file.trailer().get(b"Root"))
        .ok_or_else(|| PdfError::InvalidArgument("the file has no catalogue".into()))?;
    let mut pages = Vec::new();
    if let Some(tree) = root.get(b"Pages") {
        collect_pages(&file, tree, None, 0, &mut BTreeSet::new(), &mut pages)?;
    }

    let mut next = file.next_object_number()?;
    let fresh = |taken: &mut u32| {
        let number = *taken;
        *taken += 1;
        number
    };
    let mut replacements: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut extra: Vec<(u32, Vec<u8>)> = Vec::new();
    let (mut pictures, mut forms, mut bare, mut left) = (0, 0, 0, 0);
    // `q` and `Q`, one object each for the whole file, made when first wanted.
    let mut brackets: Option<(u32, u32)> = None;

    for page in pages {
        let Some(annots) = page.dict.get(b"Annots") else { continue };
        let items: Vec<Object> = match file.resolve(annots)? {
            Object::Array(items) => items,
            _ => continue,
        };

        let resources = page
            .resources
            .as_ref()
            .and_then(|r| dict_of(&file, Some(r)))
            .unwrap_or_default();
        let mut xobjects = dict_of(&file, resources.get(b"XObject")).unwrap_or_default();
        let mut kept = Vec::new();
        let mut drawn = String::new();
        let mut counter = 0u32;

        for item in items {
            let planned = dict_of(&file, Some(&item)).map_or(Err(Skip::Other), |annot| plan(&file, &annot));
            let plan = match planned {
                Ok(plan) => plan,
                Err(skip) => {
                    match skip {
                        Skip::Bare => bare += 1,
                        Skip::Other => left += 1,
                    }
                    kept.push(item);
                    continue;
                }
            };
            let name = loop {
                counter += 1;
                let candidate = format!("{NAME_PREFIX}{counter}");
                if xobjects.get(candidate.as_bytes()).is_none() {
                    break candidate;
                }
            };
            xobjects.set(name.as_bytes(), plan.target);
            let m = plan.matrix;
            drawn += &format!(
                "q {} {} {} {} {} {} cm /{name} Do Q\n",
                number(m[0]), number(m[1]), number(m[2]), number(m[3]), number(m[4]), number(m[5])
            );
            if plan.picture { pictures += 1 } else { forms += 1 }
        }
        if drawn.is_empty() {
            continue;
        }

        let (open, close) = *brackets.get_or_insert_with(|| {
            let (open, close) = (fresh(&mut next), fresh(&mut next));
            extra.push((open, write_stream(&Dict::default(), b"q\n")));
            extra.push((close, write_stream(&Dict::default(), b"Q\n")));
            (open, close)
        });
        let added = fresh(&mut next);
        extra.push((added, write_stream(&Dict::default(), drawn.as_bytes())));

        // The page's own drawing, whatever shape its /Contents had.
        let own: Vec<Object> = match page.dict.get(b"Contents") {
            None => Vec::new(),
            Some(contents) => match file.resolve(contents)? {
                Object::Array(items) => items,
                _ => vec![contents.clone()],
            },
        };
        let mut contents = vec![Object::Reference(open, 0)];
        contents.extend(own);
        contents.push(Object::Reference(close, 0));
        contents.push(Object::Reference(added, 0));

        let mut dict = page.dict.clone();
        let mut resources = resources;
        resources.set(b"XObject", Object::Dict(xobjects));
        dict.set(b"Resources", Object::Dict(resources));
        dict.set(b"Contents", Object::Array(contents));
        if kept.is_empty() {
            dict.remove(b"Annots");
        } else {
            dict.set(b"Annots", Object::Array(kept));
        }
        let mut body = Vec::new();
        write_object(&mut body, &Object::Dict(dict));
        replacements.push((page.number, body));
    }

    if replacements.is_empty() {
        return Ok(Flattened { bytes: bytes.to_vec(), pictures, forms, bare, left });
    }
    let bytes = file.rewrite_adding(&replacements, &extra, &Dict::default())?;
    Ok(Flattened { bytes, pictures, forms, bare, left })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::{content, Dict, File, Object};

    /// A classic-xref PDF whose object `n` is `objects[n - 1]`.
    fn build(objects: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
        );
        out
    }

    fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        out.extend_from_slice(data);
        out.extend_from_slice(b"\nendstream");
        out
    }

    /// Objects 1-4 are always the catalogue, the page tree, the page and its
    /// own drawing; `page_extra` is what else the page dictionary says.
    fn one_page(page_extra: &str, annots: &str, more: &[Vec<u8>]) -> Vec<u8> {
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R {page_extra} /Annots [{annots}] >>")
                .into_bytes(),
            stream("", b"0.9 g 0 0 612 792 re f"),
        ];
        objects.extend_from_slice(more);
        build(&objects)
    }

    /// A 1x1 red image, as object 7 in the pictures below.
    fn image() -> Vec<u8> {
        stream("/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8", &[255, 0, 0])
    }

    /// A Stamp over `rect` whose appearance (object 6) is `ap` drawn in `bbox`,
    /// naming the image `/Im` (object 7).
    fn picture_stamp(rect: &str, bbox: &str, ap: &str) -> Vec<u8> {
        one_page(
            "/Resources << >>",
            "5 0 R",
            &[
                format!("<< /Type /Annot /Subtype /Stamp /Rect [{rect}] /AP << /N 6 0 R >> >>").into_bytes(),
                stream(&format!("/Type /XObject /Subtype /Form /BBox [{bbox}] /Resources << /XObject << /Im 7 0 R >> >>"), ap.as_bytes()),
                image(),
            ],
        )
    }

    fn page(file: &File<'_>) -> Dict {
        file.object(3).expect("page").as_dict().expect("a dict").clone()
    }

    /// The page's drawing, each stream decoded, in order.
    fn drawing(file: &File<'_>, page: &Dict) -> Vec<String> {
        let Some(Object::Array(items)) = page.get(b"Contents") else { panic!("Contents is not an array") };
        items
            .iter()
            .map(|item| {
                let Object::Stream(dict, range) = file.resolve(item).expect("stream") else { panic!("not a stream") };
                String::from_utf8(content::decode(&dict, &file.bytes()[range]).expect("decodes")).expect("text")
            })
            .collect()
    }

    fn xobjects(page: &Dict) -> Dict {
        let Some(Object::Dict(resources)) = page.get(b"Resources") else { panic!("no direct Resources") };
        let Some(Object::Dict(x)) = resources.get(b"XObject") else { panic!("no XObject") };
        x.clone()
    }

    /// The `cm` numbers and the name the last drawing put down: `q … cm /N Do Q`.
    fn placement(text: &str) -> (Vec<f64>, String) {
        let words: Vec<&str> = text.split_whitespace().collect();
        let cm = words.iter().position(|w| *w == "cm").expect("a cm");
        let numbers = words[cm - 6..cm].iter().map(|w| w.parse().expect("a number")).collect();
        (numbers, words[cm + 1].trim_start_matches('/').to_string())
    }

    /// **The break this catches:** the picture left as an annotation, which a
    /// thumbnail renderer never draws. Worked by hand: a 50 x 50 form mapped
    /// onto a 100 x 100 rectangle at (100, 600) scales by 2 and moves by
    /// (100, 600); the image's own 50 x 50 placement through that is
    /// `100 0 0 100 100 600`. Swapping the two matrices' order would give
    /// `e = 5000`.
    #[test]
    fn a_picture_stamp_becomes_a_plain_image_placement_in_its_rectangle() {
        let input = picture_stamp("100 600 200 700", "0 0 50 50", "q 50 0 0 50 0 0 cm /Im Do Q");
        let out = flatten_annotations(&input).expect("flattens");
        assert_eq!((out.pictures, out.forms, out.left), (1, 0, 0));

        let file = File::parse(&out.bytes).expect("a readable file");
        let page = page(&file);
        assert!(page.get(b"Annots").is_none(), "the annotation is still on the page");
        let last = drawing(&file, &page).pop().expect("a drawing");
        let (matrix, name) = placement(&last);
        assert_eq!(matrix, [100.0, 0.0, 0.0, 100.0, 100.0, 600.0]);
        assert!(last.contains(&format!("/{name} Do")));
        assert_eq!(xobjects(&page).get(name.as_bytes()), Some(&Object::Reference(7, 0)), "it must name the image, not a form");
    }

    /// **The break:** a rotated appearance placed as if upright. The image's
    /// `0 50 -50 0 50 0` through the same mapping is `0 100 -100 0 200 600`.
    #[test]
    fn a_rotated_picture_keeps_its_rotation() {
        let input = picture_stamp("100 600 200 700", "0 0 50 50", "q 0 50 -50 0 50 0 cm /Im Do Q");
        let out = flatten_annotations(&input).expect("flattens");
        let file = File::parse(&out.bytes).expect("a readable file");
        let last = drawing(&file, &page(&file)).pop().expect("a drawing");
        assert_eq!(placement(&last).0, [0.0, 100.0, -100.0, 0.0, 200.0, 600.0]);
    }

    /// **The break:** a picture that spills outside its box drawn without the
    /// form's clip. That one stays a form, which clips.
    #[test]
    fn a_picture_larger_than_its_box_is_kept_as_a_form_so_it_is_still_clipped() {
        let input = picture_stamp("100 600 200 700", "0 0 50 50", "q 100 0 0 100 0 0 cm /Im Do Q");
        let out = flatten_annotations(&input).expect("flattens");
        assert_eq!((out.pictures, out.forms), (0, 1));
        let file = File::parse(&out.bytes).expect("a readable file");
        let page = page(&file);
        let (_, name) = placement(&drawing(&file, &page).pop().expect("a drawing"));
        assert_eq!(xobjects(&page).get(name.as_bytes()), Some(&Object::Reference(6, 0)), "it must name the form");
    }

    /// **The break:** a highlight (anything that is not one picture) dropped.
    /// A 10 x 10 form onto a 20 x 20 rectangle at (10, 10): `2 0 0 2 10 10`.
    #[test]
    fn any_other_annotation_with_an_appearance_is_put_down_as_a_form() {
        let input = one_page(
            "/Resources << >>",
            "5 0 R",
            &[
                b"<< /Type /Annot /Subtype /Highlight /Rect [10 10 30 30] /AP << /N 6 0 R >> >>".to_vec(),
                stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10]", b"1 1 0 rg 0 0 10 10 re f"),
            ],
        );
        let out = flatten_annotations(&input).expect("flattens");
        assert_eq!((out.pictures, out.forms, out.left), (0, 1, 0));
        let file = File::parse(&out.bytes).expect("a readable file");
        let page = page(&file);
        let (matrix, name) = placement(&drawing(&file, &page).pop().expect("a drawing"));
        assert_eq!(matrix, [2.0, 0.0, 0.0, 2.0, 10.0, 10.0]);
        assert_eq!(xobjects(&page).get(name.as_bytes()), Some(&Object::Reference(6, 0)));
    }

    /// **The break:** flattening what a reader does not show or cannot — a link
    /// that goes somewhere, a field somebody fills in, a hidden mark, a note, a
    /// mark with nothing to draw. All four stay exactly where they are.
    #[test]
    fn links_fields_hidden_notes_and_marks_with_no_appearance_are_left_alone() {
        let ap = "/AP << /N 10 0 R >>";
        let input = one_page(
            "/Resources << >>",
            "5 0 R 6 0 R 7 0 R 8 0 R 9 0 R",
            &[
                b"<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] >>".to_vec(),
                format!("<< /Type /Annot /Subtype /Widget /FT /Tx /Rect [0 0 10 10] {ap} >>").into_bytes(),
                format!("<< /Type /Annot /Subtype /Stamp /F 2 /Rect [0 0 10 10] {ap} >>").into_bytes(),
                format!("<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] {ap} >>").into_bytes(),
                b"<< /Type /Annot /Subtype /Stamp /Rect [0 0 10 10] >>".to_vec(),
                stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10]", b"0 0 10 10 re f"),
            ],
        );
        let out = flatten_annotations(&input).expect("flattens");
        // Four left; the stamp with nothing to draw is the one a preview misses.
        assert_eq!((out.pictures, out.forms, out.bare, out.left), (0, 0, 1, 4));
        assert_eq!(out.bytes, input, "nothing was flattened, so not a byte may change");
    }

    /// **The break:** a highlight written the way PDFium writes one — a
    /// dictionary inside the page's `/Annots`, a colour and quad points and no
    /// appearance — being dropped, or being counted as a link. It is kept, and
    /// counted as the kind of mark a preview will not show.
    #[test]
    fn a_highlight_with_no_appearance_is_kept_and_counted_as_bare() {
        let input = one_page(
            "/Resources << >>",
            "<< /Type /Annot /Subtype /Highlight /Rect [10 10 30 30] /QuadPoints [10 30 30 30 10 10 30 10] /C [1 1 0] >>",
            &[],
        );
        let out = flatten_annotations(&input).expect("flattens");
        assert_eq!((out.pictures, out.forms, out.bare, out.left), (0, 0, 1, 0));
        assert_eq!(out.bytes, input);
    }

    /// **The break:** a highlight lost from a page that is rewritten because a
    /// picture on it was baked in — the case where the page's `/Annots` is
    /// rebuilt, and the one place a mark with no appearance could be dropped.
    #[test]
    fn a_mark_with_no_appearance_stays_on_a_page_where_a_picture_is_baked_in() {
        let input = one_page(
            "/Resources << >>",
            "5 0 R << /Type /Annot /Subtype /Highlight /Rect [10 10 30 30] /QuadPoints [10 30 30 30 10 10 30 10] /C [1 1 0] >>",
            &[
                b"<< /Type /Annot /Subtype /Stamp /Rect [100 600 200 700] /AP << /N 6 0 R >> >>".to_vec(),
                stream("/Type /XObject /Subtype /Form /BBox [0 0 50 50] /Resources << /XObject << /Im 7 0 R >> >>", b"q 50 0 0 50 0 0 cm /Im Do Q"),
                image(),
            ],
        );
        let out = flatten_annotations(&input).expect("flattens");
        assert_eq!((out.pictures, out.forms, out.bare, out.left), (1, 0, 1, 0));

        let file = File::parse(&out.bytes).expect("a readable file");
        let Some(Object::Array(annots)) = page(&file).get(b"Annots").cloned() else { panic!("the highlight was lost with its page's /Annots") };
        assert_eq!(annots.len(), 1, "{annots:?}");
        let Object::Dict(highlight) = &annots[0] else { panic!("not the highlight: {annots:?}") };
        assert_eq!(highlight.get(b"Subtype"), Some(&Object::Name(b"Highlight".to_vec())));
    }

    /// **The break:** the picture drawn under the page's own state — a page
    /// that ends inside a clip or a scaled `cm` would clip or scale it. The
    /// page's drawing is wrapped `q … Q` and the picture follows.
    #[test]
    fn the_page_is_wrapped_so_the_picture_starts_from_a_clean_state() {
        let input = picture_stamp("100 600 200 700", "0 0 50 50", "q 50 0 0 50 0 0 cm /Im Do Q");
        let out = flatten_annotations(&input).expect("flattens");
        let file = File::parse(&out.bytes).expect("a readable file");
        let streams = drawing(&file, &page(&file));
        assert_eq!(streams.len(), 4);
        assert_eq!(streams[0].trim(), "q");
        assert_eq!(streams[1], "0.9 g 0 0 612 792 re f", "the page's own drawing is untouched");
        assert_eq!(streams[2].trim(), "Q");
    }

    /// **The break:** a page that inherits its resources gaining a picture
    /// whose name is nowhere — or losing the font the page had. The page gets
    /// its own copy, with what it inherited and the picture.
    #[test]
    fn a_page_that_inherits_its_resources_keeps_them_and_gains_the_picture() {
        let input = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /XObject << /Old 8 0 R >> >> >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [5 0 R] >>".to_vec(),
            stream("", b"0.9 g 0 0 612 792 re f"),
            b"<< /Type /Annot /Subtype /Stamp /Rect [100 600 200 700] /AP << /N 6 0 R >> >>".to_vec(),
            stream("/Type /XObject /Subtype /Form /BBox [0 0 50 50] /Resources << /XObject << /Im 7 0 R >> >>", b"q 50 0 0 50 0 0 cm /Im Do Q"),
            image(),
            stream("/Type /XObject /Subtype /Form /BBox [0 0 1 1]", b""),
        ]);
        let out = flatten_annotations(&input).expect("flattens");
        let file = File::parse(&out.bytes).expect("a readable file");
        let x = xobjects(&page(&file));
        assert_eq!(x.get(b"Old"), Some(&Object::Reference(8, 0)), "what the page inherited was lost");
        assert_eq!(x.0.len(), 2, "the picture was not added: {x:?}");
    }

    /// **The break:** a name that is already taken. The page already has the
    /// first name this would pick; the picture must not replace it.
    #[test]
    fn the_picture_never_takes_a_name_the_page_already_uses() {
        let taken = format!("{NAME_PREFIX}1");
        let input = one_page(
            &format!("/Resources << /XObject << /{taken} 8 0 R >> >>"),
            "5 0 R",
            &[
                b"<< /Type /Annot /Subtype /Stamp /Rect [100 600 200 700] /AP << /N 6 0 R >> >>".to_vec(),
                stream("/Type /XObject /Subtype /Form /BBox [0 0 50 50] /Resources << /XObject << /Im 7 0 R >> >>", b"q 50 0 0 50 0 0 cm /Im Do Q"),
                image(),
                stream("/Type /XObject /Subtype /Form /BBox [0 0 1 1]", b""),
            ],
        );
        let out = flatten_annotations(&input).expect("flattens");
        let file = File::parse(&out.bytes).expect("a readable file");
        let x = xobjects(&page(&file));
        assert_eq!(x.get(taken.as_bytes()), Some(&Object::Reference(8, 0)), "the page's own object was replaced");
        assert_eq!(x.0.len(), 2);
    }
}
