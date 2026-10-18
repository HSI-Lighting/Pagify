//! The Conversion tab's two exports, at the shell level: `toimage`'s PNG
//! bytes and `tohtml`'s document.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test document_export
//! ```

use pagify_shell::{export, Session};

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn a_page_encodes_as_a_png() {
    let session = Session::open(fixture("quadrants.pdf")).expect("open");

    let bytes = session.export_page_png(0, 2.0).expect("png");

    assert!(
        bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "not a PNG signature: {:?}",
        &bytes[..8.min(bytes.len())]
    );
    assert!(
        bytes.len() > 1000,
        "a 400pt page at 2x is not {} bytes — the render or encode came back empty",
        bytes.len()
    );
}

#[test]
fn the_html_carries_the_page_text_and_escapes_its_title() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    let html = export::document_html(&session, "a<b&c").expect("html");

    assert!(html.contains("<title>a&lt;b&amp;c</title>"), "title not escaped: {html}");
    assert!(html.contains("<h2>Page 1</h2>"), "no page heading: {html}");
    assert!(
        html.contains("class=\"run\"") && !html.contains("no text on this page"),
        "page 1 has text and the export did not place it: {html}"
    );
    assert!(html.contains("<style>"), "no stylesheet came with the file: {html}");
    assert!(html.trim_end().ends_with("</html>"), "the document is not closed: {html}");
}

/// **Each run where the page has it, at its own size.** The old export
/// dropped every run into one paragraph per page — the words all there, the
/// page's own look all gone. This holds the new one to the page's own
/// numbers: the sheet the page's size, the first run's span at the run's own
/// box and size, and every run's words present.
#[test]
fn the_html_places_each_run_where_the_page_has_it() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    let html = export::document_html(&session, "two-column").expect("html");

    let size = session.page_size(0).expect("size");
    assert!(
        html.contains(&format!("width:{:.1}pt;height:{:.1}pt", size.width_pt, size.height_pt)),
        "the sheet is not the page's own size: {html}"
    );

    let snapshot = session.page_text_snapshot(0).expect("snapshot");
    let run = snapshot
        .runs
        .iter()
        .find(|run| !run.text.trim().is_empty())
        .expect("a run with words on it");
    let expected = format!(
        "left:{:.1}pt;top:{:.1}pt;font-size:{:.1}pt",
        run.rect.left,
        run.rect.top,
        run.size.max(0.1)
    );
    assert!(
        html.contains(&expected),
        "the run is not placed at its own place and size ({expected}): {html}"
    );
    assert!(
        html.contains(&format!(">{}</span>", run.text.trim())),
        "the placed span does not carry its own words: {html}"
    );

    // Nothing was dropped on the way: every run's words are in the file.
    for run in &snapshot.runs {
        let text = run.text.trim();
        if !text.is_empty() {
            assert!(html.contains(text), "the export lost a run: {text:?}");
        }
    }

    // The style attribute is double-quoted HTML, so no `"` may appear inside
    // it — the family name is written single-quoted for exactly that reason.
    assert!(
        !html.contains("font-family:\""),
        "a double quote in the style attribute ends it early: {html}"
    );
    assert!(
        html.contains("font-family:'"),
        "the run's own face must be named: {html}"
    );
}

#[test]
fn a_page_with_no_text_says_so_rather_than_going_missing() {
    // quadrants is four solid colours — a drawing, not a blank page.
    let session = Session::open(fixture("quadrants.pdf")).expect("open");

    let html = export::document_html(&session, "quadrants").expect("html");

    assert!(
        !html.contains("no text on this page"),
        "a page with drawing on it is not an empty page: {html}"
    );
}

