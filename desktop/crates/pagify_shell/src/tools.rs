//! The Draw rail, the Modify rail, and snap — build plan phases 5, 6 and 7.
//!
//! Every operation here is `cad_kernel`'s. Nothing reimplements geometry: the
//! kernel trims, fillets, offsets and snaps, and this module's whole job is to
//! convert coordinates at the boundary, hand the kernel the objects it needs,
//! and put the results back into the layer as one edit.
//!
//! ## Why the interactive commands do not act here
//!
//! `cad_kernel`'s parser emits `Move`, `Rotate`, `Fillet` and friends as
//! argument-less commands — the app is expected to capture the clicks. So
//! [`apply`] does not perform them; it reports [`Applied::NeedsPicks`], which
//! carries the prompt to show and how many points to collect. The app drives
//! the picking and then calls the operation directly. That keeps every actual
//! geometric edit a plain function with plain arguments, which is what makes
//! this file testable without a pointer.

use std::collections::BTreeSet;

use cad_kernel::parser::{Command, ToolKind};
use cad_kernel::{DObject, Geom, UniformGrid, Vec2};
pub use cad_kernel::{SnapHit, SnapKind, SnapSet};

use crate::markup::Layer;
use crate::page_space::AppPoint;

/// What running a command against a layer did.
#[derive(Debug)]
pub enum Applied {
    /// Geometry was added, and this many objects now exist that did not.
    Added(usize),
    /// Existing objects were changed or replaced.
    Changed(usize),
    Removed(usize),
    /// An interactive tool was entered — the next clicks draw with it.
    Tool(ToolKind),
    /// The command needs the pointer before it can do anything.
    Interactive(Pick),
    /// Understood and did nothing, with a reason worth showing.
    Nothing(&'static str),
    Failed(String),
}

fn on_selection(op: Op, points: usize) -> Applied {
    Applied::Interactive(Pick { op, objects: 0, points, needs_selection: true, repeating: false })
}

/// Carry out an interactive operation once its clicks have been collected.
///
/// Every argument is already resolved: the objects are indices into the layer,
/// and every coordinate is in kernel space. That makes this a plain function
/// with plain arguments — which is the whole reason it is here and not in a
/// frame callback, because it means each tool can be tested by calling it.
pub fn run(
    layer: &mut Layer,
    op: Op,
    objects: &[(usize, Vec2)],
    points: &[Vec2],
) -> Result<String, String> {
    // One checkpoint for the whole operation, taken before anything moves.
    // A fillet replaces two objects and adds a third; whoever filleted a corner
    // expects one undo to put the corner back, not three.
    layer.begin(op.name());
    let outcome = run_inner(layer, op, objects, points);
    layer.end();

    // A refusal must not leave a step behind — undoing "nothing happened" is
    // worse than having no step at all.
    if outcome.is_err() {
        layer.forget_last_step();
    }
    outcome
}

fn run_inner(
    layer: &mut Layer,
    op: Op,
    objects: &[(usize, Vec2)],
    points: &[Vec2],
) -> Result<String, String> {
    let point = |i: usize| points.get(i).copied().ok_or_else(|| "not enough points".to_string());
    let object = |i: usize| {
        objects
            .get(i)
            .copied()
            .ok_or_else(|| "not enough objects picked".to_string())
    };

    match op {
        Op::Move => {
            let delta = point(1)? - point(0)?;
            Ok(format!("{} moved.", move_selection(layer, delta)))
        }
        Op::Copy => {
            let delta = point(1)? - point(0)?;
            Ok(format!("{} copied.", copy_selection(layer, delta)))
        }
        Op::Rotate => {
            let pivot = point(0)?;
            let to = point(1)?;
            let angle = (to.y - pivot.y).atan2(to.x - pivot.x);
            Ok(format!("{} rotated.", rotate_selection(layer, pivot, angle)))
        }
        Op::Scale => {
            let pivot = point(0)?;
            let reference = point(1)?;
            // The reference distance against the selection's own size, so a
            // click near the pivot shrinks and one far away grows.
            let span = (reference - pivot).len();
            let base = selection_span(layer, pivot);
            if base < 1e-9 {
                return Err("scale: the selection has no size to scale from".into());
            }
            let factor = span / base;
            scale_selection(layer, pivot, factor).map(|n| format!("{n} scaled to {factor:.2}×."))
        }
        Op::Mirror => {
            let n = mirror_selection(layer, point(0)?, point(1)?, false);
            Ok(format!("{n} mirrored."))
        }

        Op::Trim => {
            let (index, at) = object(0)?;
            trim_object(layer, index, at, false).map(|n| format!("trimmed into {n} piece(s)."))
        }
        Op::Extend => {
            let (index, at) = object(0)?;
            extend_object(layer, index, at, false).map(|_| "extended.".to_string())
        }
        Op::Fillet { radius } => {
            let (a, pa) = object(0)?;
            let (b, pb) = object(1)?;
            fillet_pair(layer, a, pa, b, pb, radius)
                .map(|_| format!("filleted at radius {radius}."))
        }
        Op::Chamfer { d1, d2 } => {
            let (a, pa) = object(0)?;
            let (b, pb) = object(1)?;
            chamfer_pair(layer, a, pa, b, pb, d1, d2).map(|_| "chamfered.".to_string())
        }
        Op::Offset { distance } => {
            let (index, _) = object(0)?;
            let side = point(0)?;
            offset_object(layer, index, distance, side)
                .map(|_| format!("offset by {distance}."))
        }
    }
}

/// How far the selection reaches from a pivot, for scaling by reference.
fn selection_span(layer: &Layer, pivot: Vec2) -> f64 {
    layer
        .selected_objects()
        .iter()
        .flat_map(|o| {
            let (min, max) = o.bbox();
            [min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)]
        })
        .map(|corner| (corner - pivot).len())
        .fold(0.0, f64::max)
}

