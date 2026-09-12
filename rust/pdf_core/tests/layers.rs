//! What a page draws, in the order it draws it — and changing that order.
//!
//! # Why the order is the feature
//!
//! A PDF has no z-index. What is drawn later covers what came before, so the
//! page's own drawing order *is* its stacking, and the only way to put
//! something on top is to move the operators that draw it. That is the whole
//! mechanism, and it is why this cannot be done by setting a property.
//!
//! Reported from use: "when I move this image it goes behind a layer", and
//! "if there are layers then I need options to choose which comes at the top".
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test layers
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, DrawnKind, Stacking};

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None)
        .expect("open")
}

/// A stable name for a thing on a page, so it can be found again after the page
/// has been rewritten and PDFium has renumbered its objects.
fn named(doc: &PdfiumDocument, page: usize) -> Vec<String> {
    doc.drawn_objects(page)
        .expect("objects")
        .iter()
        .map(|d| format!("{}:{}", d.kind.describe(), d.label))
        .collect()
}

/// **The list is in drawing order, bottom first.**
#[test]
fn a_page_says_what_it_draws_and_in_which_order() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("pictures.pdf");
    let drawn = doc.drawn_objects(0).expect("objects");

    assert_eq!(drawn.len(), 5, "the fixture draws three runs and two pictures");
    // The fixture's content stream draws its text, then its pictures, then one
    // more line — so that is what the list has to say.
    let kinds: Vec<DrawnKind> = drawn.iter().map(|d| d.kind).collect();
    assert_eq!(
        kinds,
        vec![
            DrawnKind::Words,
            DrawnKind::Words,
            DrawnKind::Picture,
            DrawnKind::Picture,
            DrawnKind::Words
        ],
        "the order is not the page's drawing order: {drawn:#?}"
    );

    // Each entry has to be nameable in a list somebody is reading.
    assert!(
        drawn[0].label.contains("Pagify moving fixture"),
        "a run should be labelled by its words: {:?}",
        drawn[0].label
    );
    assert!(
        drawn[2].label.contains("4×4"),
        "a picture should be labelled by its size: {:?}",
        drawn[2].label
    );
}

/// **Bringing something to the front puts it last, which is on top.**
#[test]
fn anything_it_can_restack_lands_at_the_front_and_disturbs_nothing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("pictures.pdf");
    let before = doc.drawn_objects(0).expect("objects");
    let was = named(&doc, 0);
    let ink = doc.page(0).and_then(|p| p.text()).unwrap_or_default();
    drop(doc);

    for target in &before {
        let mut doc = open("pictures.pdf");
        doc.restack(0, target.object, Stacking::Front).expect("bring it to the front");

        let now = named(&doc, 0);
        let me = format!("{}:{}", target.kind.describe(), target.label);
        assert_eq!(now.last(), Some(&me), "it did not land at the front: {now:#?}");

        // Everything else in the order it was already in.
        let rest_before: Vec<&String> = was.iter().filter(|d| **d != me).collect();
        let rest_after: Vec<&String> = now.iter().filter(|d| **d != me).collect();
        assert_eq!(rest_before, rest_after, "moving {me:?} re-ordered the rest");

        // And the page still says the same thing. Compared as sorted
        // characters, because extraction order follows the stream and a
        // deliberate restack is meant to change that.
        let mut a: Vec<char> = ink.chars().filter(|c| !c.is_whitespace()).collect();
        let mut b: Vec<char> = doc
            .page(0)
            .and_then(|p| p.text())
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "moving {me:?} changed what the page says");
    }
}

/// **And sending it back puts it first, which is underneath everything.**
#[test]
fn anything_it_can_restack_lands_at_the_back() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("pictures.pdf");
    let before = doc.drawn_objects(0).expect("objects");
    let was = named(&doc, 0);
    drop(doc);

    for target in &before {
        let mut doc = open("pictures.pdf");
        doc.restack(0, target.object, Stacking::Back).expect("send it to the back");

        let now = named(&doc, 0);
        let me = format!("{}:{}", target.kind.describe(), target.label);
        assert_eq!(now.first(), Some(&me), "it did not land at the back: {now:#?}");

        let rest_before: Vec<&String> = was.iter().filter(|d| **d != me).collect();
        let rest_after: Vec<&String> = now.iter().filter(|d| **d != me).collect();
        assert_eq!(rest_before, rest_after, "moving {me:?} re-ordered the rest");
    }
}

