//! The AutoCAD plot that started this: `AA125 -LIGHTING LAYOUT.PDF`, every page
//! `/Rotate 270`, its labels set on their side (text matrix `0 -1 1 0`) so that the
//! turned page shows them level. Before the page's own turn was taken into account
//! 88% of its text was thrown out of the paragraph detector as sideways text.
//!
//! Skips, with a line saying so, when the file is not on this machine.
//!
//! `cargo test -p pagify_shell --release --test turned_real_pdf -- --nocapture`

use pagify_shell::block_input::*;
use pagify_shell::Session;

const PLOT: &str = r"C:\Users\hsili\Downloads\AA125 -LIGHTING LAYOUT.PDF";

#[test]
fn a_cad_plot_turned_upright_by_its_page_is_read_as_level_text() {
    if !std::path::Path::new(PLOT).is_file() {
        eprintln!("skipped: the plot is not at {PLOT}");
        return;
    }
    let session = Session::open(PLOT).expect("open");
    let size = session.page_size(0).expect("size");
    let snapshot = session.page_text_snapshot(0).expect("snapshot");
    let objects = snapshot.runs.len();
    let pb = build_page_blocks(0, 1, 1, snapshot);

    let (w, h) = (size.width_pt, size.height_pt);
    assert!(w > h, "the plot is shown landscape: {w} x {h}");
    // Every text rectangle lies on the page as shown (it used to lie on the
    // unturned one, 1684 wide, so half of them ran off the right of this one).
    let beyond = pb.runs.values().filter(|r| r.rect.bottom > h + 1.0 || r.rect.right > w + 1.0).count();
    assert_eq!(beyond, 0, "{beyond} of {objects} text objects are off the page as shown");

    let lines: usize = pb.blocks.iter().map(|b| b.lines.len()).sum();
    let thrown_out = pb.excluded.rotated.len();
    eprintln!(
        "{objects} text objects, {} blocks, {lines} lines, {thrown_out} thrown out as rotated, {} degenerate",
        pb.blocks.len(),
        pb.excluded.degenerate.len()
    );
    // What is still thrown out is text set at a real angle on the sheet (about a
    // tenth of it: 7, 14 and 38 degree labels). None of it may be text the page
    // shows level.
    let level_but_thrown_out = pb
        .excluded
        .rotated
        .iter()
        .filter(|o| {
            pb.styles.get(o).is_some_and(|s| (s.axis.0 - 1.0).abs() < 1e-3 && s.axis.1.abs() < 1e-3)
        })
        .count();
    assert_eq!(level_but_thrown_out, 0, "level text was thrown out as sideways text");
    assert!(pb.frags.len() >= 700, "only {} of {objects} text objects reached the detector", pb.frags.len());
    assert!(lines >= 150, "only {lines} lines found on a plot with hundreds of labels");
}
