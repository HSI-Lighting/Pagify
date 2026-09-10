//! The live markup layer — build plan §5.2 and phase 4.
//!
//! The tempting implementation is to write each stroke straight into the PDF as
//! an ink annotation the moment it is drawn. **Do not.** Trim, fillet, extend,
//! offset and join all need live geometry with object identity: once two lines
//! have become ink annotations there is nothing left to fillet, because the
//! corner between them is not a thing the PDF knows about.
//!
//! So each page carries a `cad_kernel` object table, held in the session, and
//! the drawing and modify tools operate on that. It renders as an overlay above
//! the rasterised page, and on save it is written into the PDF as real content
//! *and also* preserved, so it is still editable when the file is reopened —
//! see [`crate::commit`].
//!
//! Geometry here is in **kernel space**: page points, bottom-left origin, y up.
//! Every coordinate arriving from the UI is in app space and converts through
//! [`PageSpace`], which is the only place in either crate allowed to flip.

use std::collections::{BTreeSet, HashMap};

use cad_kernel::{DObject, Document, Geom, Line, UniformGrid, Vec2};

use crate::page_space::{AppPoint, PageSpace};

/// How close a click has to be to count as a hit, in page points.
///
/// Points rather than pixels on purpose: a hit tolerance in pixels means a line
/// gets easier to select the further you zoom in, which is right, and *also*
/// that a script clicking the same coordinate selects different things at
/// different zooms, which is not. Callers scale this by zoom if they want the
/// former.
pub const HIT_TOLERANCE_PT: f64 = 3.0;

/// Which of the two selection gestures a drag is.
///
/// AutoCAD's rule, and the one every draughtsman's hands already know:
/// left-to-right takes only what is wholly inside, right-to-left takes anything
/// it touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    /// Left to right: only objects entirely within the box.
    Window,
    /// Right to left: any object the box touches.
    Crossing,
}

impl Drag {
    pub fn of(from: AppPoint, to: AppPoint) -> Drag {
        if to.x >= from.x {
            Drag::Window
        } else {
            Drag::Crossing
        }
    }
}

/// One page's markup.
pub struct Layer {
    doc: Document,
    grid: UniformGrid,
    space: PageSpace,
    selection: BTreeSet<usize>,
    /// How many times this layer's geometry has changed.
    ///
    /// A counter rather than a flag, and not a count of objects: a move or a
    /// fillet changes the drawing without changing how many things are in it,
    /// so anything derived from `len()` would call an edited page unedited and
    /// let it be closed without a word.
    edits: u64,

    /// States to go back to, oldest first, and states to go forward to.
    ///
    /// Whole-layer snapshots rather than inverse operations. A page's markup is
    /// tens of objects, so a snapshot is cheap — and the alternative means
    /// writing an inverse for every tool and getting fillet's wrong once.
    /// Correctness here is worth more than the bytes.
    past: Vec<Step>,
    future: Vec<Step>,
    /// Nesting depth of the operation in progress, so a fillet — which replaces
    /// two objects and adds a third — is one undo step and not three.
    open: u32,
}

/// A state to return to, and the name of what moved on from it.
#[derive(Debug, Clone)]
struct Step {
    objects: Vec<DObject>,
    label: String,
}

impl Layer {
    pub fn new(page_height_pt: f64) -> Self {
        Layer {
            doc: Document::default(),
            grid: UniformGrid::empty(),
            space: PageSpace::new(page_height_pt),
            selection: BTreeSet::new(),
            edits: 0,
            past: Vec::new(),
            future: Vec::new(),
            open: 0,
        }
    }

    /// Begin an operation, so everything it does undoes together.
    ///
    /// Nested calls are absorbed: `fillet` replaces two objects and adds a
    /// third, and someone who filleted a corner expects one Ctrl-Z to put the
    /// corner back — not three, with two intermediate states nobody ever saw.
    pub fn begin(&mut self, label: &str) {
        if self.open == 0 {
            self.past.push(Step { objects: self.doc.dobjects.clone(), label: label.into() });
            if self.past.len() > Self::UNDO_DEPTH {
                self.past.remove(0);
            }
            // A new edit forfeits the redo branch: there is no coherent forward
            // history once the past has changed underneath it.
            self.future.clear();
        }
        self.open += 1;
    }

