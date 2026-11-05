//! Opening a CAD set: `DWTC C3, Unit 110 - Arch.Pack.pdf`, six pages, two of
//! them ~720,000 lines. Asking every page for markup to restore loaded each
//! page whole, on the UI thread, before the first frame; a file that cannot
//! hold any is now told apart without opening a page.
//!
//! Skips, with a line saying so, when the file is not on this machine.
//!
//! `cargo test -p pagify_shell --release --test open_real_pdf -- --nocapture`

use pagify_shell::Session;

const SET: &str = r"C:\Users\hsili\Desktop\DWTC C3, Unit 110 - Arch.Pack.pdf";

#[test]
fn a_cad_set_nobody_marked_is_not_asked_page_by_page() {
    if !std::path::Path::new(SET).is_file() {
        eprintln!("skipped: the set is not at {SET}");
        return;
    }
    let session = Session::open(SET).expect("open");
    let started = std::time::Instant::now();
    let maybe = session.may_hold_markup();
    let told = started.elapsed();

    let started = std::time::Instant::now();
    let mut found = 0;
    for page in 0..session.page_count().expect("pages") {
        found += usize::from(session.restore_markup(page).expect("restore").is_some());
    }
    let asked = started.elapsed();
    eprintln!("told apart in {told:?}; asking every page took {asked:?} and found {found}");

    assert_eq!(found, 0, "the set holds markup after all");
    assert!(!maybe, "a set with no markup was a maybe, so every page would be loaded to find out");
}

/// What the pointer needs to know about a page of 720,000 lines, read in one
/// load of it, says what the four reads said, in a fraction of their time.
#[test]
fn a_heavy_page_is_read_for_the_pointer_in_one_load() {
    if !std::path::Path::new(SET).is_file() {
        eprintln!("skipped: the set is not at {SET}");
        return;
    }
    let session = Session::open(SET).expect("open");
    let page = 1;
    let started = std::time::Instant::now();
    let scale = session.page_scale(page).expect("scale");
    let marks = session.annotations(page).expect("marks");
    let links = session.internal_links(page).expect("links");
    let characters = session.characters(page).expect("characters");
    let apart = started.elapsed();

    let started = std::time::Instant::now();
    let reads = session.page_reads(page).expect("reads");
    let once = started.elapsed();
    eprintln!("four reads {apart:?}, one load {once:?}");

    assert_eq!(reads.scale.expect("scale"), scale);
    assert_eq!(reads.annotations.expect("marks"), marks);
    assert_eq!(reads.links.expect("links"), links);
    let read_once = pagify_shell::reader::Characters::new(reads.characters.expect("characters"));
    assert_eq!(read_once.text(), characters.text());
    assert!(once * 2 < apart, "one load took {once:?} against {apart:?} for four");
}
