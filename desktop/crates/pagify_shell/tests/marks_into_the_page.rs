//! **Marks drawn into the page come back as marks, and are never drawn twice.**
//!
//! Save writes Pagify's own ink into the page under `/PagifyMarks` so every
//! viewer shows it; the geometry stays in the hidden block. This is the other
//! half: the drawing is taken back out when the page's marks are next committed,
//! and a reopen with no edit leaves both the drawing and the geometry alone.
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

fn fixture(name: &str) -> String {
    format!("{}/../../../rust/pdf_core/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("pagify-marks-{name}-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// One closed rectangle, the way the markup tool draws it.
fn rectangle_layer(page_height_pt: f64) -> Layer {
    let mut layer = Layer::new(page_height_pt);
    layer.add(Geom::Polyline(Polyline {
        vertices: vec![
            PolyVertex { pos: Vec2::new(100.0, 200.0), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(300.0, 200.0), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(300.0, 400.0), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(100.0, 400.0), bulge: 0.0 },
            PolyVertex { pos: Vec2::new(100.0, 200.0), bulge: 0.0 },
        ],
        closed: true,
        widths: Vec::new(),
    }));
    layer
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

/// Whether the file draws anything under the tag — decoded, because PDFium
/// compresses the page's content when it saves.
fn holds_the_tag(bytes: &[u8]) -> bool {
    pdf_core::pdf::content::may_name(bytes, b"/PagifyMarks")
}

/// The first page's drawing, every content stream decoded, so a tag can be
/// counted rather than merely found.
fn first_page_drawing(bytes: &[u8]) -> String {
    use pdf_core::pdf::{content, Dict, File, Object};

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
    let pink = raster
        .pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 200 && (60..140).contains(&p[1]) && (100..180).contains(&p[2]))
        .count();
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
