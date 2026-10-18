//! Whole-document export: the pages' own content — words, rules, boxes and
//! pictures, where the page draws them — as one HTML file, the Conversion
//! tab's `tohtml`.
//!
//! A shell function over `Session`, not app code: it needs no window, and
//! here it can be tested against the committed fixtures. The PNG half of the
//! same tab (`toimage`) is `Session::export_page_png`.

use crate::Session;
use crate::session::PageTextSnapshot;

/// The document as one styled HTML file: **each page laid out the way the
/// page has it** — every run a span at its own size, colour and face; every
/// rule, box and outlined word an SVG path with its own fill, stroke and
/// dash; every picture its own embedded pixels under the matrix that placed
/// it — on a sheet the page's own size, under a small inline stylesheet,
/// with **the document's own font programs embedded as `@font-face`** so the
/// reader's letters and widths are the file's. The file opens looking like
/// the document, not like a wall of paragraphs.
///
/// **The page's own order, not a reading order.** Everything sits where the
/// page draws it, in the order the file stores it: nothing is reflowed, no
/// structure is invented between runs, and text over a picture is text over
/// a picture here too. Reconstruction is `reflow`'s job, done on the page
/// itself — this carries what the page stores.
///
/// **The pieces of one line ride together.** A producer splits a line
/// wherever kerning or a style changes, and writing those pieces out pinned
/// one by one overlapped them the moment the reader's fonts were not the
/// file's; a line's pieces now share one pinned wrapper and the reader's
/// text engine lays them out from the first piece's pen — see [`same_line`].
///
/// What is *not* carried: clip paths (a shape that clips others still shows
/// as a shape, but does not clip), shadings and gradients (their paths are
/// not paths), and the page's own background colour. **Annotations are not
/// page content and are not carried either** — comments, highlights, stamps,
/// placed signatures and the values in filled form fields are absent from
/// this file, even though `toimage` renders them (its render request draws
/// annotation appearances and form data). Use `toimage` when those matter.
/// `toimage` also remains the tab's way to keep a page entire.
///
/// **`pt` is the PDF's own point**, in CSS as in PDF, so the sheet prints
/// 1:1 and needs no zoom to be read on screen.
pub fn document_html(session: &Session, title: &str) -> pdf_core::Result<String> {
    Ok(document_html_watched(session, title, &mut |_, _| true)?
        .expect("a caller that never stops gets its document"))
}

/// [`document_html`] with its steps reported and the caller able to put it
/// down: `keep_going(page, pages)` is called as each page is finished —
/// `true` keeps the walk on, `false` stops it — and a stopped walk answers
/// `None`, because **half a file is not an export.**
///
/// The window's progress dialog is the caller this exists for: the export is
/// one call, and without this the dialog could show nothing but a spinner
/// while the whole document was built on the worker's thread.
pub fn document_html_watched(
    session: &Session,
    title: &str,
    keep_going: &mut dyn FnMut(usize, usize) -> bool,
) -> pdf_core::Result<Option<String>> {
    let pages = session.page_count()?;
    // **The body is built first.** The embedded `@font-face` rules are found
    // along the way — the programs live on the pages — and the head, which
    // carries them, is written before the body; so the body goes to a buffer
    // of its own and the file is assembled at the end.
    let mut body = String::new();
    let mut faces = FontSheet::default();
    for page in 0..pages {
        let size = session.page_size(page)?;
        body.push_str("<section class=\"page\">\n");
        body.push_str(&format!("<h2>Page {}</h2>\n", page + 1));
        body.push_str(&format!(
            "<div class=\"sheet\" style=\"width:{:.1}pt;height:{:.1}pt\">\n",
            size.width_pt, size.height_pt
        ));
        let snapshot = session.page_text_snapshot(page)?;
        // **A page whose pictures cannot be read is an error, not a page with
        // none** — the same rule as the paths below. The old `unwrap_or_default`
        // turned an engine failure into a silently naked page whose export
        // still reported success.
        let pictures = session.images_on(page)?;
        // **A page whose paths cannot be read is an error, not a blank
        // page**: see `Document::page_paints` — a file that quietly lost its
        // rules and boxes would be worse than one that did not export.
        let paints = session.page_paints(page)?;
        // Fonts, on the other hand, may degrade: a backend that cannot read
        // them costs the file its embedded faces (the reader substitutes by
        // name, as it would for any page) but nothing of what the page
        // *draws* — so a missing list is empty, not fatal.
        let fonts = session.page_fonts(page).unwrap_or_default();
        let families = faces.register(&fonts);
        let mut placed = 0usize;

        // **The page's own painting order, out of three lists that each
        // carry it in pieces.** Every run, picture and path records the
        // index of the object that draws it, and a form's paths are listed
        // at the form's own index — so one sort by `(object, within)` is the
        // stack the page shows, and consecutive paths merge into one SVG.
        let mut pieces: Vec<(usize, usize, Piece<'_>)> = Vec::new();
        for run in &snapshot.runs {
            if !run.text.trim().is_empty() {
                pieces.push((run.object, 0, Piece::Run(run)));
            }
        }
        for picture in &pictures {
            pieces.push((picture.object, 0, Piece::Picture(picture)));
        }
        for (index, paint) in paints.iter().enumerate() {
            pieces.push((paint.object, index + 1, Piece::Shape(paint)));
        }
        pieces.sort_by_key(|(object, order, _)| (*object, *order));

        // What a run's span needs beyond the run itself: the face the page
        // draws it with and the family its embedded program was declared
        // under.
        fn run_of<'a>(
            run: &'a pdf_core::document::TextRun,
            snapshot: &'a PageTextSnapshot,
            fonts: &'a [pdf_core::document::PageFont],
            families: &'a [Option<String>],
        ) -> Run<'a> {
            let style = snapshot.styles.get(&run.object);
            Run {
                run,
                style,
                font: style.and_then(|style| fonts.get(style.font as usize)),
                family: style
                    .and_then(|style| families.get(style.font as usize))
                    .and_then(|family| family.as_deref()),
            }
        }

        let mut open_shapes: Option<String> = None;
        let mut index = 0;
        while index < pieces.len() {
            match pieces[index].2 {
                Piece::Shape(paint) => {
                    let svg = open_shapes.get_or_insert_with(|| {
                        format!(
                            "<svg class=\"shape\" style=\"width:{:.1}pt;height:{:.1}pt\" \
                             viewBox=\"0 0 {:.1} {:.1}\">\n",
                            size.width_pt, size.height_pt, size.width_pt, size.height_pt
                        )
                    });
                    svg.push_str(&path_element(paint));
                    index += 1;
                }
                Piece::Picture(picture) => {
                    if let Some(svg) = open_shapes.take() {
                        body.push_str(&svg);
                        body.push_str("</svg>\n");
                    }
                    body.push_str(&picture_tag(session, page, picture));
                    index += 1;
                }
                Piece::Run(run) => {
                    // **The pieces of one line ride together** — see
                    // `same_line` — so the reader's own text engine lays
                    // them out from the first piece's pen instead of each
                    // piece pinning its own position, which is where
                    // overlapping came from the moment the reader's fonts
                    // were not the file's.
                    let mut line: Vec<Run<'_>> = vec![run_of(run, &snapshot, &fonts, &families)];
                    index += 1;
                    while index < pieces.len() {
                        let Piece::Run(next) = pieces[index].2 else { break };
                        if !same_line(line.last().expect("a piece"), &run_of(next, &snapshot, &fonts, &families)) {
                            break;
                        }
                        line.push(run_of(next, &snapshot, &fonts, &families));
                        index += 1;
                    }
                    if let Some(svg) = open_shapes.take() {
                        body.push_str(&svg);
                        body.push_str("</svg>\n");
                    }
                    if line.len() == 1 {
                        body.push_str(&span_pinned(&line[0], &snapshot));
                    } else {
                        body.push_str(&line_group(&line, &snapshot));
                    }
                    placed += line.len();
                }
            }
        }
        if let Some(svg) = open_shapes.take() {
            body.push_str(&svg);
            body.push_str("</svg>\n");
        }
        if placed == 0 && pictures.is_empty() && paints.is_empty() {
            body.push_str("<p class=\"none\">no text on this page</p>\n");
        }
        body.push_str("</div>\n</section>\n");
        if !keep_going(page + 1, pages) {
            return Ok(None);
        }
    }

    let mut out = String::new();
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!("<title>{}</title>\n", escape(title)));
    out.push_str(STYLE);
    out.push_str(&faces.css());
    out.push_str("</head>\n<body>\n");
    out.push_str(&format!(
        "<header class=\"doc\">\n<h1>{}</h1>\n<p class=\"meta\">{} page{}&nbsp;· converted from the PDF</p>\n</header>\n",
        escape(title),
        pages,
        if pages == 1 { "" } else { "s" }
    ));
    out.push_str(&body);
    out.push_str("</body>\n</html>\n");
    Ok(Some(out))
}