/// Run a parsed kernel command against a page's markup.
pub fn apply(layer: &mut Layer, command: &Command, defaults: &mut Defaults) -> Applied {
    match command {
        Command::Add(geom) => {
            layer.begin("draw");
            layer.add(geom.clone());
            layer.end();
            Applied::Added(1)
        }
        Command::Clear => {
            let n = layer.len();
            layer.begin("clear");
            layer.select_all();
            layer.erase_selection();
            layer.end();
            Applied::Removed(n)
        }
        Command::DeleteSelected => {
            layer.begin("erase");
            let n = layer.erase_selection();
            layer.end();
            if n == 0 {
                layer.forget_last_step();
                Applied::Nothing("nothing selected.")
            } else {
                Applied::Removed(n)
            }
        }
        Command::SelectAll => {
            layer.select_all();
            Applied::Nothing("everything on this page selected.")
        }
        Command::SelectNone | Command::Select => {
            layer.clear_selection();
            Applied::Nothing("selection cleared.")
        }
        Command::SetTool(kind) => Applied::Tool(*kind),

        Command::Move => on_selection(Op::Move, 2),
        Command::Copy => on_selection(Op::Copy, 2),
        Command::Rotate => on_selection(Op::Rotate, 2),
        Command::Scale => on_selection(Op::Scale, 2),
        Command::Mirror => on_selection(Op::Mirror, 2),

        // Repeating: you trim one piece after another, and re-arming the tool
        // by hand between each is miserable. Escape ends it.
        Command::Trim => Applied::Interactive(Pick {
            op: Op::Trim,
            objects: 1,
            points: 0,
            needs_selection: false,
            repeating: true,
        }),
        Command::Extend => Applied::Interactive(Pick {
            op: Op::Extend,
            objects: 1,
            points: 0,
            needs_selection: false,
            repeating: true,
        }),

        // The radius and distances travel with the command. Dropping them, and
        // deciding the operation from the prompt text instead, is what made
        // fillet perform a move.
        Command::Fillet(radius) => {
            if let Some(r) = radius {
                defaults.fillet_radius = *r;
            }
            Applied::Interactive(Pick {
                op: Op::Fillet { radius: defaults.fillet_radius },
                objects: 2,
                points: 0,
                needs_selection: false,
                repeating: false,
            })
        }
        Command::Chamfer(distances) => {
            if let Some((d1, d2)) = distances {
                defaults.chamfer = (*d1, d2.unwrap_or(*d1));
            }
            Applied::Interactive(Pick {
                op: Op::Chamfer { d1: defaults.chamfer.0, d2: defaults.chamfer.1 },
                objects: 2,
                points: 0,
                needs_selection: false,
                repeating: false,
            })
        }
        Command::Offset(distance) => {
            if let Some(d) = distance {
                defaults.offset_distance = *d;
            }
            Applied::Interactive(Pick {
                op: Op::Offset { distance: defaults.offset_distance },
                objects: 1,
                points: 1,
                needs_selection: false,
                repeating: true,
            })
        }

        Command::Join => match {
            layer.begin("join");
            let outcome = join_selection(layer);
            layer.end();
            if !matches!(outcome, Ok(n) if n > 0) {
                layer.forget_last_step();
            }
            outcome
        } {
            Ok(0) => Applied::Nothing("nothing joined — the selection does not meet end to end."),
            Ok(n) => Applied::Changed(n),
            Err(e) => Applied::Failed(e),
        },

        Command::Undo | Command::Redo => {
            Applied::Nothing("undo is the document's, not the layer's — use `undo`.")
        }

        _ => Applied::Nothing("recognised, and it does not mean anything on a page yet."),
    }
}