/// **The page's rules, boxes and filled panels are drawn, not dropped.** The
/// old export was the words alone; a datasheet's table rules and panels
/// vanished with it. Each path now rides in an SVG of the page's own size,
/// with the geometry and the paint the page gives it.
#[test]
fn the_html_draws_the_page_s_paths_as_svg() {
    let session = Session::open(fixture("quadrants.pdf")).expect("open");

    let html = export::document_html(&session, "quadrants").expect("html");

    let paints = session.page_paints(0).expect("paints");
    assert!(!paints.is_empty(), "quadrants is made of four filled rectangles");
    assert!(paints.iter().all(|paint| paint.fill.is_some()), "every quadrant is filled");

    assert!(html.contains("<svg class=\"shape\""), "no shape svg came out: {html}");
    assert!(html.contains("<path d=\"M"), "no path was drawn: {html}");
    let fill = paints[0].fill.expect("a filled quadrant");
    assert!(
        html.contains(&format!("fill=\"#{:02x}{:02x}{:02x}\"", fill.r, fill.g, fill.b)),
        "the first quadrant's own colour is not in the file: {html}"
    );
    // Four filled rectangles, each with four corners and a closing side.
    assert_eq!(html.matches("<path ").count(), paints.len(), "{html}");
}

/// **A page's pictures are embedded in it.** The stream's own JPEG where a
/// browser shows that as it is, the decoded pixels as a PNG otherwise — as a
/// data URI, so the file stays one file — placed by the matrix that put the
/// picture on the page, so a turned picture stays turned.
#[test]
fn the_html_embeds_the_page_s_pictures() {
    let session = Session::open(fixture("pictures.pdf")).expect("open");

    let html = export::document_html(&session, "pictures").expect("html");

    let pictures = session.images_on(0).expect("images");
    assert!(!pictures.is_empty(), "pictures.pdf carries pictures");
    assert_eq!(html.matches("<img class=\"pic\"").count(), pictures.len(), "{html}");
    assert!(html.contains("src=\"data:image/"), "the pixels are not embedded: {html}");
    assert!(
        html.contains("transform:translate(") && html.contains("matrix("),
        "the picture is not placed by its own matrix: {html}"
    );

    // **The data URI carries real image bytes**: the payload has to start
    // with the base64 of a PNG or JPEG signature, not with an empty string
    // that only looks embedded.
    let payload = html
        .split("src=\"data:image/")
        .nth(1)
        .and_then(|rest| rest.split(";base64,").nth(1))
        .expect("a data URI");
    assert!(
        payload.starts_with("iVBORw0KGgo") || payload.starts_with("/9j/"),
        "the embedded bytes do not start like a PNG or a JPEG: {}",
        &payload[..20.min(payload.len())]
    );

    // **The placement really is the page's own**: the img's transform, taken
    // apart, must put the unit square the picture occupies exactly where
    // `images_on` says the picture is. This is the arithmetic that was wrong
    // once — a transform that looks plausible and puts every picture one
    // height off — so it is held to the page's own numbers here.
    let style = html
        .split("<img class=\"pic\"")
        .nth(1)
        .and_then(|tag| tag.split("style=\"").nth(1))
        .and_then(|style| style.split('"').next())
        .expect("the img's style");
    let (tx, ty) = style
        .split("translate(")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .and_then(|pair| pair.split_once(','))
        .expect("a translate");
    let matrix: Vec<f32> = style
        .split("matrix(")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .expect("a matrix")
        .split(',')
        .map(|value| value.trim().parse().expect("a number"))
        .collect();
    let (tx, ty): (f32, f32) = (
        tx.trim().trim_end_matches("px").parse().expect("x"),
        ty.trim().trim_end_matches("px").parse().expect("y"),
    );
    let (a, b, c, d) = (matrix[0], matrix[1], matrix[2], matrix[3]);
    // **Only the translation is in pixels; the matrix's linear part scales
    // the element's own points.** The image fills the unit square in PDF
    // space — its matrix is what scales that square to the page — so the
    // corners are that square through the matrix, in points, plus the
    // translation converted from CSS pixels.
    let to_points = 72.0 / 96.0;
    let corners: Vec<(f32, f32)> = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
        .map(|(u, v)| (tx * to_points + a * u + c * v, ty * to_points + b * u + d * v))
        .to_vec();
    let left = corners.iter().map(|p| p.0).fold(f32::MAX, f32::min);
    let right = corners.iter().map(|p| p.0).fold(f32::MIN, f32::max);
    let top = corners.iter().map(|p| p.1).fold(f32::MAX, f32::min);
    let bottom = corners.iter().map(|p| p.1).fold(f32::MIN, f32::max);
    let first = &pictures[0];
    let close = |one: f32, other: f32| (one - other).abs() < 0.5;
    assert!(
        close(left, first.rect.left)
            && close(top, first.rect.top)
            && close(right, first.rect.right)
            && close(bottom, first.rect.bottom),
        "the picture's transform puts it at ({left}, {top})..({right}, {bottom}) \
         where the page says {:?}",
        first.rect
    );
}

