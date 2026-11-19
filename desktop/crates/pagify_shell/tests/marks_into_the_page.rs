//! **Marks drawn into the page come back as marks, and are never drawn twice.**
//!
//! Save writes the markup layer's ink into the page under `/PagifyMarks` so every
//! viewer shows it; the geometry stays in the hidden block. This is the other
//! half: the drawing is taken back out when the page's marks are next committed,
//! the hidden block is replaced rather than added to, and a reopen with no edit
//! leaves both the drawing and the geometry alone.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release -p pagify_shell --test marks_into_the_page
//! ```

use cad_kernel::{Geom, PolyVertex, Polyline, Vec2};
use pagify_shell::commit;
use pagify_shell::markup::Layer;
use pagify_shell::Session;
use pdf_core::command::Command;
use pdf_core::document::{Annotation, Color, Point};
use pdf_core::pdf::{File, Object};

fn fixture(name: &str) -> String {
    format!("{}/../../../rust/pdf_core/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("pagify-marks-{name}-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// A rectangle at a chosen place, the way the markup tool draws it.
fn rect_at(page_height_pt: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> Layer {
    let mut layer = Layer::new(page_height_pt);
    layer.add(Geom::Polyline(Polyline {
        vertices: vec![
            PolyVertex { pos: Vec2::new(x0, y0), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(x1, y0), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(x1, y1), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(x0, y1), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(x0, y0), bulge: 0.0 },
        ],
        closed: true,
        widths: Vec::new(),
    }));
    layer
}

/// One closed rectangle, the way the markup tool draws it.
fn rectangle_layer(page_height_pt: f64) -> Layer {
    rect_at(page_height_pt, 100.0, 200.0, 300.0, 400.0)
}

fn pink() -> Color {
    Color { r: 255, g: 92, b: 138, a: 255 }
}

fn ink_annotations(session: &Session) -> usize {
    session
        .annotations(0)
        .expect("annotations")
        .iter()
        .filter(|a| matches!(a.annotation, Annotation::Ink { .. }))
        .count()
}

/// The x of the first rectangle a restored layer holds.
fn restored_x(stored: &commit::StoredLayer) -> f64 {
    let layer = commit::to_layer(stored);
    match &layer.objects().first().expect("a restored object").geom {
        Geom::Polyline(p) => p.vertices[0].pos.x as f64,
        other => panic!("the restored object is not a rectangle: {other:?}"),
    }
}

/// How many hidden blocks the file's page content holds.
///
/// `/PagifyText` is written once per carrier, inside its `BDC … EMC`; a file
/// saved after several commits has one for each, so this is what "one per page"
/// is measured by. Every stream is decoded, because PDFium compresses on save.
fn carrier_count(bytes: &[u8]) -> usize {
    const MARK: &[u8] = b"/PagifyText";
    let file = File::parse(bytes).expect("a readable file");
    let mut count = 0;
    for number in 1..40_000u32 {
        if let Ok(Object::Stream(dict, range)) = file.object(number) {
            if let Some(drawing) = pdf_core::pdf::content::decode(&dict, &file.bytes()[range]) {
                count += drawing.windows(MARK.len()).filter(|window| *window == MARK).count();
            }
        }
    }
    count
}

/// Whether the file still stores ArialMT's outlines.
fn arial_program_present(bytes: &[u8]) -> bool {
    let file = File::parse(bytes).expect("a readable file");
    for number in 1..40_000u32 {
        let Ok(Object::Dict(font)) = file.object(number) else { continue };
        let Some(base) = font.get(b"BaseFont").and_then(Object::as_name) else { continue };
        if !String::from_utf8_lossy(base).contains("ArialMT") {
            continue;
        }
        let Some(descriptor) = font.get(b"FontDescriptor") else { continue };
        let Ok(Object::Dict(descriptor)) = file.resolve(descriptor) else { continue };
        if descriptor.get(b"FontFile2").is_some() || descriptor.get(b"FontFile3").is_some() {
            return true;
        }
    }
    false
}

/// Whether the file draws anything under the tag — decoded, because PDFium
/// compresses the page's content when it saves.
fn holds_the_tag(bytes: &[u8]) -> bool {
    pdf_core::pdf::content::may_name(bytes, b"/PagifyMarks")
}

/// The first page's drawing, every content stream decoded, so a tag can be
/// counted rather than merely found.
fn first_page_drawing(bytes: &[u8]) -> String {
    use pdf_core::pdf::{content, Dict};

    fn walk(file: &File<'_>, node: &Object, out: &mut Vec<Dict>) {
        let Some(dict) = node.as_dict() else { return };
        if dict.get(b"Type").and_then(Object::as_name) == Some(b"Page") {
            out.push(dict.clone());
            return;
        }
        if let Some(Ok(Object::Array(kids))) = dict.get(b"Kids").map(|k| file.resolve(k)) {
            for kid in kids {
                if let Ok(kid) = file.resolve(&kid) {
                    walk(file, &kid, out);
                }
            }
        }
    }

    let file = File::parse(bytes).expect("a readable file");
    let root = file.resolve(file.trailer().get(b"Root").expect("root")).expect("catalogue");
    let pages = file.resolve(root.as_dict().and_then(|d| d.get(b"Pages")).expect("pages")).expect("tree");
    let mut all = Vec::new();
    walk(&file, &pages, &mut all);
    let page = all.first().expect("a page");

    let mut out = String::new();
    if let Some(contents) = page.get(b"Contents") {
        let items = match file.resolve(contents).expect("contents") {
            Object::Array(items) => items,
            Object::Stream(..) => vec![contents.clone()],
            _ => Vec::new(),
        };
        for item in items {
            if let Object::Stream(dict, range) = file.resolve(&item).expect("a content stream") {
                out.push_str(&String::from_utf8_lossy(
                    &content::decode(&dict, &file.bytes()[range]).expect("decodes"),
                ));
                out.push('\n');
            }
        }
    }
    out
}

fn pixel_count(raster: &pagify_shell::session::PageRaster, wanted: fn(&[u8]) -> bool) -> usize {
    raster.pixels.chunks_exact(4).filter(|p| wanted(p)).count()
}

/// Pink pixels inside one box of a scale-1.0 render.
fn pink_in(raster: &pagify_shell::session::PageRaster, x0: usize, y0: usize, x1: usize, y1: usize) -> usize {
    let w = raster.width as usize;
    let mut count = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = &raster.pixels[(y * w + x) * 4..(y * w + x) * 4 + 4];
            if p[0] > 200 && (60..140).contains(&p[1]) && (100..180).contains(&p[2]) {
                count += 1;
            }
        }
    }
    count
}

/// A page whose own content carries two hidden blocks, with different layers.
///
/// Written by hand because the engine no longer produces one: each commit must
/// leave exactly one, and files saved before 9c are the ones with several.
fn two_carrier_page(first: &str, second: &str) -> Vec<u8> {
    let content = format!(
        "/PagifyText <</id 1885431553 /restore ({first})>> BDC\n\
         BT /F1 1 Tf 12 0 0 12 40 60 Tm ( ) Tj ET\n\
         EMC\n\
         /PagifyText <</id 1885431553 /restore ({second})>> BDC\n\
         BT /F1 1 Tf 12 0 0 12 40 40 Tm ( ) Tj ET\n\
         EMC\n"
    );
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
            stream.extend_from_slice(content.as_bytes());
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
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

fn blob(x0: f64, y0: f64, x1: f64, y1: f64) -> String {
    format!(
        "{{\"magic\":\"pagify.markup\",\"version\":2,\"page_height_pt\":792.0,\"objects\":[{{\"k\":\"polyline\",\"v\":[[{x0},{y0},0.0],[{x1},{y0},0.0],[{x1},{y1},0.0],[{x0},{y1},0.0],[{x0},{y0},0.0]],\"closed\":true,\"filled\":false,\"lineweight_mm\":null}}]}}"
    )
}

/// **The whole round trip.** A rectangle is drawn, saved, reopened, edited and
/// written again: one Ink annotation, no tagged drawing left, and a page that
/// looks exactly as it did before the save.
#[test]
fn a_rectangle_saved_reopened_and_edited_is_drawn_once() {
    let path = scratch("once");
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let height = session.page_size(0).expect("size").height_pt as f64;
    let layer = rectangle_layer(height);

    session.commit_markup(0, &layer, pink(), 1.0).expect("commit");
    assert_eq!(ink_annotations(&session), 1, "setup: the rectangle was not written as ink");
    let before = session.render_page(0, 1.0).expect("render before");

    session.save_to_with_fixups(&path, false, &|_: &str| None).expect("save with fix-ups");
    drop(session);
    let reopened = Session::open(&path).expect("reopen the saved bytes");

    let stored = reopened
        .with_engine(|s| commit::restore_page(&*s.document, 0).map_err(pdf_core::PdfError::InvalidArgument))
        .expect("read the hidden block")
        .expect("the layer came back");
    assert_eq!(stored.version, 2, "the marks were written in a version that does not say the drawing may be in the page");
    assert_eq!(stored.objects.len(), 1, "the rectangle did not come back");
    assert_eq!(ink_annotations(&reopened), 0, "the reopened page still carries the rectangle as a mark");
    assert!(
        holds_the_tag(&reopened.current_bytes().expect("bytes")),
        "the saved page does not draw the rectangle as tagged page content"
    );

    let again = commit::to_layer(&stored);
    reopened.commit_markup(0, &again, pink(), 1.0).expect("commit the restored layer");
    assert_eq!(ink_annotations(&reopened), 1, "the rectangle was not written back as exactly one mark");
    assert!(
        !holds_the_tag(&reopened.current_bytes().expect("bytes")),
        "the drawing is still in the page, so the next save would draw it twice"
    );

    let after = reopened.render_page(0, 1.0).expect("render after");
    assert_eq!(before.pixels, after.pixels, "the page looks different after the edit");
    let _ = std::fs::remove_file(&path);
}

/// **Opening and closing changes nothing.** The drawing is still in the page,
/// the marks are still only in the hidden block, and they still come back.
#[test]
fn reopening_without_editing_leaves_the_drawing_and_restores_the_marks() {
    let path = scratch("reopen");
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let height = session.page_size(0).expect("size").height_pt as f64;
    session.commit_markup(0, &rectangle_layer(height), pink(), 1.0).expect("commit");
    session.save_to_with_fixups(&path, false, &|_: &str| None).expect("save with fix-ups");
    drop(session);

    let reopened = Session::open(&path).expect("reopen the saved bytes");
    assert_eq!(ink_annotations(&reopened), 0, "the rectangle came back as an annotation");
    assert!(holds_the_tag(&reopened.current_bytes().expect("bytes")), "the drawing is not in the page");

    let stored = reopened
        .with_engine(|s| commit::restore_page(&*s.document, 0).map_err(pdf_core::PdfError::InvalidArgument))
        .expect("read the hidden block")
        .expect("the layer came back");
    assert_eq!(stored.version, 2);
    assert_eq!(stored.objects.len(), 1, "the rectangle did not come back");

    // The drawing is still what the page shows.
    let raster = reopened.render_page(0, 1.0).expect("render");
    let pink = pixel_count(&raster, |p| p[0] > 200 && (60..140).contains(&p[1]) && (100..180).contains(&p[2]));
    assert!(pink > 0, "the page does not draw the rectangle at all");
    let _ = std::fs::remove_file(&path);
}

/// **What the markup layer did not write is not ours to take out.** A drawn
/// signature, an article box and ink from Pagify Android are all `/PagifyColor`
/// ink, but none is in the hidden block — so a commit cannot put them back. Save
/// must leave them annotations; only the rectangle, which the blob describes,
/// goes into the page.
#[test]
fn a_drawn_signature_an_article_box_and_android_ink_survive_every_save() {
    let path = scratch("signature");
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let height = session.page_size(0).expect("size").height_pt as f64;

    // The markup layer's rectangle.
    session.commit_markup(0, &rectangle_layer(height), pink(), 1.0).expect("commit");

    // A drawn signature: `place_signature`, as the Sign tool writes it.
    let blue = Color { r: 0, g: 0, b: 255, a: 255 };
    session
        .place_signature(
            0,
            vec![vec![
                Point { x: 350.0, y: 500.0 },
                Point { x: 380.0, y: 560.0 },
                Point { x: 410.0, y: 500.0 },
                Point { x: 440.0, y: 560.0 },
            ]],
            blue,
            3.0,
            "Farzad",
        )
        .expect("the signature");

    // An article box: an Ink added as a plain annotation, as the box and
    // Android's engine write it — no `/PagifyMarkup`.
    let green = Color { r: 0, g: 160, b: 0, a: 255 };
    session
        .execute(Command::AddAnnotation {
            page_index: 0,
            annotation: Annotation::Ink {
                strokes: vec![vec![
                    Point { x: 100.0, y: 620.0 },
                    Point { x: 300.0, y: 620.0 },
                    Point { x: 300.0, y: 680.0 },
                    Point { x: 100.0, y: 680.0 },
                    Point { x: 100.0, y: 620.0 },
                ]],
                color: green,
                width: 1.5,
            },
        })
        .expect("the article box");
    assert_eq!(ink_annotations(&session), 3, "setup: the rectangle, the signature and the box");

    session.save_to_with_fixups(&path, false, &|_: &str| None).expect("save with fix-ups");
    drop(session);

    // The app's Save on the reopened file: restore each marked page and commit it.
    let reopened = Session::open(&path).expect("reopen the saved bytes");
    let layer = reopened.restore_markup(0).expect("read the hidden block").expect("the layer came back");
    reopened.commit_markup(0, &layer, pink(), 1.0).expect("commit as Save does");
    let second = scratch("signature-again");
    reopened.save_to_with_fixups(&second, false, &|_: &str| None).expect("the second save");
    drop(reopened);

    let after = Session::open(&second).expect("reopen the second save");
    let marks = after.annotations(0).expect("annotations");
    assert_eq!(ink_annotations(&after), 2, "the signature or the article box is gone: {marks:?}");
    let signatures = after.signature_marks(0).expect("signatures");
    assert_eq!(signatures.len(), 1, "the drawn signature is no longer marked as one: {signatures:?}");
    assert_eq!(signatures[0].name, "Farzad");

    // The rectangle is drawn exactly once, by the page, and is no Ink.
    let drawing = first_page_drawing(&after.current_bytes().expect("bytes"));
    assert_eq!(drawing.matches("/PagifyMarks").count(), 1, "the rectangle is not drawn exactly once:\n{drawing}");
    assert!(
        !marks.iter().any(|m| matches!(&m.annotation, Annotation::Ink { color, .. } if color.r == 255 && color.g == 92)),
        "the rectangle is still an annotation as well as drawn: {marks:?}"
    );

    // And all three still show.
    let raster = after.render_page(0, 1.0).expect("render");
    let pink_pixels = pixel_count(&raster, |p| p[0] > 200 && (60..140).contains(&p[1]) && (100..180).contains(&p[2]));
    let blue_pixels = pixel_count(&raster, |p| p[0] < 80 && p[1] < 80 && p[2] > 180);
    let green_pixels = pixel_count(&raster, |p| p[0] < 90 && p[1] > 120 && p[2] < 90);
    assert!(pink_pixels > 0, "the rectangle is not drawn");
    assert!(blue_pixels > 0, "the signature does not draw its pixels");
    assert!(green_pixels > 0, "the article box does not draw its pixels");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&second);
}

/// **Files marked before the key existed still bake.** 0.1.125 wrote the
/// rectangles with no `/PagifyMarkup`, and a file carries no tag of its own. The
/// app recommits every marked page at Save, which writes the key; the same Save
/// then bakes them, and the page must look exactly as it did.
#[test]
fn a_file_marked_by_0_1_125_bakes_on_its_first_save() {
    let path = r"C:\Users\hsili\Desktop\Pagify_EditText_Evidence\PRISMA-3-rectangles.pdf";
    if !std::path::Path::new(path).is_file() {
        eprintln!("skipping: {path} is not on this machine");
        return;
    }
    let out = scratch("prisma-baked");

    let session = Session::open(path).expect("open the owner's file");
    let layer = session.restore_markup(0).expect("read the hidden block").expect("the layer came back");
    assert_eq!(layer.len(), 3, "the owner's file does not carry the three rectangles");
    session.commit_markup(0, &layer, pink(), 1.5).expect("commit as Save does");
    session.save_to_with_fixups(&out, false, &|_: &str| None).expect("save with fix-ups");
    drop(session);

    let after = Session::open(&out).expect("reopen the baked file");
    assert_eq!(ink_annotations(&after), 0, "an Ink annotation of the three rectangles is left");
    let drawing = first_page_drawing(&after.current_bytes().expect("bytes"));
    assert_eq!(drawing.matches("/PagifyMarks").count(), 3, "the three rectangles were not all baked:\n{drawing}");

    let before = Session::open(path).expect("open the input again");
    let drawn_with_marks = before.render_page(0, 0.3).expect("render the input");
    let drawn_as_page = after.render_page(0, 0.3).expect("render the baked file");
    assert_eq!(
        drawn_with_marks.pixels, drawn_as_page.pixels,
        "the page draws differently after baking the rectangles"
    );
    let _ = std::fs::remove_file(&out);
}

/// **One commit leaves one hidden block.** Before this, every commit added one:
/// the carrier is a space PDFium makes no run of, so `remove_text` never found
/// it, and each Save commits every marked page.
#[test]
fn committing_twice_leaves_one_hidden_block() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let height = session.page_size(0).expect("size").height_pt as f64;
    session.commit_markup(0, &rectangle_layer(height), pink(), 1.0).expect("commit");
    assert_eq!(carrier_count(&session.current_bytes().expect("bytes")), 1, "setup: one block after one commit");
    session.commit_markup(0, &rectangle_layer(height), pink(), 1.0).expect("commit again");
    assert_eq!(
        carrier_count(&session.current_bytes().expect("bytes")),
        1,
        "the second commit added a hidden block instead of replacing the first"
    );
}

