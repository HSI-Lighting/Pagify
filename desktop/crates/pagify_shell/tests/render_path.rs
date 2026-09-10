//! Phase 0's acceptance: the render path, end to end, with no window.
//!
//! The build plan asks to "verify the render path end to end before building
//! any UI on top of it", because that one step proves PDFium loading, the
//! buffer contract and the coordinate convention together. These are that
//! check, kept as tests so it stays proved.
//!
//! Two of the failures being guarded against are *silent*. A page that renders
//! blank looks exactly like a blank page in the document. A page rendered
//! upside-down or with red and blue transposed looks, at a glance, like a page.
//! Neither throws. `quadrants.pdf` exists to make both readable off single
//! pixels — see the fixtures README.

use pagify_shell::Session;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../workspace/Pagify/rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// RGBA at a pixel, from a tightly packed top-left-origin buffer.
fn pixel(raster: &pagify_shell::PageRaster, x: u32, y: u32) -> (u8, u8, u8, u8) {
    let i = (y as usize * raster.width as usize + x as usize) * 4;
    (
        raster.pixels[i],
        raster.pixels[i + 1],
        raster.pixels[i + 2],
        raster.pixels[i + 3],
    )
}

#[test]
fn pdfium_loads_and_a_document_opens() {
    let session = Session::open(fixture("pages-ladder.pdf")).expect(
        "could not open a fixture. If this is a PDFium loading fault rather than a \
         missing file, check pagify_shell::pdfium::describe() — on an unfetched \
         platform slice this is the first test that will say so.",
    );

    assert_eq!(session.page_count().expect("page count"), 5);
}

#[test]
fn page_geometry_survives_the_engine() {
    // Every page in this fixture has a distinct width, on purpose, so the page
    // tree is verifiable without extracting text or comparing pixels.
    let session = Session::open(fixture("pages-ladder.pdf")).expect("open");

    let widths: Vec<u32> = (0..session.page_count().expect("page count"))
        .map(|i| session.page_size(i).expect("page size").width_pt.round() as u32)
        .collect();

    assert_eq!(widths, vec![200, 250, 300, 350, 400]);
}

#[test]
fn a_rendered_page_is_not_blank() {
    // The guard that makes a blank render fail CI rather than look like a blank
    // page in the document.
    let session = Session::open(fixture("text-lines.pdf")).expect("open");
    let raster = session.render_page(0, 2.0).expect("render");

    assert!(
        raster.ink(200) > 0,
        "page rendered {}x{} but every pixel is white — PDFium bound, the buffer \
         contract held, and nothing was drawn",
        raster.width,
        raster.height,
    );
}

#[test]
fn the_render_is_the_size_it_promised() {
    // The app sizes its texture from `page_pixel_size` before asking for the
    // pixels. If the two ever disagree the cache silently treats every entry as
    // a miss, and the only symptom is that scrolling got slow.
    let session = Session::open(fixture("mixed-sizes.pdf")).expect("open");

    for index in 0..session.page_count().expect("page count") {
        for scale in [1.0_f32, 1.5, 2.0] {
            let promised = session.page_pixel_size(index, scale).expect("size");
            let raster = session.render_page(index, scale).expect("render");
            assert_eq!(
                promised,
                (raster.width, raster.height),
                "page {index} at scale {scale}"
            );
        }
    }
}