#[test]
fn an_embedded_picture_exports_as_a_png_of_its_own_pixels() {
    let session = Session::open(fixture("pictures.pdf")).expect("open");
    let images = session.images_on(0).expect("images");
    let first = images.first().expect("pictures.pdf carries pictures");

    let png = session.export_image_png(0, first.object).expect("png");

    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
    // PNG's IHDR: width and height are the first two 4-byte fields after the
    // 8-byte signature and the 8-byte chunk header.
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    assert_eq!(
        (width, height),
        (first.pixel_width, first.pixel_height),
        "the PNG's dimensions must be the image's own pixels, not its size on the page"
    );
}

#[test]
fn the_character_wrap_is_a_count_and_keeps_the_characters() {
    use pagify_shell::export::wrap_by_chars;

    let wrapped = wrap_by_chars("abcdefghij", 4);
    assert_eq!(wrapped, ["abcd", "efgh", "ij"]);
    assert_eq!(wrapped.concat(), "abcdefghij", "the wrap lost characters");

    // A line short of the width is one piece, not zero.
    assert_eq!(wrap_by_chars("abc", 4), ["abc"]);
}

#[test]
fn the_plain_formats_come_from_the_same_pages() {
    use pagify_shell::export::document_other;

    let session = Session::open(fixture("two-column.pdf")).expect("open");

    let txt = document_other(&session, "txt").expect("txt");
    assert!(!txt.contains("<"), "txt must be plain: {txt}");
    assert!(!txt.trim().is_empty());

    let rtf = document_other(&session, "rtf").expect("rtf");
    assert!(rtf.starts_with("{\\rtf1"), "{rtf}");
    assert!(rtf.trim_end().ends_with('}'), "the RTF must close: {rtf}");

    let xml = document_other(&session, "xml").expect("xml");
    assert!(xml.contains("<?xml version=\"1.0\""), "{xml}");
    assert!(xml.contains("<page n=\"1\">") && xml.contains("</document>"), "{xml}");

    match document_other(&session, "docx") {
        Err(why) => assert!(
            why.to_string().contains("txt, .rtf and .xml"),
            "unhelpful refusal: {why}"
        ),
        Ok(_) => panic!("docx cannot be written by this build"),
    }
}

