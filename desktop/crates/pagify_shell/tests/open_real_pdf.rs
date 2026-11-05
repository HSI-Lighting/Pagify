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