/// One piece of a page, in the order the page draws it — see
/// [`document_html`]'s own note on the sort.
enum Piece<'a> {
    Run(&'a pdf_core::document::TextRun),
    Picture(&'a pdf_core::document::PageImage),
    Shape(&'a pdf_core::document::PagePaint),
}

/// One run ready to be written: the run, the face the page draws it with,
/// and the family its embedded program was declared under. See [`Run::css`]
/// for what each contributes to the span.
struct Run<'a> {
    run: &'a pdf_core::document::TextRun,
    style: Option<&'a pdf_core::document::RunStyle>,
    font: Option<&'a pdf_core::document::PageFont>,
    family: Option<&'a str>,
}

impl Run<'_> {
    /// The face's name as the page's fonts list gives it — **the program's
    /// own name with the subset tag dropped**, which is what a reader can
    /// match — falling back to the snapshot's raw name.
    fn name<'s>(&'s self, snapshot: &'s PageTextSnapshot) -> Option<&'s str> {
        self.font
            .map(|font| font.name.as_str())
            .filter(|name| !name.is_empty())
            .or_else(|| snapshot.faces.get(&self.run.object).map(String::as_str))
    }

    /// The span's whole style attribute body, placement left out: a run of
    /// its own is pinned by its caller, a piece of a line rides in the
    /// line's own pinned wrapper.
    fn css(&self, snapshot: &PageTextSnapshot) -> String {
        let run = self.run;
        let mut css = format!("font-size:{:.1}pt;", run.size.max(0.1));
        // **Every opaque colour is stated, including white.** The old test
        // skipped pure white as "the same as the sheet", but this stylesheet's
        // own body colour is dark: white lettering over a picture or a filled
        // heading would come out black. Transparency still gets `rgba`, and a
        // white run still reads as white.
        if run.color.a < 255 {
            css.push_str(&format!(
                "color:rgba({},{},{},{:.3});",
                run.color.r,
                run.color.g,
                run.color.b,
                f32::from(run.color.a) / 255.0
            ));
        } else {
            css.push_str(&format!("color:rgb({},{},{});", run.color.r, run.color.g, run.color.b));
        }

        // **The document's own program first, then the name.** A reader that
        // can use the embedded face gets the file's own letters and widths;
        // one that cannot falls back by name — and the name is single-quoted
        // because the whole declaration lives in a double-quoted HTML
        // attribute: a `"` in the CSS would end the attribute and leave the
        // rest parsed as stray attributes, the mistake this line already made
        // once.
        if let Some(name) = self.name(snapshot) {
            let quoted = name.replace('\\', "\\\\").replace('\'', "\\'");
            let family = match self.family {
                Some(family) => format!("'{family}',"),
                None => String::new(),
            };
            css.push_str(&format!("font-family:{family}'{quoted}',{};", family_of(name)));
        } else if let Some(family) = self.family {
            css.push_str(&format!("font-family:'{family}';"));
        }

        // **The face's own weight and angle where the font states them**, and
        // the measured stem only as the fallback the stem was always meant to
        // be. The same values are on the `@font-face` declaration of a
        // program that is embedded, so the reader selects that face exactly
        // and never synthesizes a bolder or slanted copy on top of it.
        let weight = self
            .font
            .map(|font| font.weight)
            .filter(|weight| *weight > 0)
            .or_else(|| self.style.and_then(|style| style.stem_milli_em).map(css_weight));
        if let Some(weight) = weight {
            css.push_str(&format!("font-weight:{weight};"));
        }
        if self.font.map(|font| font.italic).unwrap_or(false) {
            css.push_str("font-style:italic;");
        }
        css
    }
}

