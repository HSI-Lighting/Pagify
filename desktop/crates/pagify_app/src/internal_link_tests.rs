use super::ui_tests::{click, harness_from};
use super::*;

/// Three pages, 200 x 300; page 1 has a link at the top left (PDF rect
/// `[10 250 60 290]`, so 10..60 across and 10..50 down in the page's own
/// top-left space) going to page 3, and a second, inside it, going to page 2.
fn linked_pdf() -> std::path::PathBuf {
    let page = |annots: &str| {
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 6 0 R {annots} >>")
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_string(),
        page("/Annots [7 0 R 8 0 R]"),
        page(""),
        page(""),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 250 60 290] /Border [0 0 0] /Dest [5 0 R /XYZ 0 300 0] >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [20 260 30 270] /Border [0 0 0] /A << /S /GoTo /D [4 0 R /Fit] >> >>".to_string(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
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
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n", objects.len() + 1).as_bytes(),
    );
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("pagify-internal-link-{}-{n}.pdf", std::process::id()));
    std::fs::write(&path, out).expect("the fixture could not be written");
    path
}

fn app() -> PagifyApp {
    let path = linked_pdf();
    let app = PagifyApp::new(Some(path.to_str().unwrap()));
    assert!(app.tab().doc.is_some(), "the fixture did not open");
    app
}

#[test]
fn a_point_on_a_link_names_the_page_it_goes_to_and_the_smallest_link_wins() {
    let mut app = app();
    assert_eq!(app.internal_link_at(0, AppPoint { x: 15.0, y: 40.0 }), Some(2), "on the big link");
    assert_eq!(
        app.internal_link_at(0, AppPoint { x: 25.0, y: 35.0 }),
        Some(1),
        "on the small link inside it"
    );
    assert_eq!(app.internal_link_at(0, AppPoint { x: 150.0, y: 200.0 }), None, "bare paper");
    assert_eq!(app.internal_link_at(1, AppPoint { x: 15.0, y: 40.0 }), None, "another page has none");
}

#[test]
fn clicking_a_link_goes_to_its_page() {
    let mut h = harness_from(app());
    h.run_steps(3);
    assert_eq!(h.state().tab().page, 0);
    let at = h
        .state()
        .tab()
        .last_view
        .expect("the page was never drawn")
        .to_screen(AppPoint { x: 15.0, y: 40.0 });
    click(&mut h, at);
    assert_eq!(h.state().tab().page, 2, "the click did not go to the linked page");
}

#[test]
fn the_links_of_a_page_are_read_once_not_every_frame() {
    let mut app = app();
    for _ in 0..50 {
        app.internal_link_at(0, AppPoint { x: 15.0, y: 40.0 });
    }
    let (page, _, links) = app.tab().doc.as_ref().unwrap().caches.internal_links.as_ref().expect("nothing was cached");
    assert_eq!((*page, links.len()), (0, 2));
    // Asking about another page replaces it, rather than answering from the first.
    assert_eq!(app.internal_link_at(1, AppPoint { x: 15.0, y: 40.0 }), None);
    assert_eq!(app.tab().doc.as_ref().unwrap().caches.internal_links.as_ref().unwrap().0, 1);
}