#[test]
fn the_page_is_the_right_way_up_and_the_right_way_round() {
    // `quadrants.pdf` is one 400pt page in four flat colours: red top-left,
    // green top-right, blue bottom-left, yellow bottom-right.
    //
    // This is the test that catches the two silent faults. A vertical flip
    // swaps red for blue — and a flip is the specific hazard of adding a second
    // coordinate convention to a codebase that already flips once at the PDFium
    // boundary. A channel-order fault swaps red for blue as well, but leaves
    // green where it was, so the two are distinguishable rather than merely
    // both wrong.
    let session = Session::open(fixture("quadrants.pdf")).expect("open");
    let raster = session.render_page(0, 1.0).expect("render");

    assert_eq!((raster.width, raster.height), (400, 400));

    let quarter = raster.width / 4;
    let (three_quarters, low, high) = (quarter * 3, quarter, quarter * 3);

    let top_left = pixel(&raster, quarter, low);
    let top_right = pixel(&raster, three_quarters, low);
    let bottom_left = pixel(&raster, quarter, high);
    let bottom_right = pixel(&raster, three_quarters, high);

    let (r, g, b, a) = top_left;
    assert!(r > g && r > b, "top-left should be red, got {top_left:?}");
    assert_eq!(a, 255, "the page should be opaque");

    let (r, g, b, _) = top_right;
    assert!(g > r && g > b, "top-right should be green, got {top_right:?}");

    let (r, g, b, _) = bottom_left;
    assert!(b > r && b > g, "bottom-left should be blue, got {bottom_left:?}");

    let (r, g, b, _) = bottom_right;
    assert!(
        r > b && g > b,
        "bottom-right should be yellow, got {bottom_right:?}"
    );
}

#[test]
fn the_cache_answers_the_second_render() {
    // The page cache is what makes a swipe resolve to a memcpy. It is wired
    // through `DocumentSession`, so this also proves the session owns one at
    // all — a per-document cache rather than a global.
    let session = Session::open(fixture("text-lines.pdf")).expect("open");

    let first = session.render_page(0, 1.0).expect("first render");
    assert!(!first.from_cache, "the first render cannot be a cache hit");

    session.prefetch_page(0, 1.0).expect("prefetch");
    let second = session.render_page(0, 1.0).expect("second render");
    assert!(second.from_cache, "a prefetched page should be served from cache");

    assert_eq!(first.pixels, second.pixels, "the cached raster differs from the fresh one");
}

#[test]
fn documents_open_and_render_concurrently_without_crossing() {
    // The guard for the fault that shaped `Session`: PDFium recycles document
    // and page addresses, and `pdfium-render` keys a process-global page-index
    // cache on those raw addresses. Open one document while another is being
    // read and a page lookup can be answered from the wrong file — measured in
    // pdf_core's registry.rs at 771 of 800 opens failing, and among the reads
    // that got through, one that returned [200, 612, 612, 612]: one correct
    // page followed by three reporting US Letter, PDFium's fallback when it
    // cannot find a page's geometry.
    //
    // Constructing a PdfiumDocument directly and holding it outside the
    // registry lock reproduces this as a SIGABRT under the parallel test
    // runner. `Session` routes through `registry::insert_with`, which holds the
    // lock across the open itself. This test is what stops that being
    // "simplified" away later: desktop is explicitly multi-document, so the
    // two-thread case is the normal case, not an edge one.
    //
    // Every page in `pages-ladder.pdf` has a distinct width, so a page served
    // from the wrong document is visible as a wrong number rather than needing
    // pixels compared.
    const EXPECTED: [u32; 5] = [200, 250, 300, 350, 400];

    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|worker| {
                scope.spawn(move || {
                    for round in 0..4 {
                        let session = Session::open(fixture("pages-ladder.pdf"))
                            .expect("open under contention");

                        let widths: Vec<u32> = (0..session.page_count().expect("count"))
                            .map(|i| {
                                session.page_size(i).expect("size").width_pt.round() as u32
                            })
                            .collect();

                        assert_eq!(
                            widths, EXPECTED,
                            "worker {worker} round {round} read another document's geometry"
                        );

                        // Render too: the hazard is not open-vs-open alone, it
                        // is an open racing a *read*.
                        let raster = session.render_page(0, 1.0).expect("render");
                        assert_eq!(raster.width, 200, "page 0 of the ladder is 200pt wide");

                        // `session` drops here, deregistering — a close racing
                        // the next open is the other half of the same hazard.
                    }
                })
            })
            .collect();

        for worker in workers {
            worker.join().expect("a worker panicked");
        }
    });
}