    pub fn end(&mut self) {
        self.open = self.open.saturating_sub(1);
    }

    /// How many steps back this layer can go.
    pub const UNDO_DEPTH: usize = 64;

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Drop the most recent checkpoint without changing anything.
    ///
    /// For an operation that turned out to do nothing — a trim that missed, an
    /// erase with an empty selection. Undoing "nothing happened" is worse than
    /// having no step to undo: it looks broken, because nothing visibly moves.
    pub fn forget_last_step(&mut self) {
        self.past.pop();
    }

    /// Step back, returning what was undone.
    pub fn undo(&mut self) -> Option<String> {
        let step = self.past.pop()?;
        self.future.push(Step {
            objects: std::mem::replace(&mut self.doc.dobjects, step.objects),
            label: step.label.clone(),
        });
        self.selection.clear();
        self.edits += 1;
        self.reindex();
        Some(step.label)
    }

    /// Step forward again.
    pub fn redo(&mut self) -> Option<String> {
        let step = self.future.pop()?;
        self.past.push(Step {
            objects: std::mem::replace(&mut self.doc.dobjects, step.objects),
            label: step.label.clone(),
        });
        self.selection.clear();
        self.edits += 1;
        self.reindex();
        Some(step.label)
    }

    /// How many edits this layer has seen. Compared against the value at the
    /// last save to know whether there is anything to lose.
    pub fn edits(&self) -> u64 {
        self.edits
    }