/// What an interactive operation is, and what it still needs.
///
/// The distinction that the first attempt at this got wrong: some of these want
/// **objects** clicked on and some want **free points**, and they are not
/// interchangeable. Fillet needs two objects; move needs two points; offset
/// needs one object and then one point saying which side. Collapsing that into
/// "how many clicks" is how fillet ended up performing a move.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pick {
    pub op: Op,
    /// Objects to click on, in order, before any points.
    pub objects: usize,
    /// Free points to click, after the objects.
    pub points: usize,
    /// Acts on the current selection, so an empty one is worth refusing before
    /// asking anyone to start clicking.
    pub needs_selection: bool,
    /// Keeps going until Escape. Trim and extend are used on one piece after
    /// another and re-arming by hand each time is miserable.
    pub repeating: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Move,
    Copy,
    Rotate,
    Scale,
    Mirror,
    Trim,
    Extend,
    Fillet { radius: f64 },
    Chamfer { d1: f64, d2: f64 },
    Offset { distance: f64 },
}

impl Op {
    /// How this operation is named in the undo history.
    pub fn name(self) -> &'static str {
        match self {
            Op::Move => "move",
            Op::Copy => "copy",
            Op::Rotate => "rotate",
            Op::Scale => "scale",
            Op::Mirror => "mirror",
            Op::Trim => "trim",
            Op::Extend => "extend",
            Op::Fillet { .. } => "fillet",
            Op::Chamfer { .. } => "chamfer",
            Op::Offset { .. } => "offset",
        }
    }
}

impl Pick {
    /// What to ask for next, given what has been collected.
    pub fn prompt(&self, objects_done: usize, points_done: usize) -> String {
        let verb = match self.op {
            Op::Move => "move",
            Op::Copy => "copy",
            Op::Rotate => "rotate",
            Op::Scale => "scale",
            Op::Mirror => "mirror",
            Op::Trim => "trim",
            Op::Extend => "extend",
            Op::Fillet { .. } => "fillet",
            Op::Chamfer { .. } => "chamfer",
            Op::Offset { .. } => "offset",
        };

        if objects_done < self.objects {
            let which = match (self.op, objects_done) {
                (Op::Trim, _) => "click the piece to cut away",
                (Op::Extend, _) => "click the end to lengthen",
                (Op::Offset { .. }, _) => "click the object to offset",
                (_, 0) => "click the first object",
                _ => "click the second object",
            };
            return format!("{verb}: {which}");
        }

        let which = match (self.op, points_done) {
            (Op::Offset { .. }, _) => "click the side to offset towards",
            (Op::Move | Op::Copy, 0) => "base point",
            (Op::Move | Op::Copy, _) => "where to",
            (Op::Rotate, 0) => "pivot",
            (Op::Rotate, _) => "a point to swing to",
            (Op::Scale, 0) => "pivot",
            (Op::Scale, _) => "a reference distance",
            (Op::Mirror, 0) => "first point on the mirror line",
            (Op::Mirror, _) => "second point on the mirror line",
            _ => "a point",
        };
        format!("{verb}: {which}")
    }

    pub fn total(&self) -> usize {
        self.objects + self.points
    }
}

/// Values a modify command reuses when none is given, the way AutoCAD keeps the
/// last radius you filleted with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Defaults {
    pub fillet_radius: f64,
    pub chamfer: (f64, f64),
    pub offset_distance: f64,
}

impl Default for Defaults {
    fn default() -> Self {
        // Page points. Ten is a visible corner on a drawing without being a
        // gesture, and zero would make `fillet` look broken the first time.
        Defaults { fillet_radius: 10.0, chamfer: (10.0, 10.0), offset_distance: 10.0 }
    }
}

// ---------------------------------------------------------------------------
// The Modify rail. Each takes its picks already resolved, in kernel space.
// ---------------------------------------------------------------------------

fn selected(layer: &Layer) -> Vec<usize> {
    layer.selection().iter().copied().collect()
}

pub fn move_selection(layer: &mut Layer, by: Vec2) -> usize {
    let indices = selected(layer);
    for index in &indices {
        if let Some(object) = layer.objects().get(*index) {
            let moved = object.translated(by);
            layer.replace(*index, moved);
        }
    }
    indices.len()
}

/// Copy leaves the originals in place and appends the copies, then selects
/// them — so a second move acts on what was just made, which is what AutoCAD
/// does and what hands expect.
pub fn copy_selection(layer: &mut Layer, by: Vec2) -> usize {
    let copies: Vec<DObject> = selected(layer)
        .iter()
        .filter_map(|i| layer.objects().get(*i).map(|o| o.translated(by)))
        .collect();

    let count = copies.len();
    layer.clear_selection();
    let mut made = BTreeSet::new();
    for copy in copies {
        made.insert(layer.add_object(copy));
    }
    for index in made {
        layer.select_box_index(index);
    }
    count
}