/// **Every Save commits, so every Save must replace.** Saved, reopened and
/// recommitted three times — with Arial on offer as the app's finder supplies it
/// — the page still carries one hidden block.
#[test]
fn a_page_saved_and_reopened_three_times_keeps_one_hidden_block() {
    let arial_path = r"C:\Windows\Fonts\arial.ttf";
    if !std::path::Path::new(arial_path).is_file() {
        eprintln!("skipping: {arial_path} is not on this machine");
        return;
    }
    let arial = std::fs::read(arial_path).expect("read Arial");
    let finder = |_: &str| Some(arial.clone());
    let path = scratch("three");
    let pink = pink();

    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let height = session.page_size(0).expect("size").height_pt as f64;
    session.commit_markup(0, &rectangle_layer(height), pink, 1.0).expect("commit");
    session.save_to_with_fixups(&path, false, &finder).expect("save");
    drop(session);

    for round in 1..=3 {
        let session = Session::open(&path).expect("reopen");
        let layer = session.restore_markup(0).expect("read the hidden block").expect("the layer came back");
        session.commit_markup(0, &layer, pink, 1.0).expect("commit as Save does");
        assert_eq!(
            carrier_count(&session.current_bytes().expect("bytes")),
            1,
            "round {round}: the page carries more than one hidden block"
        );
        session.save_to_with_fixups(&path, false, &finder).expect("save again");
    }
    assert_eq!(
        carrier_count(&std::fs::read(&path).expect("read the saved file")),
        1,
        "the saved file carries more than one hidden block"
    );
    let _ = std::fs::remove_file(&path);
}