    pub fn space(&self) -> PageSpace {
        self.space
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    pub fn objects(&self) -> &[DObject] {
        &self.doc.dobjects
    }

    pub fn is_empty(&self) -> bool {
        self.doc.dobjects.is_empty()
    }

    pub fn len(&self) -> usize {
        self.doc.dobjects.len()
    }

    /// Add geometry, in kernel space.
    pub fn add(&mut self, geom: Geom) -> usize {
        let index = self.doc.push(DObject::new(geom));
        self.edits += 1;
        self.reindex();
        index
    }

    pub fn add_object(&mut self, object: DObject) -> usize {
        let index = self.doc.push(object);
        self.edits += 1;
        self.reindex();
        index
    }

    /// Remove objects by index. Indices are taken all at once because removing
    /// them one at a time renumbers the ones not yet removed.
    pub fn remove(&mut self, indices: &BTreeSet<usize>) -> usize {
        if indices.is_empty() {
            return 0;
        }
        let before = self.doc.dobjects.len();
        let mut index = 0;
        self.doc.dobjects.retain(|_| {
            let keep = !indices.contains(&index);
            index += 1;
            keep
        });
        self.selection.clear();
        self.edits += 1;
        self.reindex();
        before - self.doc.dobjects.len()
    }

    pub fn replace(&mut self, index: usize, object: DObject) -> bool {
        match self.doc.dobjects.get_mut(index) {
            Some(slot) => {
                *slot = object;
                self.edits += 1;
                self.reindex();
                true
            }
            None => false,
        }
    }

    fn reindex(&mut self) {
        // Rebuilt wholesale rather than updated. `UniformGrid::update` exists
        // for moving a few objects among many; a markup layer is small enough
        // that a rebuild is cheaper than tracking which indices moved, and much
        // harder to get subtly wrong.
        self.grid = UniformGrid::build_auto(&self.doc.dobjects, 4.0);
    }

    // -- picking ------------------------------------------------------------

    /// The object under a point, nearest first.
    ///
    /// Candidates come from the spatial index and are then measured **per
    /// shape**, never by bounding box — the bounding box of a large arc is
    /// mostly empty space, and picking by it means clicking nothing and
    /// selecting something.
    pub fn hit(&self, at: AppPoint, tolerance_pt: f64) -> Option<usize> {
        let point = self.space.to_kernel(at);

        let mut best: Option<(usize, f64)> = None;
        for candidate in self.grid.query_near(point, tolerance_pt) {
            let index = candidate as usize;
            let Some(object) = self.doc.dobjects.get(index) else { continue };
            if !self.doc.is_selectable(index) {
                continue;
            }

            let distance = object.distance_to_point(point);
            if distance <= tolerance_pt && best.map_or(true, |(_, b)| distance < b) {
                best = Some((index, distance));
            }
        }
        best.map(|(index, _)| index)
    }

    /// Objects a box selects, by the gesture the drag describes.
    pub fn in_box(&self, from: AppPoint, to: AppPoint) -> Vec<usize> {
        let drag = Drag::of(from, to);
        let a = self.space.to_kernel(from);
        let b = self.space.to_kernel(to);

        let min = Vec2::new(a.x.min(b.x), a.y.min(b.y));
        let max = Vec2::new(a.x.max(b.x), a.y.max(b.y));

        let mut hits = Vec::new();
        for candidate in self.grid.query_bbox(min, max) {
            let index = candidate as usize;
            let Some(object) = self.doc.dobjects.get(index) else { continue };
            if !self.doc.is_selectable(index) {
                continue;
            }

            let (o_min, o_max) = object.bbox();
            let inside = o_min.x >= min.x && o_min.y >= min.y && o_max.x <= max.x && o_max.y <= max.y;

            let selected = match drag {
                Drag::Window => inside,
                // Touching, not merely bbox-overlapping. The kernel's own
                // intersection code answers this exactly, against the four
                // edges as line segments — a diagonal line whose bounding box
                // overlaps the corner of the selection box does not actually
                // cross it, and a bbox test would take it anyway.
                Drag::Crossing => inside || crosses_box(&object.geom, min, max),
            };

            if selected {
                hits.push(index);
            }
        }
        hits.sort_unstable();
        hits
    }

    // -- selection ----------------------------------------------------------

    pub fn selection(&self) -> &BTreeSet<usize> {
        &self.selection
    }

    pub fn selected_objects(&self) -> Vec<&DObject> {
        self.selection
            .iter()
            .filter_map(|i| self.doc.dobjects.get(*i))
            .collect()
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn select_all(&mut self) {
        self.selection = (0..self.doc.dobjects.len()).collect();
    }

    /// Click. Replaces the selection unless `add`, which extends it — and
    /// toggles, so a second shift-click takes an object back out.
    pub fn select_at(&mut self, at: AppPoint, tolerance_pt: f64, add: bool) -> Option<usize> {
        let hit = self.hit(at, tolerance_pt);
        match hit {
            Some(index) => {
                if !add {
                    self.selection.clear();
                    self.selection.insert(index);
                } else if !self.selection.insert(index) {
                    self.selection.remove(&index);
                }
            }
            None if !add => self.selection.clear(),
            None => {}
        }
        hit
    }

    /// Add one known index to the selection. Used by operations that create
    /// objects and want them selected afterwards.
    pub fn select_box_index(&mut self, index: usize) {
        if index < self.doc.dobjects.len() {
            self.selection.insert(index);
        }
    }

    pub fn select_box(&mut self, from: AppPoint, to: AppPoint, add: bool) -> usize {
        let hits = self.in_box(from, to);
        if !add {
            self.selection.clear();
        }
        for index in &hits {
            self.selection.insert(*index);
        }
        hits.len()
    }

    /// Delete whatever is selected. AutoCAD's ERASE, and the reason the focus
    /// guard exists.
    pub fn erase_selection(&mut self) -> usize {
        let doomed = self.selection.clone();
        self.remove(&doomed)
    }
}

/// Whether geometry actually meets the box, rather than merely sharing a
/// bounding box with it.
fn crosses_box(geom: &Geom, min: Vec2, max: Vec2) -> bool {
    let corners = [
        Vec2::new(min.x, min.y),
        Vec2::new(max.x, min.y),
        Vec2::new(max.x, max.y),
        Vec2::new(min.x, max.y),
    ];

    for i in 0..4 {
        let edge = Geom::Line(Line { a: corners[i], b: corners[(i + 1) % 4] });
        if !cad_kernel::intersect(geom, &edge).is_empty() {
            return true;
        }
    }
    false
}

/// Every page's markup, held for the lifetime of the open document.
#[derive(Default)]
pub struct Markup {
    pages: HashMap<usize, Layer>,
}

impl Markup {
    /// Drop one page's layer.
    ///
    /// For when its marks have been written into the document itself: leaving
    /// the layer in place would paint them a second time, over the real ones.
    pub fn forget(&mut self, index: usize) {
        self.pages.remove(&index);
    }

    /// The layer for a page, created empty on first use.
    ///
    /// Takes the page height because a layer cannot convert a coordinate
    /// without it, and a layer that guessed would put every mark on the wrong
    /// half of the page.
    pub fn page(&mut self, index: usize, page_height_pt: f64) -> &mut Layer {
        self.pages
            .entry(index)
            .or_insert_with(|| Layer::new(page_height_pt))
    }

    pub fn existing(&self, index: usize) -> Option<&Layer> {
        self.pages.get(&index)
    }

    pub fn existing_mut(&mut self, index: usize) -> Option<&mut Layer> {
        self.pages.get_mut(&index)
    }

    /// Pages that actually carry marks — what a save has to walk.
    pub fn marked_pages(&self) -> Vec<usize> {
        let mut pages: Vec<usize> = self
            .pages
            .iter()
            .filter(|(_, layer)| !layer.is_empty())
            .map(|(index, _)| *index)
            .collect();
        pages.sort_unstable();
        pages
    }

    pub fn is_empty(&self) -> bool {
        self.pages.values().all(Layer::is_empty)
    }

    /// A number that changes whenever any page's markup does.
    ///
    /// What "unsaved" is measured against. Summed across pages rather than kept
    /// per page because a document is saved whole, and someone who drew on page
    /// nine and closed from page one must still be warned.
    pub fn revision(&self) -> u64 {
        self.pages.values().map(Layer::edits).sum()
    }

    /// Marks that would be lost right now, and the pages they are on.
    pub fn unsaved(&self) -> (usize, usize) {
        let pages: Vec<&Layer> = self.pages.values().filter(|l| !l.is_empty()).collect();
        (pages.iter().map(|l| l.len()).sum(), pages.len())
    }

    pub fn clear(&mut self) {
        self.pages.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_kernel::{Circle, Line};

    const H: f64 = 800.0;

    fn layer_with_a_horizontal_line() -> Layer {
        let mut layer = Layer::new(H);
        // In app space this runs across the page at y = 700 (near the bottom).
        layer.add(Geom::Line(Line {
            a: Vec2::new(100.0, 100.0),
            b: Vec2::new(300.0, 100.0),
        }));
        layer
    }

    #[test]
    fn a_mark_lands_where_the_click_was_in_app_space() {
        let layer = layer_with_a_horizontal_line();
        // The line is at kernel y=100, so app y = 800 - 100 = 700.
        assert_eq!(layer.hit(AppPoint::new(200.0, 700.0), HIT_TOLERANCE_PT), Some(0));
        assert_eq!(layer.hit(AppPoint::new(200.0, 100.0), HIT_TOLERANCE_PT), None,
            "the mirrored position must NOT hit — that would be a missing flip");
    }

    #[test]
    fn picking_is_per_shape_not_by_bounding_box() {
        // A circle's bounding box is mostly not the circle. A click in the
        // middle is inside the box and nowhere near the curve.
        let mut layer = Layer::new(H);
        layer.add(Geom::Circle(Circle { center: Vec2::new(200.0, 200.0), radius: 100.0 }));

        let centre_app = AppPoint::new(200.0, H as f32 as f64 - 200.0);
        assert_eq!(layer.hit(centre_app, HIT_TOLERANCE_PT), None,
            "the middle of a circle is not the circle");

        // On the curve, it hits.
        let on_curve = AppPoint::new(300.0, 600.0);
        assert_eq!(layer.hit(on_curve, HIT_TOLERANCE_PT), Some(0));
    }

    #[test]
    fn the_nearest_object_wins_a_crowded_click() {
        let mut layer = Layer::new(H);
        layer.add(Geom::Line(Line { a: Vec2::new(0.0, 100.0), b: Vec2::new(400.0, 100.0) }));
        layer.add(Geom::Line(Line { a: Vec2::new(0.0, 102.0), b: Vec2::new(400.0, 102.0) }));

        // App y 697 == kernel y 103 — closer to the second line.
        assert_eq!(layer.hit(AppPoint::new(200.0, 697.0), 5.0), Some(1));
    }

    #[test]
    fn drag_direction_decides_the_gesture() {
        assert_eq!(Drag::of(AppPoint::new(0.0, 0.0), AppPoint::new(10.0, 10.0)), Drag::Window);
        assert_eq!(Drag::of(AppPoint::new(10.0, 0.0), AppPoint::new(0.0, 10.0)), Drag::Crossing);
    }

    #[test]
    fn a_window_takes_only_what_is_wholly_inside() {
        let layer = layer_with_a_horizontal_line();

        // Box covering the whole line (app space, dragged left to right).
        assert_eq!(layer.in_box(AppPoint::new(50.0, 650.0), AppPoint::new(350.0, 750.0)), vec![0]);

        // Box over only half of it — a window does not take it.
        assert!(layer.in_box(AppPoint::new(50.0, 650.0), AppPoint::new(200.0, 750.0)).is_empty());
    }

    #[test]
    fn a_crossing_takes_anything_it_touches() {
        let layer = layer_with_a_horizontal_line();
        // Same half-covering box, dragged right to left.
        assert_eq!(
            layer.in_box(AppPoint::new(200.0, 650.0), AppPoint::new(50.0, 750.0)),
            vec![0]
        );
    }

    #[test]
    fn a_crossing_does_not_take_something_that_only_shares_a_bounding_box() {
        // The test that distinguishes real intersection from a bbox test. A
        // diagonal from bottom-left to top-right passes nowhere near a small
        // box tucked into the top-left corner of its bounding box.
        let mut layer = Layer::new(H);
        layer.add(Geom::Line(Line { a: Vec2::new(0.0, 0.0), b: Vec2::new(400.0, 400.0) }));

        // Kernel box x 0..40, y 360..400 — inside the diagonal's bbox, far off
        // the line itself. In app space that is y 400..440, dragged right to left.
        let hits = layer.in_box(AppPoint::new(40.0, 440.0), AppPoint::new(0.0, 400.0));
        assert!(hits.is_empty(), "a bounding-box test would have taken the diagonal");
    }

    #[test]
    fn clicking_empty_space_clears_the_selection_but_shift_clicking_does_not() {
        let mut layer = layer_with_a_horizontal_line();
        layer.select_at(AppPoint::new(200.0, 700.0), HIT_TOLERANCE_PT, false);
        assert_eq!(layer.selection().len(), 1);

        layer.select_at(AppPoint::new(10.0, 10.0), HIT_TOLERANCE_PT, true);
        assert_eq!(layer.selection().len(), 1, "shift-clicking nothing keeps what was chosen");

        layer.select_at(AppPoint::new(10.0, 10.0), HIT_TOLERANCE_PT, false);
        assert!(layer.selection().is_empty());
    }

    #[test]
    fn shift_clicking_a_selected_object_takes_it_back_out() {
        let mut layer = layer_with_a_horizontal_line();
        let at = AppPoint::new(200.0, 700.0);

        layer.select_at(at, HIT_TOLERANCE_PT, false);
        layer.select_at(at, HIT_TOLERANCE_PT, true);
        assert!(layer.selection().is_empty(), "a second shift-click deselects");
    }

    #[test]
    fn erasing_removes_every_selected_object_at_once() {
        let mut layer = Layer::new(H);
        for i in 0..5 {
            let y = 100.0 + i as f64 * 50.0;
            layer.add(Geom::Line(Line { a: Vec2::new(0.0, y), b: Vec2::new(100.0, y) }));
        }

        layer.select_all();
        assert_eq!(layer.erase_selection(), 5);
        assert!(layer.is_empty());
    }

    #[test]
    fn removing_several_objects_does_not_renumber_the_ones_still_going() {
        // The bug of removing one at a time: after the first removal every
        // later index is off by one, and the wrong things get deleted.
        let mut layer = Layer::new(H);
        for i in 0..5 {
            let y = i as f64 * 10.0;
            layer.add(Geom::Line(Line { a: Vec2::new(0.0, y), b: Vec2::new(1.0, y) }));
        }

        let doomed: BTreeSet<usize> = [0, 2, 4].into_iter().collect();
        assert_eq!(layer.remove(&doomed), 3);
        assert_eq!(layer.len(), 2);

        // The survivors are the ones that were at 1 and 3 — y = 10 and y = 30.
        let ys: Vec<f64> = layer
            .objects()
            .iter()
            .map(|o| match &o.geom {
                Geom::Line(l) => l.a.y,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(ys, vec![10.0, 30.0]);
    }

    #[test]
    fn each_page_gets_its_own_layer_and_only_marked_pages_are_listed() {
        let mut markup = Markup::default();
        markup.page(0, H);
        markup.page(3, H).add(Geom::Line(Line {
            a: Vec2::ZERO,
            b: Vec2::new(10.0, 10.0),
        }));

        assert_eq!(markup.marked_pages(), vec![3], "page 0 was touched but never marked");
        assert!(!markup.is_empty());
    }

    #[test]
    fn two_pages_of_different_heights_each_flip_around_their_own() {
        let mut markup = Markup::default();
        markup.page(0, 800.0).add(Geom::Line(Line {
            a: Vec2::new(0.0, 100.0),
            b: Vec2::new(100.0, 100.0),
        }));
        markup.page(1, 400.0).add(Geom::Line(Line {
            a: Vec2::new(0.0, 100.0),
            b: Vec2::new(100.0, 100.0),
        }));

        // Same kernel y, different page heights, so different app y.
        assert_eq!(
            markup.existing(0).unwrap().hit(AppPoint::new(50.0, 700.0), HIT_TOLERANCE_PT),
            Some(0)
        );
        assert_eq!(
            markup.existing(1).unwrap().hit(AppPoint::new(50.0, 300.0), HIT_TOLERANCE_PT),
            Some(0)
        );
    }
}

#[cfg(test)]
mod unsaved_tests {
    use super::*;
    use cad_kernel::Line;

    #[test]
    fn an_edit_that_changes_no_object_count_still_counts_as_an_edit() {
        // The reason this is a counter and not a length. Moving a mark, or
        // filleting two of them, changes the drawing without changing how many
        // things are in it — and anything derived from `len()` would call that
        // page unedited and let it be closed without a word.
        let mut layer = Layer::new(800.0);
        layer.add(Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(10.0, 0.0) }));
        let after_drawing = layer.edits();
        let count = layer.len();

        let moved = layer.objects()[0].translated(Vec2::new(5.0, 5.0));
        layer.replace(0, moved);

        assert_eq!(layer.len(), count, "the test moved rather than added");
        assert!(layer.edits() > after_drawing, "a move did not register as an edit");
    }

    #[test]
    fn the_revision_moves_for_a_change_on_any_page() {
        // Someone who drew on page nine and closed from page one must still be
        // warned, so this is summed across the document rather than kept per
        // page.
        let mut markup = Markup::default();
        let start = markup.revision();

        markup.page(8, 800.0).add(Geom::Line(Line { a: Vec2::ZERO, b: Vec2::new(1.0, 1.0) }));
        assert!(markup.revision() > start, "an edit on page nine was invisible");
    }

    #[test]
    fn an_untouched_document_has_nothing_to_lose() {
        let mut markup = Markup::default();
        markup.page(0, 800.0); // visited, never drawn on
        assert_eq!(markup.revision(), 0);
        assert_eq!(markup.unsaved(), (0, 0));
    }

    #[test]
    fn unsaved_counts_the_marks_and_the_pages_carrying_them() {
        let mut markup = Markup::default();
        for page in [0usize, 3, 3] {
            let height = 800.0;
            markup.page(page, height).add(Geom::Line(Line {
                a: Vec2::ZERO,
                b: Vec2::new(10.0, 10.0),
            }));
        }
        assert_eq!(markup.unsaved(), (3, 2), "three marks across two pages");
    }
}

#[cfg(test)]
mod undo_tests {
    use super::*;
    use cad_kernel::Line;

    fn line(y: f64) -> Geom {
        Geom::Line(Line { a: Vec2::new(0.0, y), b: Vec2::new(100.0, y) })
    }

    fn layer_with(n: usize) -> Layer {
        let mut layer = Layer::new(800.0);
        for i in 0..n {
            layer.begin("draw");
            layer.add(line(i as f64 * 10.0));
            layer.end();
        }
        layer
    }

    #[test]
    fn a_drawn_mark_can_be_taken_back() {
        let mut layer = layer_with(3);
        assert_eq!(layer.len(), 3);

        assert_eq!(layer.undo().as_deref(), Some("draw"));
        assert_eq!(layer.len(), 2);
        layer.undo();
        layer.undo();
        assert_eq!(layer.len(), 0);
    }

    #[test]
    fn undoing_past_the_beginning_stops_rather_than_emptying() {
        let mut layer = layer_with(2);
        for _ in 0..10 {
            layer.undo();
        }
        assert_eq!(layer.len(), 0);
        assert!(!layer.can_undo());
        assert!(layer.undo().is_none());
    }

    #[test]
    fn redo_puts_it_back() {
        let mut layer = layer_with(2);
        layer.undo();
        assert_eq!(layer.len(), 1);

        assert_eq!(layer.redo().as_deref(), Some("draw"));
        assert_eq!(layer.len(), 2);
        assert!(!layer.can_redo());
    }

    #[test]
    fn a_new_edit_forfeits_the_redo_branch() {
        // There is no coherent forward history once the past has changed
        // underneath it, and offering one would restore a state that never
        // followed from what is now on the page.
        let mut layer = layer_with(3);
        layer.undo();
        assert!(layer.can_redo());

        layer.begin("draw");
        layer.add(line(999.0));
        layer.end();

        assert!(!layer.can_redo(), "a stale redo survived a new edit");
    }

    /// The behaviour that makes undo feel right rather than pedantic.
    #[test]
    fn one_operation_is_one_step_however_many_objects_it_touched() {
        // A fillet replaces two objects and adds a third. Whoever filleted a
        // corner expects one undo to put the corner back — not three, with two
        // intermediate states nobody ever saw.
        let mut layer = Layer::new(800.0);
        layer.begin("draw");
        layer.add(line(0.0));
        layer.end();
        layer.begin("draw");
        layer.add(line(50.0));
        layer.end();
        let before = layer.len();

        layer.begin("fillet");
        layer.begin("nested"); // absorbed
        layer.add(line(25.0));
        layer.replace(0, DObject::new(line(1.0)));
        layer.end();
        layer.end();

        assert_eq!(layer.undo().as_deref(), Some("fillet"));
        assert_eq!(layer.len(), before, "the fillet came back in pieces");
    }

    #[test]
    fn an_operation_that_did_nothing_leaves_no_step_to_undo() {
        // Undoing "nothing happened" looks broken, because nothing moves.
        let mut layer = layer_with(1);
        layer.begin("erase");
        let removed = layer.erase_selection(); // nothing is selected
        layer.end();
        assert_eq!(removed, 0);
        layer.forget_last_step();

        assert_eq!(layer.undo().as_deref(), Some("draw"), "the empty step was kept");
    }

    #[test]
    fn the_history_is_bounded() {
        let mut layer = layer_with(Layer::UNDO_DEPTH + 20);
        let mut steps = 0;
        while layer.undo().is_some() {
            steps += 1;
            assert!(steps <= Layer::UNDO_DEPTH + 1, "the history is unbounded");
        }
        assert!(steps <= Layer::UNDO_DEPTH);
    }

    #[test]
    fn undoing_a_move_restores_where_it_was() {
        // The case a count of objects cannot see.
        let mut layer = layer_with(1);
        layer.select_all();

        layer.begin("move");
        let moved = layer.objects()[0].translated(Vec2::new(0.0, 40.0));
        layer.replace(0, moved);
        layer.end();

        match &layer.objects()[0].geom {
            Geom::Line(l) => assert_eq!(l.a.y, 40.0),
            _ => unreachable!(),
        }
        layer.undo();
        match &layer.objects()[0].geom {
            Geom::Line(l) => assert_eq!(l.a.y, 0.0, "the move was not undone"),
            _ => unreachable!(),
        }
    }
}