pub fn rotate_selection(layer: &mut Layer, pivot: Vec2, radians: f64) -> usize {
    let indices = selected(layer);
    for index in &indices {
        if let Some(object) = layer.objects().get(*index) {
            let turned = object.rotated(pivot, radians);
            layer.replace(*index, turned);
        }
    }
    indices.len()
}

pub fn scale_selection(layer: &mut Layer, pivot: Vec2, factor: f64) -> Result<usize, String> {
    if factor.abs() < 1e-9 {
        return Err("scale: a factor of zero would collapse everything to a point".into());
    }
    let indices = selected(layer);
    for index in &indices {
        if let Some(object) = layer.objects().get(*index) {
            let scaled = object.scaled(pivot, factor);
            layer.replace(*index, scaled);
        }
    }
    Ok(indices.len())
}

pub fn mirror_selection(layer: &mut Layer, a: Vec2, b: Vec2, keep_original: bool) -> usize {
    let indices = selected(layer);
    let mirrored: Vec<DObject> = indices
        .iter()
        .filter_map(|i| layer.objects().get(*i).map(|o| o.mirrored(a, b)))
        .collect();

    let count = mirrored.len();
    if keep_original {
        for object in mirrored {
            layer.add_object(object);
        }
    } else {
        for (index, object) in indices.iter().zip(mirrored) {
            layer.replace(*index, object);
        }
    }
    count
}

pub fn offset_object(
    layer: &mut Layer,
    index: usize,
    distance: f64,
    side_hint: Vec2,
) -> Result<usize, String> {
    let object = layer
        .objects()
        .get(index)
        .ok_or_else(|| "offset: no such object".to_string())?
        .clone();

    let offset = object
        .offset(distance, side_hint)
        .map_err(|e| format!("offset: {e}"))?;
    Ok(layer.add_object(offset))
}

/// Trim a target back to whatever cuts it.
///
/// Everything else on the page is a cutter, which is AutoCAD's "trim to
/// everything" default and the behaviour anyone marking up a drawing wants —
/// selecting cutting edges first is a step that earns its keep only on a
/// crowded model.
pub fn trim_object(
    layer: &mut Layer,
    target: usize,
    pick: Vec2,
    edge_mode: bool,
) -> Result<usize, String> {
    let geom = layer
        .objects()
        .get(target)
        .ok_or_else(|| "trim: no such object".to_string())?
        .geom
        .clone();

    let cutters: Vec<Geom> = layer
        .objects()
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != target)
        .map(|(_, o)| o.geom.clone())
        .collect();

    let survivors = geom
        .trim_at(&cutters, pick, edge_mode)
        .map_err(|e| format!("trim: {e}"))?;

    let style = layer.objects()[target].style;
    let doomed: BTreeSet<usize> = [target].into_iter().collect();
    layer.remove(&doomed);

    let mut kept = 0;
    for piece in survivors {
        layer.add_object(DObject::with_style(piece, style));
        kept += 1;
    }
    Ok(kept)
}

pub fn extend_object(
    layer: &mut Layer,
    target: usize,
    pick: Vec2,
    edge_mode: bool,
) -> Result<(), String> {
    let object = layer
        .objects()
        .get(target)
        .ok_or_else(|| "extend: no such object".to_string())?
        .clone();

    let boundaries: Vec<Geom> = layer
        .objects()
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != target)
        .map(|(_, o)| o.geom.clone())
        .collect();

    let longer = object
        .geom
        .extend_to(&boundaries, pick, edge_mode)
        .map_err(|e| format!("extend: {e}"))?;

    layer.replace(target, DObject::with_style(longer, object.style));
    Ok(())
}

/// Round the corner between two objects. `radius` of zero makes a sharp corner
/// — the two are trimmed or extended to meet, and no arc is added.
pub fn fillet_pair(
    layer: &mut Layer,
    first: usize,
    first_pick: Vec2,
    second: usize,
    second_pick: Vec2,
    radius: f64,
) -> Result<usize, String> {
    let (a, b) = pair(layer, first, second, "fillet")?;

    let out = cad_kernel::fillet_geoms(&a.geom, first_pick, &b.geom, second_pick, radius)?;

    layer.replace(first, DObject::with_style(out.g1_new, a.style));
    layer.replace(second, DObject::with_style(out.g2_new, b.style));

    Ok(match out.arc {
        Some(arc) => {
            layer.add_object(DObject::with_style(arc, a.style));
            3
        }
        None => 2,
    })
}