/// **A picture brought forward really is drawn after what it was behind.**
///
/// The order of the list is one way of asking; the other is where the operator
/// now sits in the stream, which is what a reader actually sees. Asserted on
/// the stream so that a list built from something other than drawing order
/// could not make this pass.
#[test]
fn the_operator_really_does_move_down_the_stream() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("pictures.pdf");
    let pictures = doc.images_on(0).expect("images");
    let first = pictures.first().cloned().expect("a picture");
    let stream = doc.page_stream(0).expect("stream");
    let before = String::from_utf8_lossy(&stream).to_string();
    drop(doc);

    // The lower picture is drawn before the last line of text in the fixture.
    let text_at = before.find("Text below both pictures").expect("the fixture's last line");
    let image_at = before.find("/Im0 Do").expect("the first picture");
    assert!(image_at < text_at, "the fixture should draw the picture first");

    let mut doc = open("pictures.pdf");
    doc.restack(0, first.object, Stacking::Front).expect("bring it to the front");
    let after = String::from_utf8_lossy(&doc.page_stream(0).expect("stream")).to_string();

    let text_now = after.find("Text below both pictures").expect("the last line");
    let image_now = after.rfind("/Im0 Do").expect("the picture");
    assert!(
        image_now > text_now,
        "the picture is still drawn before the text, so it is still underneath"
    );
}

/// **What it cannot do, it declines** — and says which thing it declined.
#[test]
fn something_it_cannot_place_is_refused_rather_than_guessed_at() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("pictures.pdf");
    let count = doc.drawn_objects(0).expect("objects").len();

    let refused = doc.restack(0, count + 5, Stacking::Front);
    assert!(refused.is_err(), "an object that is not there should be refused");

    // And the page is untouched.
    let after = doc.drawn_objects(0).expect("objects");
    assert_eq!(after.len(), count, "a refused restack changed the page");
}

/// **A page that draws through a group can still be read.**
///
/// Reported from use, on a brochure: *"there looks like it had layers and we
/// are not able to read it"*. A page laid out in a design program routinely
/// draws a whole panel through one form XObject, and a list that stopped at the
/// page's own objects said `group, 145 × 63 pt` and nothing else — technically
/// true and of no use to anybody.
#[test]
fn what_a_group_draws_is_listed_under_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("forms.pdf");
    let drawn = doc.drawn_objects(0).expect("objects");

    // The page itself draws a heading and the same form twice.
    let top: Vec<&pdf_core::document::DrawnObject> =
        drawn.iter().filter(|d| d.depth == 0).collect();
    assert_eq!(top.len(), 3, "the page draws a heading and two groups: {top:#?}");
    assert_eq!(
        top.iter().filter(|d| d.kind == DrawnKind::Group).count(),
        2,
        "both groups should be listed: {top:#?}"
    );

    // And what is inside them is listed too, with its words.
    let inside: Vec<&pdf_core::document::DrawnObject> =
        drawn.iter().filter(|d| d.depth > 0).collect();
    assert!(
        inside.iter().any(|d| d.label.contains("Inside a form")),
        "the words inside the group were not listed: {inside:#?}"
    );
    assert!(
        inside.iter().any(|d| d.kind == DrawnKind::Shape),
        "the shape inside the group was not listed: {inside:#?}"
    );

    // Each one sits directly after the group that draws it, so the list reads
    // in the order the page paints.
    let first_group = drawn.iter().position(|d| d.kind == DrawnKind::Group).expect("a group");
    assert!(drawn[first_group + 1].depth > 0, "a group's contents do not follow it");
}