/// Whether `next` continues `previous`'s line: **one line of the page, in
/// pieces.**
///
/// A producer splits a line into several text objects whenever kerning or a
/// style change happens inside it, and every piece then carries its own pen
/// position. Written out as separate pinned spans they overlap the moment
/// the reader's fonts are not the file's — the reported "sometimes the texts
/// overlap" — so the pieces ride together in one pinned wrapper and the
/// reader's own text engine lays them out from the first piece's pen, as it
/// lays out any line of text.
///
/// Held to what the page itself says: the same baseline (within three
/// fifths of a point), both running horizontally, and the next starting
/// where the last left off — a gap a kerning move or a space wide, or a hair
/// of overlap from rounding. A larger gap is a column or a tab stop and is
/// left where the page put it. Mixed sizes are merged on purpose: pieces
/// that share a baseline share one in the reader's engine too.
fn same_line(previous: &Run<'_>, next: &Run<'_>) -> bool {
    let horizontal = |run: &Run<'_>| {
        run.style.map(|style| style.axis.0 > 0.999 && style.axis.1.abs() < 0.01).unwrap_or(true)
    };
    if !horizontal(previous) || !horizontal(next) {
        return false;
    }
    let baseline = (previous.run.origin.y - next.run.origin.y).abs() <= 0.6;
    let gap = next.run.rect.left - previous.run.rect.right;
    baseline && (-4.0..=6.0).contains(&gap)
}

/// One run of its own, pinned where the page has it: the box's top-left for
/// upright text, the baseline start plus a turn for text at an angle — see
/// [`document_html`].
fn span_pinned(piece: &Run<'_>, snapshot: &PageTextSnapshot) -> String {
    let run = piece.run;
    let axis = piece.style.map(|style| style.axis).unwrap_or((1.0, 0.0));
    let placement = if axis.0 > 0.999 && axis.1.abs() < 0.01 {
        format!("left:{:.1}pt;top:{:.1}pt;", run.rect.left, run.rect.top)
    } else {
        // In the file's own space y grows downwards, which is CSS's too, so
        // the style's axis is the turn directly.
        let degrees = axis.1.atan2(axis.0).to_degrees();
        format!(
            "left:{:.1}pt;top:{:.1}pt;transform:rotate({degrees:.1}deg);transform-origin:left bottom;",
            run.origin.x, run.origin.y
        )
    };
    format!(
        "<span class=\"run\" style=\"{}\">{}</span>\n",
        // **The whole declaration is HTML-escaped, not only the CSS-quoted
        // name.** A face's name comes from the document, and a `"` in it
        // (`#22` in a PDF name is legal) would otherwise end this attribute
        // and let the file after it be parsed as markup. The CSS-level `\'`
        // escaping is still right for the CSS string; this is the layer above.
        escape(&format!("position:absolute;{placement}{}", piece.css(snapshot))),
        escape(run.text.trim())
    )
}

