//! Verifies the coordinate space `PdfPathSegment::point()` reports, against a
//! real fixture, before anything is built on an assumption about it.
use pdfium_render::prelude::*;

fn main() {
    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(lib).expect("bind"));
    let doc = pdfium.load_pdf_from_file("fixtures/outlined.pdf", None).expect("open");
    let page = doc.pages().get(0).expect("page 0");

    for (i, object) in page.objects().iter().enumerate().take(3) {
        let Some(path) = object.as_path_object() else { continue };
        let bounds = object.bounds().expect("bounds");
        println!(
            "object {i}: FPDFPageObj_GetBounds  L{:.2} T{:.2} R{:.2} B{:.2}",
            bounds.left().value, bounds.top().value, bounds.right().value, bounds.bottom().value
        );

        // Raw, no .transform() — is this already in page space, or local space?
        let mut raw_min = (f32::MAX, f32::MAX);
        let mut raw_max = (f32::MIN, f32::MIN);
        for seg in path.segments().iter() {
            let (x, y) = (seg.x().value, seg.y().value);
            raw_min = (raw_min.0.min(x), raw_min.1.min(y));
            raw_max = (raw_max.0.max(x), raw_max.1.max(y));
        }
        println!(
            "           raw segment bbox (no transform)  L{:.2} B{:.2} R{:.2} T{:.2}",
            raw_min.0, raw_min.1, raw_max.0, raw_max.1
        );

        // How are curves represented? One BezierTo call with 3 points, or 3
        // consecutive BezierTo segments each giving one point?
        println!("           segment sequence:");
        for seg in path.segments().iter().take(12) {
            println!(
                "             {:?}  ({:.2}, {:.2})  close={}",
                seg.segment_type(), seg.x().value, seg.y().value, seg.is_close()
            );
        }

        // With the object's own matrix applied.
        let matrix = object.matrix().expect("matrix");
        let mut tf_min = (f32::MAX, f32::MAX);
        let mut tf_max = (f32::MIN, f32::MIN);
        for seg in path.segments().transform(matrix).iter() {
            let (x, y) = (seg.x().value, seg.y().value);
            tf_min = (tf_min.0.min(x), tf_min.1.min(y));
            tf_max = (tf_max.0.max(x), tf_max.1.max(y));
        }
        println!(
            "           transformed segment bbox         L{:.2} B{:.2} R{:.2} T{:.2}",
            tf_min.0, tf_min.1, tf_max.0, tf_max.1
        );
        println!(
            "           matrix a={:.3} b={:.3} c={:.3} d={:.3} e={:.3} f={:.3}",
            matrix.a(), matrix.b(), matrix.c(), matrix.d(), matrix.e(), matrix.f()
        );
        println!();
    }
}