/// **An edit made after reopening is what the next reopen restores.** Draw A,
/// save; reopen, replace A by B, save; reopen and Save as the app does. Only B
/// may be drawn, and B is what the hidden block holds — before this, A came
/// back, the next Save wrote A and removed B's drawing.
#[test]
fn an_edit_made_after_reopening_is_what_the_next_reopen_restores() {
    let path = scratch("edit");
    let pink = pink();

    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let height = session.page_size(0).expect("size").height_pt as f64;
    session.commit_markup(0, &rect_at(height, 100.0, 200.0, 300.0, 400.0), pink, 1.0).expect("A");
    session.save_to_with_fixups(&path, false, &|_: &str| None).expect("save 1");
    drop(session);

    let session = Session::open(&path).expect("reopen 1");
    session.commit_markup(0, &rect_at(height, 350.0, 450.0, 550.0, 650.0), pink, 1.0).expect("B replaces A");
    session.save_to_with_fixups(&path, false, &|_: &str| None).expect("save 2");
    drop(session);

    let session = Session::open(&path).expect("reopen 2");
    let stored = session
        .with_engine(|s| commit::restore_page(&*s.document, 0).map_err(pdf_core::PdfError::InvalidArgument))
        .expect("read the hidden block")
        .expect("the layer came back");
    assert_eq!(restored_x(&stored), 350.0, "the edit made after reopening did not replace the old marks");
    let layer = commit::to_layer(&stored);
    session.commit_markup(0, &layer, pink, 1.0).expect("Save's commit");
    session.save_to_with_fixups(&path, false, &|_: &str| None).expect("save 3");
    drop(session);

    let session = Session::open(&path).expect("reopen 3");
    let raster = session.render_page(0, 1.0).expect("render");
    // By its own x band: A and B do not overlap across the page, and the rows a
    // raster reports are not the app's own y, so the columns are what to read.
    let height = raster.height as usize;
    assert!(pink_in(&raster, 340, 0, 560, height) > 0, "B is not drawn");
    assert_eq!(pink_in(&raster, 90, 0, 310, height), 0, "A is drawn as well as B");
    assert_eq!(
        carrier_count(&std::fs::read(&path).expect("read the saved file")),
        1,
        "the page carries more than one hidden block"
    );
    let _ = std::fs::remove_file(&path);
}