pub fn chamfer_pair(
    layer: &mut Layer,
    first: usize,
    first_pick: Vec2,
    second: usize,
    second_pick: Vec2,
    d1: f64,
    d2: f64,
) -> Result<usize, String> {
    let (a, b) = pair(layer, first, second, "chamfer")?;

    let out = cad_kernel::chamfer_geoms(&a.geom, first_pick, &b.geom, second_pick, d1, d2)?;

    layer.replace(first, DObject::with_style(out.g1_new, a.style));
    layer.replace(second, DObject::with_style(out.g2_new, b.style));
    layer.add_object(DObject::with_style(out.bridge, a.style));
    Ok(3)
}

fn pair(
    layer: &Layer,
    first: usize,
    second: usize,
    verb: &str,
) -> Result<(DObject, DObject), String> {
    if first == second {
        return Err(format!("{verb}: pick two different objects"));
    }
    let a = layer
        .objects()
        .get(first)
        .ok_or_else(|| format!("{verb}: no such object"))?
        .clone();
    let b = layer
        .objects()
        .get(second)
        .ok_or_else(|| format!("{verb}: no such object"))?
        .clone();
    Ok((a, b))
}

/// Join the selection into as few objects as the kernel can manage.
pub fn join_selection(layer: &mut Layer) -> Result<usize, String> {
    let indices = selected(layer);
    if indices.len() < 2 {
        return Err("join: select at least two objects".into());
    }

    let input: Vec<(usize, Geom)> = indices
        .iter()
        .filter_map(|i| layer.objects().get(*i).map(|o| (*i, o.geom.clone())))
        .collect();

    let style = layer.objects()[indices[0]].style;
    let out = cad_kernel::join_geoms(&input);
    if out.merged.is_empty() {
        return Ok(0);
    }

    // `consumed_indices` index into the slice that was handed in, NOT into the
    // document. Treating them as document indices removes whatever happens to
    // sit at those positions — which, for any selection that is not the first
    // n objects, is the wrong geometry and looks like join eating the page.
    let consumed: BTreeSet<usize> = out
        .consumed_indices
        .iter()
        .filter_map(|i| input.get(*i).map(|(doc_index, _)| *doc_index))
        .collect();
    layer.remove(&consumed);

    let mut made = 0;
    for geom in out.merged {
        layer.add_object(DObject::with_style(geom, style));
        made += 1;
    }
    Ok(made)
}

// ---------------------------------------------------------------------------
// Snap — phase 7
// ---------------------------------------------------------------------------

/// A snap, back in app space and ready to draw a badge at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapped {
    pub kind: SnapKind,
    pub at: AppPoint,
    /// The object the snap sits on. `None` for an intersection, where two
    /// objects have equal claim and the kernel does not pick between them.
    pub object: Option<usize>,
}

/// Find the snap under the cursor.
///
/// `from` is the anchor a command has already collected. Perpendicular and
/// tangent are meaningless without one — there is no "perpendicular to that
/// line" until you know perpendicular *from where* — so they only offer
/// themselves once a command has supplied it, which is the kernel's own rule
/// and the reason this parameter is threaded through rather than defaulted.
pub fn snap_at(
    layer: &Layer,
    cursor: AppPoint,
    radius_pt: f64,
    enabled: SnapSet,
    forced: Option<SnapKind>,
    from: Option<AppPoint>,
) -> Option<Snapped> {
    let space = layer.space();
    let objects = layer.objects();
    if objects.is_empty() {
        return None;
    }

    // Rebuilt rather than cached: a snap query happens per frame while the
    // pointer moves, and a stale index snaps to where a line used to be.
    let grid = UniformGrid::build_auto(objects, 4.0);

    let hit: SnapHit = cad_kernel::find_snap(
        space.to_kernel(cursor),
        radius_pt,
        enabled,
        forced,
        from.map(|p| space.to_kernel(p)),
        objects,
        Some(&grid),
    )?;

    Some(Snapped {
        kind: hit.kind,
        at: space.from_kernel(hit.point),
        object: hit.dobject,
    })
}

/// Constrain a point to the horizontal or vertical from an anchor. Ortho.
pub fn orthogonal(from: AppPoint, to: AppPoint) -> AppPoint {
    if (to.x - from.x).abs() >= (to.y - from.y).abs() {
        AppPoint::new(to.x, from.y)
    } else {
        AppPoint::new(from.x, to.y)
    }
}