/// The pieces of one line, riding together in a wrapper pinned where the
/// line starts.
///
/// **Only the first piece's words are trimmed.** The rest keep exactly what
/// the page holds — the space a piece starts with *is* the space between the
/// two words once they flow as one line (`white-space: pre` keeps it), and
/// trimming the pieces was how "market" came out glued to "editors".
fn line_group(line: &[Run<'_>], snapshot: &PageTextSnapshot) -> String {
    let first = line[0].run;
    let mut out = format!(
        "<span class=\"line\" style=\"left:{:.1}pt;top:{:.1}pt;\">",
        first.rect.left, first.rect.top
    );
    for (index, piece) in line.iter().enumerate() {
        let text = if index == 0 { piece.run.text.trim() } else { piece.run.text.as_str() };
        out.push_str(&format!(
            "<span class=\"run\" style=\"{}\">{}</span>",
            // Escaped for the same reason [`span_pinned`]'s is: the face name
            // is the document's, and this attribute is HTML.
            escape(&piece.css(snapshot)),
            escape(text)
        ));
    }
    out.push_str("</span>\n");
    out
}

/// The document's embedded font programs, one `@font-face` each.
///
/// **Document-wide and deduplicated.** [`Session::page_fonts`] numbers a
/// page's fonts from zero on every page, and the same program recurs on page
/// after page; a family per page would carry the same hundred kilobytes a
/// hundred times. The key is the program's own bytes (an FNV-1a hash), and
/// the family is that hash spelled out, so two pages naming the same program
/// agree without ever having seen each other.
#[derive(Default)]
struct FontSheet {
    /// Family, program, weight, italic — in the order first declared.
    faces: Vec<(String, Vec<u8>, u32, bool)>,
    families: std::collections::HashMap<u64, String>,
}

impl FontSheet {
    /// Declares a page's fonts and answers, per id, the family a run's span
    /// should name — `None` for a font the document embeds no program for
    /// (the reader substitutes by name, which is the honest fallback) and for
    /// one whose program a browser does not load.
    fn register(&mut self, fonts: &[pdf_core::document::PageFont]) -> Vec<Option<String>> {
        fonts.iter().map(|font| self.declare(font)).collect()
    }

    fn declare(&mut self, font: &pdf_core::document::PageFont) -> Option<String> {
        let program = font.program.as_deref()?;
        embeddable_font(program)?;
        let key = fnv1a(program);
        if let Some(family) = self.families.get(&key) {
            return Some(family.clone());
        }
        let family = format!("pdf-{key:016x}");
        self.faces.push((family.clone(), program.to_vec(), font.weight, font.italic));
        self.families.insert(key, family.clone());
        Some(family)
    }

    /// The `@font-face` rules, or nothing when the document embeds no
    /// programs at all — an empty `<style>` would only be noise.
    fn css(&self) -> String {
        if self.faces.is_empty() {
            return String::new();
        }
        let mut out = String::from("<style>\n");
        for (family, program, weight, italic) in &self.faces {
            let (mime, format) =
                embeddable_font(program).expect("only embeddable programs are declared");
            out.push_str(&format!("@font-face {{ font-family: '{family}';"));
            if *weight > 0 {
                out.push_str(&format!(" font-weight: {weight};"));
            }
            if *italic {
                out.push_str(" font-style: italic;");
            }
            out.push_str(&format!(
                " src: url(data:{mime};base64,{}) format('{format}'); }}\n",
                base64(program)
            ));
        }
        out.push_str("</style>\n");
        out
    }
}

/// Whether a font program is one `@font-face` can carry, and how to label it
/// — TrueType's `0x00010000` or `true`, OpenType's `OTTO` — or `None` for
/// what browsers do not load: a bare CFF, a Type 1 program, a collection.
fn embeddable_font(program: &[u8]) -> Option<(&'static str, &'static str)> {
    match program.get(..4)? {
        [0x00, 0x01, 0x00, 0x00] | b"true" => Some(("font/ttf", "truetype")),
        b"OTTO" => Some(("font/otf", "opentype")),
        _ => None,
    }
}

/// FNV-1a over the program bytes — the key a family name is made of.
fn fnv1a(data: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// The file's whole stylesheet, inline — nothing to fetch, and no font files
/// to carry: a light page look for screen and a clean one for paper. Sheets
/// are sized in real points, the same point a PDF uses, so what is laid out
/// here prints the size the page says it is.
const STYLE: &str = r#"<style>
:root { color-scheme: light; }
* { box-sizing: border-box; }
html { background: #e9e6e0; }
body { margin: 0; color: #201e1b;
  font: 15px/1.5 -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif; }
header.doc { max-width: 960px; margin: 28px auto 0; padding: 0 20px; }
header.doc h1 { margin: 0; font-size: 21px; font-weight: 600; letter-spacing: -0.01em; }
header.doc .meta { margin: 4px 0 6px; color: #746e66; font-size: 13px; }
.page { width: fit-content; margin: 18px auto; }
.page h2 { margin: 0 0 6px; color: #8b857c; font-size: 11px; font-weight: 600;
  letter-spacing: 0.09em; text-transform: uppercase; }
.sheet { position: relative; overflow: hidden; background: #fff;
  border: 1px solid #d8d3ca; box-shadow: 0 1px 5px rgba(30, 25, 20, 0.16); }
.run { white-space: pre; line-height: 1; }
.line { position: absolute; white-space: pre; line-height: 1; }
.shape { position: absolute; left: 0; top: 0; pointer-events: none; }
.pic { position: absolute; transform-origin: 0 0; }
.gone { position: absolute; display: flex; align-items: center; justify-content: center;
  border: 1px dashed #c9c2b6; color: #a09a90; font-size: 11px; }
.none { position: absolute; inset: 0; display: flex; align-items: center;
  justify-content: center; color: #a09a90; font-style: italic; }
@media print {
  html, body { background: #fff; }
  header.doc { margin: 0 0 10px; padding: 0; }
  .page { margin: 0 auto; }
  .page:not(:last-child) { break-after: page; }
  .sheet { border: 0; box-shadow: none; }
}
</style>
"#;

/// One path as an SVG `<path>`: the `d` its segments spell, then the paint
/// its own fields carry — fill (with its rule and opacity), stroke (with its
/// colour, width, dash, cap and join). Attributes that would say a default
/// are left out, so a black rule is one short element.
fn path_element(paint: &pdf_core::document::PagePaint) -> String {
    use pdf_core::document::{LineCap, LineJoin, PaintSegment};

    let mut d = String::new();
    let point = |point: pdf_core::document::Point| format!("{:.1} {:.1} ", point.x, point.y);
    for segment in &paint.segments {
        match *segment {
            PaintSegment::Move(to) => {
                d.push('M');
                d.push_str(&point(to));
            }
            PaintSegment::Line { to, close } => {
                d.push('L');
                d.push_str(&point(to));
                if close {
                    d.push('Z');
                }
            }
            PaintSegment::Curve { control1, control2, to } => {
                d.push('C');
                d.push_str(&point(control1));
                d.push_str(&point(control2));
                d.push_str(&point(to));
            }
        }
    }

    let mut attrs = String::new();
    match paint.fill {
        Some(fill) => {
            attrs.push_str(&format!(" fill=\"#{:02x}{:02x}{:02x}\"", fill.r, fill.g, fill.b));
            if paint.even_odd {
                attrs.push_str(" fill-rule=\"evenodd\"");
            }
            if fill.a < 255 {
                attrs.push_str(&format!(" fill-opacity=\"{:.3}\"", f32::from(fill.a) / 255.0));
            }
        }
        None => attrs.push_str(" fill=\"none\""),
    }
    if let Some(stroke) = paint.stroke {
        attrs.push_str(&format!(
            " stroke=\"#{:02x}{:02x}{:02x}\" stroke-width=\"{:.2}\"",
            stroke.r, stroke.g, stroke.b, paint.stroke_width
        ));
        if stroke.a < 255 {
            attrs.push_str(&format!(" stroke-opacity=\"{:.3}\"", f32::from(stroke.a) / 255.0));
        }
        if !paint.dash.is_empty() {
            let pattern: Vec<String> = paint.dash.iter().map(|length| format!("{length:.2}")).collect();
            attrs.push_str(&format!(" stroke-dasharray=\"{}\"", pattern.join(" ")));
        }
        match paint.line_cap {
            LineCap::Round => attrs.push_str(" stroke-linecap=\"round\""),
            LineCap::Square => attrs.push_str(" stroke-linecap=\"square\""),
            LineCap::Butt => {}
        }
        match paint.line_join {
            LineJoin::Round => attrs.push_str(" stroke-linejoin=\"round\""),
            LineJoin::Bevel => attrs.push_str(" stroke-linejoin=\"bevel\""),
            LineJoin::Miter => {}
        }
    }
    format!("<path d=\"{}\"{}/>\n", d.trim_end(), attrs)
}

/// One picture as an `<img>`: its pixels embedded as a data URI — the
/// stream's own JPEG where a browser shows that as it is, the decoded pixels
/// as a PNG otherwise — placed by the matrix that put it on the page, so a
/// turned or skewed picture stays turned or skewed.
///
/// **Placement is the matrix over the page's own box.** An image occupies the
/// **unit square** in PDF space (its matrix is what scales that square to the
/// page — a 4×4-pixel bitmap at 120×60 points of page still has `a, d = 120,
/// 60`), so the element is 1×1 and the matrix's linear part does the rest.
/// The translation is the one part CSS needs in pixels, and it is taken from
/// [`picture_placement`] — the page's reported rect minus the matrix's own
/// smallest offsets — so a turn or a skew (nonzero `b`/`c`) and a cropped
/// page both land the picture where the page has it.
///
/// A picture whose pixels cannot be expanded (an unusual colour space, a
/// JPEG 2000) leaves a dashed placeholder where it belongs rather than
/// nothing: the page saying it had a picture is worth keeping.
fn picture_tag(
    session: &Session,
    page: usize,
    picture: &pdf_core::document::PageImage,
) -> String {
    let embedded = session
        .image_raw(page, picture.object)
        .ok()
        .flatten()
        .or_else(|| {
            session
                .export_image_png(page, picture.object)
                .ok()
                .map(|png| (png, "image/png"))
        });
    let (bytes, mime) = match embedded {
        Some(embedded) => embedded,
        None => {
            let (width, height) = (
                (picture.rect.right - picture.rect.left).max(0.0),
                (picture.rect.bottom - picture.rect.top).max(0.0),
            );
            return format!(
                "<span class=\"gone\" style=\"left:{:.1}pt;top:{:.1}pt;width:{:.1}pt;height:{:.1}pt\">picture</span>\n",
                picture.rect.left, picture.rect.top, width, height
            );
        }
    };

    let (tx_pt, ty_pt, [a, b, c, d]) = picture_placement(picture.matrix, &picture.rect);
    // Only the translation is in CSS pixels; the matrix's linear part scales
    // the element's own points (1×1 pt) to its size on the page.
    let to_pixels = 96.0 / 72.0;
    format!(
        "<img class=\"pic\" alt=\"\" style=\"left:0;top:0;width:1pt;height:1pt;\
         transform:translate({:.3}px,{:.3}px) matrix({a:.5},{b:.5},{c:.5},{d:.5},0,0);\" \
         src=\"data:{mime};base64,{}\">\n",
        tx_pt * to_pixels,
        ty_pt * to_pixels,
        base64(&bytes)
    )
}

/// Where [`picture_tag`]'s element is translated to, in page points, and the
/// `matrix()` linear terms that then scale it — the arithmetic a browser does
/// with the emitted `style`.
///
/// The element is a 1×1-point box whose top-left is its own `(0, 0)`, and the
/// CSS transform maps its local `(u, q)` (`u` right, `q` down, both 0..=1) to
/// `(T + a·u - c·q, T' - b·u + d·q)` — the y flip and the matrix's own terms.
/// The picture's **page box** (the same top-left space every other piece of
/// this export is laid out in) is `rect`; setting the translation to `rect`'s
/// top-left minus the linear part's own minimum offsets puts the transformed
/// box exactly on it. That is what makes rotated, skewed and cropped-page
/// pictures land right: the old formula used `e` and the sheet height alone,
/// so a nonzero `c` (any turned picture) shifted it sideways by its own width,
/// and a crop box not at the origin shifted every picture by the crop offset.
fn picture_placement(
    matrix: [f32; 6],
    rect: &pdf_core::document::Rect,
) -> (f32, f32, [f32; 4]) {
    let [a, b, c, d, _e, _f] = matrix;
    let min_x = 0.0f32.min(a).min(-c);
    let min_y = 0.0f32.min(-b).min(d);
    (rect.left - min_x, rect.top - min_y, [a, -b, -c, d])
}

/// Base64, the alphabet and padding a data URI uses.
///
/// **Written out rather than pulled in**: it is the whole of what the export
/// needs from a base64 crate, it is twenty lines, and a dependency for it
/// would be one more thing to keep current in a binary that already carries
/// a PDF engine.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        out.push(ALPHABET[(bits >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(bits >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(bits >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(bits & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// The CSS weight whose measured stem is nearest.
///
/// The named points are the ones
/// [`pdf_core::document::RunStyle::stem_milli_em`] records, from a real
/// typeface family — Thin 20, Light 51, Regular 74, Medium 100, SemiBold
/// 124, ExtraBold 198. Nearest wins; a stem heavier than every named point
/// reads as the heaviest. Nothing here is a claim about the face beyond what
/// its own font program measured.
fn css_weight(stem: u16) -> u32 {
    const NAMED: [(u16, u32); 6] = [
        (20, 100),
        (51, 300),
        (74, 400),
        (100, 500),
        (124, 600),
        (198, 800),
    ];
    NAMED
        .iter()
        .min_by_key(|(milli, _)| milli.abs_diff(stem))
        .map(|(_, weight)| *weight)
        .unwrap_or(400)
}

/// What to fall back on when the browser has not got the run's own face.
///
/// Nothing in a PDF states a family, so this reads the name — and only for
/// the kinds of name that say clearly what they are: a face whose name
/// mentions none of these is treated as a sans.
fn family_of(face: &str) -> &'static str {
    let lower = face.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    if has(&["times", "serif", "georgia", "garamond", "palatino", "bookman"]) {
        "serif"
    } else if has(&["courier", "mono", "consol", "menlo"]) {
        "monospace"
    } else {
        "sans-serif"
    }
}

/// Hard-wrap `line` at `width` characters, on character boundaries.
///
/// `fromclipboard`'s own wrap, and the reason it is a character count rather
/// than a measurement: this build writes lines it cannot ask the width of
/// (see the `pagemarks` parse arm), so a wrap point can only be a count. It
/// is a formatting choice, not a typographic claim — anything that needs
/// better goes through the editor.
pub fn wrap_by_chars(line: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![line.to_string()];
    }
    let chars: Vec<char> = line.chars().collect();
    chars.chunks(width).map(|chunk| chunk.iter().collect()).collect()
}

/// The document's text in one of the plain formats — the Conversion tab's
/// To Other: `.txt`, `.rtf` or `.xml`, chosen by the extension the caller
/// asks for. `.html` is [`document_html`]'s.
pub fn document_other(session: &Session, extension: &str) -> pdf_core::Result<String> {
    Ok(document_other_watched(session, extension, &mut |_, _| true)?
        .expect("a caller that never stops gets its text"))
}

/// [`document_other`] with its steps reported and the caller able to put it
/// down, exactly as [`document_html_watched`] is to [`document_html`] —
/// reading every page's text is the slow half, and a stopped read answers
/// `None` rather than half a document.
pub fn document_other_watched(
    session: &Session,
    extension: &str,
    keep_going: &mut dyn FnMut(usize, usize) -> bool,
) -> pdf_core::Result<Option<String>> {
    let page_count = session.page_count()?;
    let mut pages: Vec<Vec<String>> = Vec::with_capacity(page_count);
    for page in 0..page_count {
        let snapshot = session.page_text_snapshot(page)?;
        pages.push(
            snapshot
                .runs
                .iter()
                .map(|run| run.text.trim().to_string())
                .filter(|text| !text.is_empty())
                .collect(),
        );
        if !keep_going(page + 1, page_count) {
            return Ok(None);
        }
    }

    let text = match extension.to_ascii_lowercase().as_str() {
        "txt" => pages
            .iter()
            .map(|lines| lines.join("\n"))
            .collect::<Vec<_>>()
            .join("\n\n"),
        "rtf" => {
            let mut out = String::from("{\\rtf1\\ansi\n");
            for (page, lines) in pages.iter().enumerate() {
                if page > 0 {
                    out.push_str("\\page\n");
                }
                for line in lines {
                    out.push_str(&escape_rtf(line));
                    out.push_str("\\par\n");
                }
            }
            out.push('}');
            out
        }
        "xml" => {
            let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<document>\n");
            for (page, lines) in pages.iter().enumerate() {
                out.push_str(&format!("  <page n=\"{}\">\n", page + 1));
                for line in lines {
                    out.push_str(&format!("    <line>{}</line>\n", escape_xml(line)));
                }
                out.push_str("  </page>\n");
            }
            out.push_str("</document>\n");
            out
        }
        other => {
            return Err(pdf_core::PdfError::InvalidArgument(format!(
                "toother writes .txt, .rtf and .xml — not `.{other}`; .html is `tohtml`"
            )))
        }
    };
    Ok(Some(text))
}

/// RTF: backslash, braces and anything above ASCII, the way RTF spells them.
pub(crate) fn escape_rtf(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            c if (c as u32) < 128 => out.push(c),
            c => out.push_str(&format!("\\u{}?", c as u32)),
        }
    }
    out
}

/// XML: the four markup characters, nothing more.
pub(crate) fn escape_xml(text: &str) -> String {
    let mut escaped = text.replace('&', "&amp;");
    escaped = escaped.replace('<', "&lt;").replace('>', "&gt;");
    escaped.replace('"', "&quot;")
}

/// Four characters stand between text and markup; everything else passes
/// through as it is. Shared with the accessibility report, which writes
/// markup too.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod other_format_tests {
    use super::*;

    #[test]
    fn the_rtf_escape_spells_markup_and_unicode() {
        assert_eq!(escape_rtf("a\\b{c}"), "a\\\\b\\{c\\}");
        assert_eq!(escape_rtf("café"), "caf\\u233?");
    }

    #[test]
    fn the_xml_escape_covers_the_four_characters() {
        assert_eq!(escape_xml("a<b>&\"c\""), "a&lt;b&gt;&amp;&quot;c&quot;");
    }

    #[test]
    fn a_measured_stem_lands_on_its_nearest_named_weight() {
        assert_eq!(css_weight(74), 400, "Regular");
        assert_eq!(css_weight(51), 300, "Light");
        assert_eq!(css_weight(124), 600, "SemiBold");
        // Between points, the nearer one; past every point, the heaviest.
        assert_eq!(css_weight(80), 400);
        assert_eq!(css_weight(110), 500);
        assert_eq!(css_weight(500), 800);
    }

    #[test]
    fn a_family_falls_back_by_what_its_name_says() {
        assert_eq!(family_of("Times New Roman"), "serif");
        assert_eq!(family_of("Garamond-Bold"), "serif");
        assert_eq!(family_of("Courier New"), "monospace");
        assert_eq!(family_of("Montserrat"), "sans-serif");
    }
}

#[cfg(test)]
mod html_tests {
    use super::*;
    use pdf_core::document::{Color, Point, Rect, RunStyle, TextRun};

    fn run() -> TextRun {
        TextRun {
            object: 0,
            text: "words".into(),
            rect: Rect { left: 10.0, top: 20.0, right: 60.0, bottom: 32.0 },
            origin: Point { x: 10.0, y: 30.0 },
            size: 12.0,
            color: Color { r: 0, g: 0, b: 0, a: 255 },
        }
    }

    fn snapshot(run: TextRun, style: RunStyle) -> PageTextSnapshot {
        PageTextSnapshot {
            runs: vec![run.clone()],
            styles: std::collections::HashMap::from([(run.object, style)]),
            faces: std::collections::HashMap::from([(run.object, "Montserrat".to_string())]),
            shapes: Vec::new(),
        }
    }

    const UPRIGHT: RunStyle = RunStyle { font: 0, stem_milli_em: None, axis: (1.0, 0.0) };

    #[test]
    fn an_upright_run_is_pinned_to_its_own_box() {
        let snapshot = snapshot(run(), UPRIGHT);
        let piece =
            Run { run: &snapshot.runs[0], style: Some(&UPRIGHT), font: None, family: None };
        let span = span_pinned(&piece, &snapshot);
        assert!(span.contains("position:absolute;left:10.0pt;top:20.0pt;"), "{span}");
        assert!(span.contains("font-size:12.0pt;"), "{span}");
        assert!(!span.contains("transform"), "upright text needs no turn: {span}");
        assert!(span.contains("font-family:'Montserrat',sans-serif;"), "{span}");
    }

    #[test]
    fn a_turned_run_is_pinned_to_its_baseline_and_turned() {
        let mut run = run();
        run.origin = Point { x: 40.0, y: 90.0 };
        let style = RunStyle { axis: (0.0, -1.0), ..UPRIGHT };
        let snapshot = snapshot(run, style);
        let piece =
            Run { run: &snapshot.runs[0], style: Some(&style), font: None, family: None };
        let span = span_pinned(&piece, &snapshot);
        assert!(span.contains("left:40.0pt;top:90.0pt;"), "{span}");
        assert!(span.contains("transform:rotate(-90.0deg);"), "{span}");
        assert!(span.contains("transform-origin:left bottom;"), "{span}");
    }

    /// The face's own stated weight wins; the measured stem is the fallback
    /// it was always meant to be; and an italic face says so.
    #[test]
    fn the_face_s_weight_and_angle_reach_the_span() {
        use pdf_core::document::PageFont;
        let stated = PageFont {
            id: 0,
            name: "Montserrat-SemiBoldItalic".into(),
            program: None,
            weight: 600,
            italic: true,
        };
        let style = RunStyle { stem_milli_em: Some(124), ..UPRIGHT };
        let snapshot = snapshot(run(), style);
        let piece = Run {
            run: &snapshot.runs[0],
            style: Some(&style),
            font: Some(&stated),
            family: None,
        };
        let span = span_pinned(&piece, &snapshot);
        assert!(span.contains("font-weight:600;"), "{span}");
        assert!(span.contains("font-style:italic;"), "{span}");
        assert!(span.contains("font-family:'Montserrat-SemiBoldItalic',sans-serif;"), "{span}");

        // No stated weight: the stem still answers, as it did before.
        let stem_only = Run { font: None, ..piece };
        let span = span_pinned(&stem_only, &snapshot);
        assert!(span.contains("font-weight:600;"), "stem 124 reads SemiBold: {span}");
    }

    /// The RFC 4648 test vectors — the shortest ones that exercise every
    /// padding case.
    #[test]
    fn base64_matches_the_rfc_s_own_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// **The same-line test**: pieces of one line ride together, and
    /// everything that is not that line's continuation does not. Wrong here
    /// is text overlapping (too eager) or a column dragged onto a heading
    /// (too shy).
    #[test]
    fn the_same_line_test_holds_pieces_together_and_splits_everything_else() {
        let upright = RunStyle { font: 0, stem_milli_em: None, axis: (1.0, 0.0) };
        let piece = |text: &'static str, left: f32, right: f32, baseline: f32| TextRun {
            object: 0,
            text: text.into(),
            rect: Rect { left, top: baseline - 10.0, right, bottom: baseline + 2.0 },
            origin: Point { x: left, y: baseline },
            size: 12.0,
            color: Color { r: 0, g: 0, b: 0, a: 255 },
        };
        fn run_of<'a>(run: &'a TextRun, style: &'a RunStyle) -> Run<'a> {
            Run { run, style: Some(style), font: None, family: None }
        }

        let first = piece("Th", 72.0, 87.0, 30.0);
        let joined = piece("e editors", 87.3, 130.0, 30.0);
        assert!(same_line(&run_of(&first, &upright), &run_of(&joined, &upright)), "a kerned split is one line");

        let next_row = piece("Below", 72.0, 100.0, 44.0);
        assert!(!same_line(&run_of(&first, &upright), &run_of(&next_row, &upright)), "another baseline is another line");

        let far = piece("Right", 300.0, 330.0, 30.0);
        assert!(!same_line(&run_of(&first, &upright), &run_of(&far, &upright)), "a column is not that line's next word");

        let mut turned = piece("side", 87.3, 100.0, 30.0);
        turned.origin = Point { x: 40.0, y: 90.0 };
        let turned_style = RunStyle { axis: (0.0, -1.0), ..upright };
        let turned_run = Run { run: &turned, style: Some(&turned_style), font: None, family: None };
        assert!(
            !same_line(&run_of(&first, &upright), &turned_run),
            "turned text is pinned on its own"
        );
    }

    /// Only programs a browser loads are declared — and only once, however
    /// many pages hold them.
    #[test]
    fn only_loadable_programs_are_declared_and_only_once() {
        assert!(embeddable_font(&[0x00, 0x01, 0x00, 0x00, 0, 0]).is_some(), "TrueType");
        assert!(embeddable_font(b"true, and more").is_some(), "the other TrueType tag");
        assert!(embeddable_font(b"OTTO, and more").is_some(), "OpenType");
        assert!(embeddable_font(b"ttcf, and more").is_none(), "a collection");
        assert!(embeddable_font(&[0x01, 0x00, 0x04, 0x00]).is_none(), "a bare CFF");
        assert!(embeddable_font(b"%!PS-AdobeFont").is_none(), "a Type 1 program");

        let font = |id: u32| pdf_core::document::PageFont {
            id,
            name: "Montserrat".into(),
            program: Some(vec![0x00, 0x01, 0x00, 0x00, 42]),
            weight: 400,
            italic: false,
        };
        let mut sheet = FontSheet::default();
        let first_page = sheet.register(&[font(0)]);
        let second_page = sheet.register(&[font(0), font(1)]);
        assert_eq!(first_page[0], second_page[0], "a page names the declared family");
        assert_eq!(second_page[1], second_page[0], "one program, one family");
        assert_eq!(sheet.css().matches("@font-face").count(), 1, "declared twice");
        assert!(sheet.css().contains("font-weight: 400"), "{}", sheet.css());

        let bare = pdf_core::document::PageFont { program: None, ..font(2) };
        assert_eq!(sheet.register(&[bare]), vec![None], "a font with no program is not declared");
    }

    /// **White text stays white.** The old stylesheet skipped pure white as if
    /// the sheet were its background; this stylesheet's body colour is dark, so
    /// a white heading over a picture or a filled box came out black.
    #[test]
    fn a_white_run_stays_white_in_the_export() {
        let mut white = run();
        white.color = Color { r: 255, g: 255, b: 255, a: 255 };
        let snapshot = snapshot(white, UPRIGHT);
        let piece =
            Run { run: &snapshot.runs[0], style: Some(&UPRIGHT), font: None, family: None };
        let span = span_pinned(&piece, &snapshot);
        assert!(span.contains("color:rgb(255,255,255);"), "white text lost its colour: {span}");
    }

    /// **A face name is the document's to choose, and it is not markup.** A
    /// PDF `/BaseFont` can decode to a quote (`#22`), which used to end the
    /// `style="…"` attribute and let the rest of the name be parsed as tags.
    #[test]
    fn a_face_name_cannot_break_out_of_the_style_attribute() {
        use pdf_core::document::PageFont;
        let hostile = PageFont {
            id: 0,
            name: "x\"><img src=x onerror=alert(1)>".into(),
            program: None,
            weight: 0,
            italic: false,
        };
        let snapshot = snapshot(run(), UPRIGHT);
        let piece = Run {
            run: &snapshot.runs[0],
            style: Some(&UPRIGHT),
            font: Some(&hostile),
            family: None,
        };
        let span = span_pinned(&piece, &snapshot);
        assert!(!span.contains("\"><img"), "the name escaped the attribute: {span}");
        assert!(!span.contains("<img"), "a tag came out of the face name: {span}");
        assert!(span.contains("&quot;"), "the quote was not HTML-escaped: {span}");
        assert_eq!(span.matches("style=\"").count(), 1, "the attribute was split: {span}");
    }

    /// **A turned picture sits on the box the page reports.** The old
    /// translation used the matrix's `e` and the sheet height, which ignores
    /// `c`: a 90°-turned picture came out one width to the left.
    #[test]
    fn a_rotated_picture_lands_on_the_box_the_page_reports() {
        // The probe's own numbers: a 80×40 picture turned clockwise, whose
        // page box is (120, 180)..(160, 260).
        let matrix = [0.0, -80.0, 40.0, 0.0, 120.0, 662.0];
        let rect = Rect { left: 120.0, top: 180.0, right: 160.0, bottom: 260.0 };
        let (tx, ty, linear) = picture_placement(matrix, &rect);
        assert_eq!((tx, ty), (160.0, 180.0), "translation misses the matrix's c");
        assert_eq!(linear, [0.0, 80.0, -40.0, 0.0], "the linear terms are the PDF matrix mapped to CSS");

        // And the element's transformed corners really cover the rect.
        let corners: Vec<(f32, f32)> = [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
            .map(|(u, q)| (tx + linear[0] * u + linear[2] * q, ty + linear[1] * u + linear[3] * q))
            .to_vec();
        let left = corners.iter().map(|p| p.0).fold(f32::MAX, f32::min);
        let right = corners.iter().map(|p| p.0).fold(f32::MIN, f32::max);
        let top = corners.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        let bottom = corners.iter().map(|p| p.1).fold(f32::MIN, f32::max);
        assert!(
            (left - rect.left).abs() < 0.01
                && (right - rect.right).abs() < 0.01
                && (top - rect.top).abs() < 0.01
                && (bottom - rect.bottom).abs() < 0.01,
            "the transformed box ({left},{top})..({right},{bottom}) is not the page's {rect:?}"
        );
    }

    /// **The page's box is the crop-adjusted one**, so a picture on a page
    /// whose CropBox does not start at the origin lands where the page shows
    /// it — the old formula subtracted only the sheet height.
    #[test]
    fn a_picture_on_a_cropped_page_uses_the_reported_box() {
        let matrix = [200.0, 0.0, 0.0, 150.0, 100.0, 400.0];
        // The same picture after a 50 x / 90 y crop: images_on reports the
        // crop-relative box, so the export must use it too.
        let cropped = Rect { left: 50.0, top: 152.0, right: 250.0, bottom: 302.0 };
        let (tx, ty, linear) = picture_placement(matrix, &cropped);
        assert_eq!((tx, ty), (50.0, 152.0), "the crop origin was not taken from the page's box");
        assert_eq!(linear, [200.0, 0.0, 0.0, 150.0]);
    }

    /// A path's `d` spells its segments in order — a closing line ends its
    /// sub-path — and its attributes carry the paint it was drawn with.
    #[test]
    fn a_path_spells_its_outline_and_its_paint() {        use pdf_core::document::{LineJoin, PagePaint, PaintSegment};
        let paint = PagePaint {
            object: 0,
            rect: Rect { left: 10.0, top: 20.0, right: 110.0, bottom: 20.0 },
            segments: vec![
                PaintSegment::Move(Point { x: 10.0, y: 20.0 }),
                PaintSegment::Line { to: Point { x: 110.0, y: 20.0 }, close: true },
            ],
            fill: Some(Color { r: 255, g: 0, b: 0, a: 128 }),
            even_odd: true,
            stroke: Some(Color { r: 0, g: 0, b: 255, a: 255 }),
            stroke_width: 1.5,
            dash: vec![3.0, 2.0],
            line_cap: pdf_core::document::LineCap::Round,
            line_join: LineJoin::Bevel,
        };
        let element = path_element(&paint);
        assert!(element.contains("d=\"M10.0 20.0 L110.0 20.0 Z\""), "{element}");
        assert!(element.contains("fill=\"#ff0000\""), "{element}");
        assert!(element.contains("fill-rule=\"evenodd\""), "{element}");
        assert!(element.contains("fill-opacity=\"0.502\""), "{element}");
        assert!(element.contains("stroke=\"#0000ff\""), "{element}");
        assert!(element.contains("stroke-width=\"1.50\""), "{element}");
        assert!(element.contains("stroke-dasharray=\"3.00 2.00\""), "{element}");
        assert!(element.contains("stroke-linecap=\"round\""), "{element}");
        assert!(element.contains("stroke-linejoin=\"bevel\""), "{element}");
    }
}