/// A one-page PDF drawing `content`, with Helvetica as `/F1` — the same
/// minimal builder the snapshot tests use.
fn page_drawing(content: &str) -> Vec<u8> {
    let objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A file that removes itself, named per test so two running at once cannot
/// collide.
struct TempPdf(std::path::PathBuf);

impl TempPdf {
    fn new(name: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!("pagify-export-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).expect("write the scratch page");
        TempPdf(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempPdf {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// **The pieces of one line ride together** — the reported "sometimes the
/// texts overlap", built from its own shape: a line split into text objects
/// wherever kerning changed, each piece with its own pen position. Pinned one
/// by one they overlap as soon as the reader's font is not the file's; as one
/// line the reader lays them out from the first piece's pen.
#[test]
fn the_pieces_of_one_line_ride_together() {
    // The datasheet's own shape: one line, three objects.
    let page = page_drawing(
        "BT /F1 12 Tf 1 0 0 1 72 700 Tm (Th) Tj (e editors) Tj ( market) Tj ET\n\
         BT /F1 12 Tf 1 0 0 1 72 660 Tm (A line of its own) Tj ET",
    );
    let file = TempPdf::new("line-pieces.pdf", &page);
    let session = Session::open(file.path().to_string_lossy().into_owned()).expect("open");

    let html = export::document_html(&session, "pieces").expect("html");

    assert_eq!(
        html.matches("class=\"line\"").count(),
        1,
        "the three pieces of one line are one line: {html}"
    );
    assert!(
        html.contains(">Th</span><span class=\"run\""),
        "the pieces did not ride together: {html}"
    );
    // **The pieces keep their own spaces** — `"e editors "` with its trailing
    // space, `" market"` with its leading one: once the pieces flow as one
    // line, those spaces are the spaces between the words, and trimming them
    // is what glues the words together.
    assert!(
        html.contains(">e editors </span>") && html.contains("> market</span>"),
        "a piece or its own space is missing: {html}"
    );
    assert!(
        html.contains(">A line of its own</span>"),
        "the line of its own is missing: {html}"
    );
}

/// **The document's own font program is embedded**, once, with its weight and
/// angle — `@font-face` at the head and every run of that face naming the
/// family, which is what makes the reader's letters and widths the file's.
/// Driven through the app's own path: the typing fonts offered, a run retyped
/// with a new face, and the export read back — no save needed, the export
/// reads the session as it stands.
#[test]
fn the_html_embeds_the_document_s_own_font_program() {
    use pdf_core::document::TextStyle;

    let font_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../third_party/fonts/Montserrat-Regular.ttf");
    let Ok(program) = std::fs::read(&font_path) else {
        eprintln!("skipped: {} is not here", font_path.display());
        return;
    };
    let Some(face) = pdf_core::pdf::embed::face_name(&program) else {
        eprintln!("skipped: the shipped font names no face");
        return;
    };

    let session = Session::open(fixture("text-lines.pdf")).expect("open");
    session.set_typing_fonts(vec![program]).expect("typing fonts");
    let target = session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|run| run.text.trim().chars().count() > 4)
        .expect("a run with words");
    session
        .execute(pdf_core::command::Command::SetTextRun {
            page_index: 0,
            object: target.object,
            text: "Zwölf Ünique".into(),
            style: TextStyle { face: Some(face.clone()), ..Default::default() },
        })
        .expect("the retype");

    let html = export::document_html(&session, "fonts").expect("html");

    assert!(html.contains("@font-face"), "no face was embedded: {html}");
    assert!(
        html.contains("url(data:font/ttf;base64,"),
        "the program is not a data URI of a TrueType face: {html}"
    );
    // The declared family is the one that run asks for — not a bare name a
    // reader's machine might or might not have.
    let family = html
        .split("@font-face { font-family: '")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .expect("a declared family");
    assert!(family.starts_with("pdf-"), "the family is not a generated one: {family}");
    assert!(
        html.contains(&format!("font-family:'{family}',")),
        "no run names the embedded family {family:?}: {html}"
    );
    // And the run's own words came through, so the face was embedded for a
    // reason.
    assert!(html.contains("Zwölf Ünique"), "the retyped words are missing: {html}");
}

/// **A watched export that is stopped answers `None`** — half a file is not
/// an export — and one that is never stopped answers exactly what the plain
/// call does. This is the contract the window's progress dialog (and its
/// Cancel) is built on; see `pagify_shell::export::document_html_watched`.
#[test]
fn a_stopped_export_answers_none_and_an_unstopped_one_the_whole_file() {
    let session = Session::open(fixture("pages-ladder.pdf")).expect("open");
    let pages = session.page_count().expect("count");
    assert!(pages > 2, "the ladder has several pages");

    let whole = export::document_html(&session, "ladder").expect("html");
    let mut seen = Vec::new();
    let same = export::document_html_watched(&session, "ladder", &mut |page, total| {
        seen.push((page, total));
        true
    })
    .expect("html")
    .expect("a caller that never stops gets its document");
    assert_eq!(same, whole, "watching changed the document");
    assert_eq!(seen.last().copied(), Some((pages, pages)), "the watch saw every page");

    let mut walked = 0usize;
    let stopped = export::document_html_watched(&session, "ladder", &mut |page, _| {
        walked = page;
        page < 2
    })
    .expect("html");
    assert!(stopped.is_none(), "a stopped export answered a document");
    assert_eq!(walked, 2, "it stopped when told to");

    // The same contract for To Other's read of every page.
    let text = export::document_other(&session, "txt").expect("text");
    let watched = export::document_other_watched(&session, "txt", &mut |_, _| true)
        .expect("text")
        .expect("a caller that never stops gets its text");
    assert_eq!(watched, text, "watching changed the text");
    assert!(
        export::document_other_watched(&session, "txt", &mut |page, _| page < 1)
            .expect("text")
            .is_none(),
        "a stopped read answered a document"
    );
}