/// **A page with old hidden blocks restores the newest, and one commit leaves
/// one.** Files saved since 0.1.91 can carry several; each commit appended. The
/// last is the newest, because that is the one the last commit wrote.
#[test]
fn a_page_with_old_hidden_blocks_restores_the_newest_and_keeps_one() {
    let path = scratch("two-blocks");
    std::fs::write(&path, two_carrier_page(&blob(100.0, 200.0, 300.0, 400.0), &blob(350.0, 450.0, 550.0, 650.0)))
        .expect("write the two-block page");
    assert_eq!(carrier_count(&std::fs::read(&path).expect("read")), 2, "setup: two hidden blocks");

    let session = Session::open(&path).expect("open the two-block page");
    let stored = session
        .with_engine(|s| commit::restore_page(&*s.document, 0).map_err(pdf_core::PdfError::InvalidArgument))
        .expect("read the hidden block")
        .expect("a layer came back");
    assert_eq!(restored_x(&stored), 350.0, "the older hidden block came back instead of the newest");

    let layer = commit::to_layer(&stored);
    session.commit_markup(0, &layer, pink(), 1.0).expect("commit");
    assert_eq!(
        carrier_count(&session.current_bytes().expect("bytes")),
        1,
        "the old hidden blocks were not all replaced"
    );
    let _ = std::fs::remove_file(&path);
}