/// **What a group draws cannot be re-stacked on its own.**
///
/// A form's contents are a stream shared by every place the page draws it —
/// this fixture draws the same one twice — so moving something in there would
/// move it in both. The group is what moves.
#[test]
fn something_inside_a_group_is_not_offered_as_movable() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("forms.pdf");
    let drawn = doc.drawn_objects(0).expect("objects");

    for entry in &drawn {
        assert_eq!(
            entry.movable,
            entry.depth == 0,
            "movable does not follow depth: {entry:#?}"
        );
    }
    // And the two groups really are the same form drawn twice, which is what
    // makes this the honest answer rather than a limitation.
    let groups: Vec<&pdf_core::document::DrawnObject> =
        drawn.iter().filter(|d| d.kind == DrawnKind::Group).collect();
    assert_eq!(groups.len(), 2);
    assert_eq!(
        groups[0].rect.right - groups[0].rect.left,
        groups[1].rect.right - groups[1].rect.left,
        "the fixture should draw one form twice"
    );
}

/// **Shapes re-stack too.**
///
/// Asked for from use: *"send shapes, images, texts behind and in front like
/// layers"*. A path is a run of construction operators closed by a painting
/// one, and which of them is which page object is settled by counting — see
/// `path_operators`, measured against PDFium on ten pages of a real report.
#[test]
fn a_shape_can_be_sent_behind_and_brought_in_front() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("covered.pdf");
    let before = named(&doc, 0);
    let shape = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Shape)
        .expect("the fixture's panel");
    let me = format!("{}:{}", shape.kind.describe(), shape.label);
    drop(doc);

    // To the back: the panel stops covering the picture it was painted over.
    let mut doc = open("covered.pdf");
    doc.restack(0, shape.object, Stacking::Back).expect("send the panel back");
    let now = named(&doc, 0);
    assert_eq!(now.first(), Some(&me), "the panel did not go to the back: {now:#?}");
    let rest_before: Vec<&String> = before.iter().filter(|d| **d != me).collect();
    let rest_after: Vec<&String> = now.iter().filter(|d| **d != me).collect();
    assert_eq!(rest_before, rest_after, "sending the panel back re-ordered the rest");

    // And to the front.
    let mut doc = open("covered.pdf");
    doc.restack(0, shape.object, Stacking::Front).expect("bring the panel forward");
    let now = named(&doc, 0);
    assert_eq!(now.last(), Some(&me), "the panel did not come to the front: {now:#?}");
}

/// **Every kind the list names can be re-stacked, or says why not.**
///
/// The three kinds are the point of the feature — words, pictures and shapes —
/// and a regression that quietly dropped one would otherwise show up only as a
/// button that did nothing.
#[test]
fn words_pictures_and_shapes_can_all_be_re_stacked() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("covered.pdf");
    let drawn = doc.drawn_objects(0).expect("objects");
    drop(doc);

    let mut seen: Vec<DrawnKind> = Vec::new();
    for entry in &drawn {
        let mut doc = open("covered.pdf");
        doc.restack(0, entry.object, Stacking::Front)
            .unwrap_or_else(|e| panic!("{:?} ({:?}) could not be re-stacked: {e}", entry.kind, entry.label));
        seen.push(entry.kind);
    }
    for kind in [DrawnKind::Words, DrawnKind::Picture, DrawnKind::Shape] {
        assert!(seen.contains(&kind), "{kind:?} was never exercised: {seen:?}");
    }
}