/// Round a point to the nearest grid intersection, in app space.
pub fn to_grid(point: AppPoint, spacing_pt: f64) -> AppPoint {
    if spacing_pt <= 0.0 {
        return point;
    }
    AppPoint::new(
        (point.x / spacing_pt).round() * spacing_pt,
        (point.y / spacing_pt).round() * spacing_pt,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_kernel::Line;

    const H: f64 = 800.0;

    fn line(ax: f64, ay: f64, bx: f64, by: f64) -> Geom {
        Geom::Line(Line { a: Vec2::new(ax, ay), b: Vec2::new(bx, by) })
    }

    #[test]
    fn a_parsed_draw_command_becomes_geometry() {
        let mut layer = Layer::new(H);
        let command = cad_kernel::parser::parse("l 0,0 100,100").expect("parses");

        let mut defaults = Defaults::default();
        assert!(matches!(apply(&mut layer, &command, &mut defaults), Applied::Added(1)));
        assert_eq!(layer.len(), 1);
    }

    #[test]
    fn a_bare_draw_verb_enters_a_tool_rather_than_drawing_nothing() {
        let mut layer = Layer::new(H);
        let command = cad_kernel::parser::parse("circle").expect("parses");
        let mut defaults = Defaults::default();
        assert!(matches!(apply(&mut layer, &command, &mut defaults), Applied::Tool(ToolKind::Circle)));
        assert!(layer.is_empty());
    }

    /// The bug this whole shape exists to prevent.
    ///
    /// The first version decided which operation to run by looking at the
    /// *prompt text* — anything that did not start with "copy" became a move.
    /// So clicking Fillet and then two objects moved the selection instead of
    /// filleting, silently and destructively. The operation now travels as data.
    #[test]
    fn each_modify_command_arms_its_own_operation() {
        let mut layer = Layer::new(H);
        let mut defaults = Defaults::default();

        let cases: [(&str, Op); 7] = [
            ("move", Op::Move),
            ("copy", Op::Copy),
            ("rotate", Op::Rotate),
            ("mirror", Op::Mirror),
            ("trim", Op::Trim),
            ("extend", Op::Extend),
            ("fillet 5", Op::Fillet { radius: 5.0 }),
        ];

        for (line, expected) in cases {
            let command = cad_kernel::parser::parse(line).expect("parses");
            match apply(&mut layer, &command, &mut defaults) {
                Applied::Interactive(pick) => assert_eq!(
                    pick.op, expected,
                    "`{line}` armed {:?}, not {expected:?}",
                    pick.op
                ),
                other => panic!("`{line}` gave {other:?}"),
            }
        }
    }

    #[test]
    fn a_radius_or_distance_travels_with_the_command_and_then_sticks() {
        let mut layer = Layer::new(H);
        let mut defaults = Defaults::default();

        match apply(&mut layer, &cad_kernel::parser::parse("fillet 25").unwrap(), &mut defaults) {
            Applied::Interactive(p) => assert_eq!(p.op, Op::Fillet { radius: 25.0 }),
            other => panic!("{other:?}"),
        }
        // Bare `fillet` reuses it, the way AutoCAD keeps the last radius.
        match apply(&mut layer, &cad_kernel::parser::parse("fillet").unwrap(), &mut defaults) {
            Applied::Interactive(p) => assert_eq!(p.op, Op::Fillet { radius: 25.0 }),
            other => panic!("{other:?}"),
        }
        assert_eq!(defaults.fillet_radius, 25.0);
    }

    #[test]
    fn objects_and_points_are_not_interchangeable() {
        let mut layer = Layer::new(H);
        let mut defaults = Defaults::default();

        // Fillet wants two objects and no free points.
        match apply(&mut layer, &cad_kernel::parser::parse("fillet").unwrap(), &mut defaults) {
            Applied::Interactive(p) => {
                assert_eq!((p.objects, p.points), (2, 0));
                assert!(!p.needs_selection, "fillet picks its own objects");
            }
            other => panic!("{other:?}"),
        }
        // Offset wants one object and then a point saying which side.
        match apply(&mut layer, &cad_kernel::parser::parse("offset 5").unwrap(), &mut defaults) {
            Applied::Interactive(p) => assert_eq!((p.objects, p.points), (1, 1)),
            other => panic!("{other:?}"),
        }
        // Move wants two points and acts on what is already selected.
        match apply(&mut layer, &cad_kernel::parser::parse("move").unwrap(), &mut defaults) {
            Applied::Interactive(p) => {
                assert_eq!((p.objects, p.points), (0, 2));
                assert!(p.needs_selection);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn running_a_fillet_makes_an_arc_and_does_not_move_anything() {
        let mut layer = Layer::new(H);
        let a = layer.add(line(0.0, 0.0, 100.0, 0.0));
        let b = layer.add(line(100.0, 0.0, 100.0, 100.0));
        layer.select_all();

        let before = layer.len();
        run(
            &mut layer,
            Op::Fillet { radius: 10.0 },
            &[(a, Vec2::new(50.0, 0.0)), (b, Vec2::new(100.0, 50.0))],
            &[],
        )
        .expect("fillets");

        assert_eq!(layer.len(), before + 1, "a fillet adds exactly the arc");
        assert!(
            layer.objects().iter().any(|o| matches!(o.geom, Geom::Arc(_))),
            "no arc — this is the move-instead-of-fillet bug"
        );
    }

    #[test]
    fn running_a_move_moves_and_running_an_offset_offsets() {
        let mut layer = Layer::new(H);
        let index = layer.add(line(0.0, 0.0, 100.0, 0.0));
        layer.select_all();

        run(&mut layer, Op::Move, &[], &[Vec2::ZERO, Vec2::new(0.0, 40.0)]).expect("moves");
        match &layer.objects()[index].geom {
            Geom::Line(l) => assert_eq!(l.a.y, 40.0),
            other => panic!("{other:?}"),
        }

        run(
            &mut layer,
            Op::Offset { distance: 10.0 },
            &[(index, Vec2::new(50.0, 40.0))],
            &[Vec2::new(50.0, 80.0)],
        )
        .expect("offsets");
        assert_eq!(layer.len(), 2, "offset adds a parallel copy");
    }

    #[test]
    fn a_prompt_asks_for_the_right_thing_at_each_step() {
        let fillet = Pick {
            op: Op::Fillet { radius: 5.0 },
            objects: 2,
            points: 0,
            needs_selection: false,
            repeating: false,
        };
        assert!(fillet.prompt(0, 0).contains("first object"));
        assert!(fillet.prompt(1, 0).contains("second object"));

        let offset = Pick {
            op: Op::Offset { distance: 5.0 },
            objects: 1,
            points: 1,
            needs_selection: false,
            repeating: true,
        };
        assert!(offset.prompt(0, 0).contains("object to offset"));
        assert!(offset.prompt(1, 0).contains("side"));
    }

    #[test]
    fn an_interactive_modify_asks_for_points_instead_of_acting() {
        let mut layer = Layer::new(H);
        layer.add(line(0.0, 0.0, 10.0, 0.0));
        layer.select_all();

        let mut defaults = Defaults::default();
        match apply(&mut layer, &cad_kernel::parser::parse("move").unwrap(), &mut defaults) {
            Applied::Interactive(pick) => {
                assert_eq!(pick.points, 2);
                assert!(pick.needs_selection);
                assert!(!pick.prompt(0, 0).is_empty());
            }
            other => panic!("expected Interactive, got {other:?}"),
        }
        assert_eq!(layer.len(), 1, "and nothing moved yet");
    }

    #[test]
    fn move_shifts_every_selected_object() {
        let mut layer = Layer::new(H);
        layer.add(line(0.0, 0.0, 10.0, 0.0));
        layer.add(line(0.0, 5.0, 10.0, 5.0));
        layer.select_all();

        assert_eq!(move_selection(&mut layer, Vec2::new(100.0, 0.0)), 2);
        match &layer.objects()[0].geom {
            Geom::Line(l) => assert_eq!(l.a, Vec2::new(100.0, 0.0)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn copy_leaves_the_original_and_selects_what_it_made() {
        let mut layer = Layer::new(H);
        layer.add(line(0.0, 0.0, 10.0, 0.0));
        layer.select_all();

        assert_eq!(copy_selection(&mut layer, Vec2::new(0.0, 50.0)), 1);
        assert_eq!(layer.len(), 2, "the original stays");
        assert_eq!(layer.selection().len(), 1, "and the copy is what is now selected");

        // A second move must act on the copy, not the original.
        move_selection(&mut layer, Vec2::new(0.0, 50.0));
        let ys: Vec<f64> = layer.objects().iter().map(|o| match &o.geom {
            Geom::Line(l) => l.a.y,
            _ => unreachable!(),
        }).collect();
        assert!(ys.contains(&0.0) && ys.contains(&100.0), "got {ys:?}");
    }

    #[test]
    fn scale_refuses_to_collapse_everything_to_a_point() {
        let mut layer = Layer::new(H);
        layer.add(line(0.0, 0.0, 10.0, 0.0));
        layer.select_all();
        assert!(scale_selection(&mut layer, Vec2::ZERO, 0.0).is_err());
    }

    #[test]
    fn mirror_can_keep_the_original_or_replace_it() {
        let mut layer = Layer::new(H);
        layer.add(line(10.0, 10.0, 20.0, 10.0));
        layer.select_all();

        mirror_selection(&mut layer, Vec2::ZERO, Vec2::new(1.0, 0.0), true);
        assert_eq!(layer.len(), 2, "keeping the original leaves both");
    }

    #[test]
    fn trim_cuts_a_line_back_to_what_crosses_it() {
        let mut layer = Layer::new(H);
        // A horizontal line crossed by a vertical one at x = 50.
        let target = layer.add(line(0.0, 0.0, 100.0, 0.0));
        layer.add(line(50.0, -10.0, 50.0, 10.0));

        // Pick the right-hand piece, which is the one to remove.
        let kept = trim_object(&mut layer, target, Vec2::new(80.0, 0.0), false)
            .expect("trims");
        assert!(kept >= 1);

        // Whatever survives must not reach past the cutter on the picked side.
        let reaches_past = layer.objects().iter().any(|o| match &o.geom {
            Geom::Line(l) => l.a.y.abs() < 1e-9 && (l.a.x > 60.0 || l.b.x > 60.0),
            _ => false,
        });
        assert!(!reaches_past, "the picked piece is still there");
    }

    #[test]
    fn fillet_with_a_radius_adds_the_arc_and_trims_both_lines() {
        let mut layer = Layer::new(H);
        let a = layer.add(line(0.0, 0.0, 100.0, 0.0));
        let b = layer.add(line(100.0, 0.0, 100.0, 100.0));

        let touched = fillet_pair(
            &mut layer, a, Vec2::new(50.0, 0.0), b, Vec2::new(100.0, 50.0), 10.0,
        ).expect("fillets");

        assert_eq!(touched, 3, "two trimmed lines and an arc");
        assert!(layer.objects().iter().any(|o| matches!(o.geom, Geom::Arc(_))));
    }

    #[test]
    fn filleting_an_object_with_itself_is_refused() {
        let mut layer = Layer::new(H);
        let a = layer.add(line(0.0, 0.0, 100.0, 0.0));
        assert!(fillet_pair(&mut layer, a, Vec2::ZERO, a, Vec2::ZERO, 5.0).is_err());
    }

    #[test]
    fn offset_makes_a_parallel_copy_on_the_side_you_point_at() {
        let mut layer = Layer::new(H);
        let index = layer.add(line(0.0, 0.0, 100.0, 0.0));

        offset_object(&mut layer, index, 10.0, Vec2::new(50.0, 20.0)).expect("offsets");
        assert_eq!(layer.len(), 2);

        let offset_y = match &layer.objects()[1].geom {
            Geom::Line(l) => l.a.y,
            _ => unreachable!(),
        };
        assert!(offset_y > 0.0, "offset landed on the wrong side: {offset_y}");
    }

    #[test]
    fn join_needs_at_least_two_objects() {
        let mut layer = Layer::new(H);
        layer.add(line(0.0, 0.0, 10.0, 0.0));
        layer.select_all();
        assert!(join_selection(&mut layer).is_err());
    }

    // -- snap ---------------------------------------------------------------

    #[test]
    fn endpoint_snap_finds_the_end_and_reports_it_in_app_space() {
        let mut layer = Layer::new(H);
        layer.add(line(100.0, 100.0, 300.0, 100.0));

        // Near the left end. Kernel (100,100) is app (100, 700).
        let snapped = snap_at(
            &layer, AppPoint::new(102.0, 698.0), 10.0, SnapSet::defaults(), None, None,
        ).expect("a snap");

        assert_eq!(snapped.kind, SnapKind::End);
        assert!((snapped.at.x - 100.0).abs() < 1e-6);
        assert!((snapped.at.y - 700.0).abs() < 1e-6, "snap came back in kernel space");
    }

    #[test]
    fn a_forced_snap_overrides_what_is_enabled() {
        let mut layer = Layer::new(H);
        layer.add(line(0.0, 100.0, 200.0, 100.0));

        let nothing_enabled = SnapSet::default();
        let midpoint = snap_at(
            &layer,
            AppPoint::new(105.0, 700.0),
            20.0,
            nothing_enabled,
            Some(SnapKind::Mid),
            None,
        ).expect("a forced snap ignores the toggles");

        assert_eq!(midpoint.kind, SnapKind::Mid);
        assert!((midpoint.at.x - 100.0).abs() < 1e-6);
    }

    #[test]
    fn snapping_an_empty_layer_finds_nothing_rather_than_panicking() {
        let layer = Layer::new(H);
        assert!(snap_at(&layer, AppPoint::new(0.0, 0.0), 10.0, SnapSet::defaults(), None, None).is_none());
    }

    #[test]
    fn ortho_keeps_whichever_axis_moved_further() {
        let from = AppPoint::new(100.0, 100.0);
        assert_eq!(orthogonal(from, AppPoint::new(200.0, 110.0)), AppPoint::new(200.0, 100.0));
        assert_eq!(orthogonal(from, AppPoint::new(110.0, 200.0)), AppPoint::new(100.0, 200.0));
    }

    #[test]
    fn grid_snap_rounds_to_the_nearest_intersection_and_zero_spacing_is_a_no_op() {
        assert_eq!(to_grid(AppPoint::new(107.0, 93.0), 10.0), AppPoint::new(110.0, 90.0));
        assert_eq!(to_grid(AppPoint::new(107.0, 93.0), 0.0), AppPoint::new(107.0, 93.0));
    }
}