/// **The owner's PRISMA, through the app's Save.** Its 0.1.125 block holds an
/// ArialMT program and draws the rectangles; the new block supersedes it. One
/// save later the page carries one block, Arial's program is gone, and the file
/// is under 650,000 bytes.
#[test]
fn the_owners_prisma_drops_the_old_arial_on_its_first_save() {
    let path = r"C:\Users\hsili\Desktop\Pagify_EditText_Evidence\PRISMA-3-rectangles.pdf";
    if !std::path::Path::new(path).is_file() {
        eprintln!("skipping: {path} is not on this machine");
        return;
    }
    let arial_path = r"C:\Windows\Fonts\arial.ttf";
    if !std::path::Path::new(arial_path).is_file() {
        eprintln!("skipping: {arial_path} is not on this machine");
        return;
    }
    let arial = std::fs::read(arial_path).expect("read Arial");
    let finder = |_: &str| Some(arial.clone());
    let out = scratch("prisma-one-block");

    let input = std::fs::read(path).expect("read the owner's file");
    assert_eq!(carrier_count(&input), 1, "setup: the owner's file carries one hidden block");
    assert!(arial_program_present(&input), "setup: the owner's file stores ArialMT's outlines");

    let session = Session::open(path).expect("open the owner's file");
    let layer = session.restore_markup(0).expect("read the hidden block").expect("the layer came back");
    session.commit_markup(0, &layer, pink(), 1.5).expect("commit as Save does");
    session.save_to_with_fixups(&out, false, &finder).expect("save with the app's finder");
    drop(session);

    let saved = std::fs::read(&out).expect("read the saved file");
    eprintln!("PRISMA after its first save: {} bytes, {} hidden block(s)", saved.len(), carrier_count(&saved));
    assert_eq!(carrier_count(&saved), 1, "the old block was kept beside the new one");
    assert!(!arial_program_present(&saved), "ArialMT's outlines are still in the file");
    assert!(saved.len() < 650_000, "the file is {} bytes; the old block's font was not dropped", saved.len());
    let _ = std::fs::remove_file(&out);
}