/// **One step up puts it over exactly the thing that was above it.**
///
/// Asked for from use: a panel with the order in it, and the means to move
/// things up and down. A jump to either end is easy — nothing is in force at
/// the head or tail of a stream — and a single step is not: the landing point
/// is wherever the neighbour ends, with whatever transform and clip are in
/// force there.
#[test]
fn moving_up_one_swaps_it_with_its_neighbour_and_nothing_else() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("pictures.pdf");
    let was = named(&doc, 0);
    let drawn = doc.drawn_objects(0).expect("objects");
    drop(doc);

    // Every one but the last can go up; every one but the first can go down.
    for (position, target) in drawn.iter().enumerate() {
        if position + 1 < drawn.len() {
            let mut doc = open("pictures.pdf");
            doc.restack(0, target.object, Stacking::Up).expect("move up one");
            let mut expected = was.clone();
            expected.swap(position, position + 1);
            assert_eq!(named(&doc, 0), expected, "up from {position}");
        }
        if position > 0 {
            let mut doc = open("pictures.pdf");
            doc.restack(0, target.object, Stacking::Down).expect("move down one");
            let mut expected = was.clone();
            expected.swap(position, position - 1);
            assert_eq!(named(&doc, 0), expected, "down from {position}");
        }
    }

    // And the ends say so rather than doing nothing quietly.
    let mut doc = open("pictures.pdf");
    let last = drawn.last().expect("something").object;
    assert!(doc.restack(0, last, Stacking::Up).is_err(), "the top can go no higher");
    let first = drawn.first().expect("something").object;
    assert!(doc.restack(0, first, Stacking::Down).is_err(), "the bottom can go no lower");
}

/// **Something drawn inside a clip can be re-stacked, and stays clipped.**
///
/// The first version refused this case outright. Measured on a real brochure
/// page, that was 22 of 50 shapes and 9 of the words and pictures — most of
/// what anybody would want to move. The clips in force are now carried: each
/// replayed under the transform it was set under, inside the block that
/// carries the object.
///
/// Asserted on what is drawn, through the words: the line held inside the
/// panel is 30-point type wider than the panel, so if the clip were lost it
/// would spread across the page.
#[test]
fn something_inside_a_clip_keeps_its_clip_when_re_stacked() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("clipped.pdf");
    let held = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("Held inside"))
        .expect("the clipped words");
    let width_before = held.rect.right - held.rect.left;
    drop(doc);

    for to in [Stacking::Front, Stacking::Back] {
        let mut doc = open("clipped.pdf");
        doc.restack(0, held.object, to).expect("re-stack the clipped words");
        let now = doc
            .text_runs(0)
            .expect("runs")
            .into_iter()
            .find(|r| r.text.contains("Held inside"))
            .expect("the words are still there");
        assert!(
            (now.rect.right - now.rect.left) <= width_before + 1.0,
            "{to:?}: the clip was lost — the words spread from {width_before:.0}pt to {:.0}pt",
            now.rect.right - now.rect.left
        );
        // And they are where they were: re-stacking is not moving.
        assert!(
            (now.rect.left - held.rect.left).abs() < 0.5 && (now.rect.top - held.rect.top).abs() < 0.5,
            "{to:?}: the words moved from {:?} to {:?}",
            held.rect,
            now.rect
        );
    }
}

/// **A shape that paints and clips is split: the paint moves, the clip stays.**
///
/// `W f` is one path doing two jobs. Sending the panel to the back has to take
/// its paint there and leave its clip exactly where it was, or the words it was
/// holding in spill out. The stay-behind copy is the same path closed with
/// `n`, which paints nothing.
#[test]
fn a_shape_that_paints_and_clips_leaves_its_clip_behind_when_re_stacked() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("clipped.pdf");
    let panel = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Shape)
        .expect("the clipping panel");
    let held = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("Held inside"))
        .expect("the clipped words");
    let width_before = held.rect.right - held.rect.left;
    drop(doc);

    let mut doc = open("clipped.pdf");
    doc.restack(0, panel.object, Stacking::Back).expect("send the panel back");

    // The words are still held in.
    let now = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("Held inside"))
        .expect("the words are still there");
    assert!(
        (now.rect.right - now.rect.left) <= width_before + 1.0,
        "the clip went with the paint — the words spread to {:.0}pt",
        now.rect.right - now.rect.left
    );
    // The panel is now first, and still the same size where it was.
    let first = doc.drawn_objects(0).expect("objects").into_iter().next().expect("something");
    assert_eq!(first.kind, DrawnKind::Shape, "the panel is not at the back");
    assert!(
        (first.rect.left - panel.rect.left).abs() < 0.5
            && (first.rect.right - panel.rect.right).abs() < 0.5,
        "the panel changed shape: {:?} then {:?}",
        panel.rect,
        first.rect
    );
}
