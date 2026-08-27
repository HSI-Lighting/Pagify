import CoreGraphics
import Foundation
import SwiftUI

// Choosing what to capture, and what a capture *is*. Android's
// `ui/components/CaptureOverlay.kt` with `core/Capture.kt` and
// `core/CaptureLayout.kt` behind it.
//
// **Not a screenshot.** The chosen region is re-rendered from the document by
// the engine, which is what makes "only the contents of the PDF — no
// notifications, no popups from other apps" a property of the design rather than
// something to filter out: those pixels never exist.

// `CaptureFormat`, `CaptureScale` and `CaptureFill` live in `SettingsScreen.swift`
// with the rest of what outlives a document: the quality and the format somebody
// last chose are a habit, not a property of the file being read.

/// One page's share of a capture.
///
/// `crop` is in that page's own points; `dest` is where it belongs in the
/// picture, in capture units. The two are separate because a capture is not a
/// page: it is a rectangle someone dragged on screen, which may take the bottom
/// of one page, the gap below it, and the top of the next.
struct CaptureTile {
    let pageIndex: Int
    let crop: PageRect
    let dest: PageRect

    /// `pageIndex` rather than `page_index`: the engine renames struct fields to
    /// camel case on this wire.
    var json: [String: Any] {
        ["pageIndex": pageIndex, "crop": crop.json, "dest": dest.json]
    }
}

/// One capture, fully specified.
///
/// Sized in **capture units** — screen pixels at the zoom the reader was at —
/// with `scale` multiplying that for the export. The framing comes from what was
/// on screen; the resolution does not, which is what lets a capture be sharper
/// than the display that framed it.
struct CaptureRequest {
    var tiles: [CaptureTile]
    var width: CGFloat
    var height: CGFloat
    /// Shows wherever no page reaches: between two pages, and past the edge of
    /// one. The reader's own background, so a capture looks like what was on
    /// screen.
    var background: MarkColor
    /// The page the drag started on. For the file name and the label on the
    /// header only — a capture spanning two pages has to be called something.
    var originPage: Int
    var scale: CaptureScale = .high
    var format: CaptureFormat = .png
    /// 1–100. Ignored for PNG.
    var quality: Int = 92
    /// The ring drawn with the lasso, in capture units; empty for a plain box.
    ///
    /// The picture is still the ring's bounding box — an image is a rectangle —
    /// but everything outside the ring is painted over with `background` by the
    /// engine, so a detail can be lifted off a busy drawing without the things
    /// beside it.
    ///
    /// Part of the request rather than a one-off argument because the editor
    /// re-exports at other scales and formats, and a mask that survived only the
    /// first render would quietly come back as a rectangle the moment somebody
    /// chose a sharper one.
    var mask: [CGPoint] = []

    /// The picture's own coordinate space, which is what markup is drawn in.
    ///
    /// A mark drawn across the join between two pages belongs to neither of them,
    /// so marks are positioned against the capture rather than against any page
    /// inside it.
    var localBounds: PageRect {
        PageRect(left: 0, top: 0, right: width, bottom: height)
    }

    var tilesWireJSON: String {
        guard let data = try? JSONSerialization.data(withJSONObject: tiles.map(\.json)),
              let text = String(data: data, encoding: .utf8) else { return "[]" }
        return text
    }
}

/// A capture that has been taken, with what it took and what it looks like.
///
/// The bytes are what gets saved, shared or pasted, byte for byte; `picture` is a
/// downscaled copy for the screen and is never what is exported.
struct CapturePreview: Identifiable {
    /// The file name is unique per capture — it carries a timestamp — so it is
    /// the identity a presentation can key on.
    var id: String { fileName }

    let request: CaptureRequest
    let bytes: Data
    let fileName: String
    let picture: CGImage

    var sizeLabel: String {
        bytes.count >= 1024 * 1024
            ? String(format: "%.1f MB", Double(bytes.count) / (1024 * 1024))
            : "\((bytes.count + 1023) / 1024) KB"
    }
}

/// The smallest crop worth capturing, in page points.
///
/// Below this a drag is a tap that moved: a stray finger on the page should not
/// produce a two-pixel image and a share sheet. Roughly 3 mm square on paper.
let minimumCapturePoints: CGFloat = 8

extension PageRect {
    /// True when a dragged rectangle is big enough to mean it.
    var isWorthCapturing: Bool {
        (right - left) >= minimumCapturePoints && (bottom - top) >= minimumCapturePoints
    }

    var width: CGFloat { right - left }
    var height: CGFloat { bottom - top }

    /// The same rectangle grown by `margin` on every side.
    func inflatedBy(_ margin: CGFloat) -> PageRect {
        PageRect(left: left - margin, top: top - margin,
                 right: right + margin, bottom: bottom + margin)
    }
}

/// File name for a capture.
///
/// Sortable, unambiguous and safe on every filesystem: the source document, the
/// page as the reader numbers it, and the time. A gallery full of `image_1.png`
/// is what this exists to avoid.
func captureFileName(documentName: String, pageIndex: Int,
                     format: CaptureFormat, timestamp: String) -> String {
    // Only ASCII letters, digits, space, underscore and hyphen survive — the one
    // set every filesystem this can reach agrees about.
    let allowed = Set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 _-")
    let kept = (documentName as NSString).deletingPathExtension
        .filter { allowed.contains($0) }
        .trimmingCharacters(in: .whitespaces)
    let stem = String(kept.prefix(48))
    return "\(stem.isEmpty ? "Pagify" : stem) p\(pageIndex + 1) \(timestamp).\(format.fileExtension)"
}

// ------------------------------------------------------------- the arithmetic --

/// Where a page sits in the reader, and how big it is in its own points.
///
/// `bounds` is in reader points — the same space a drag is reported in — so the
/// two can be intersected directly.
extension AnnotationColors {
    /// What shows wherever no page reaches — between two pages, and past the edge
    /// of one. The reader's own backdrop, so a capture looks like what was on
    /// screen rather than announcing where the sheets ended.
    static let captureBackground = MarkColor(argb: 0xFF_1C_1C_1E)
}

struct PlacedPage {
    let pageIndex: Int
    let bounds: PageRect
    /// The page's natural size, in its own points.
    let sizePoints: CGSize
}

/// Turn a dragged rectangle into one tile per page it touches.
///
/// This is the arithmetic behind "capture what I can see, not what fits on one
/// page". The reader stacks pages in a column with gaps, so a box dragged around
/// something interesting routinely crosses a join: the bottom of one page, the
/// gap, and the top of the next. Capturing only the page the drag began on is
/// what made the feature feel broken.
///
/// Kept here, pure, rather than inside a gesture handler, because it is the part
/// that can be wrong in ways nobody notices — a tile off by the height of a gap
/// looks perfectly plausible until you compare it with the screen.
func captureTiles(for drag: PageRect, pages: [PlacedPage]) -> [CaptureTile] {
    pages.compactMap { page -> CaptureTile? in
        guard let visible = page.bounds.intersection(drag),
              visible.width >= 1, visible.height >= 1 else { return nil }

        // Points of screen per point of page, for this page. Per page rather than
        // shared: the reader draws every page to the same width, so an A3 sheet
        // and an A5 one are at very different scales on the same screen.
        let horizontal = page.bounds.width / page.sizePoints.width
        let vertical = page.bounds.height / page.sizePoints.height
        // Finite as well as positive: a page whose size has not been measured yet
        // is zero by zero, and dividing by that gives infinity rather than
        // anything a `<= 0` check would catch. The tile it produced looked valid
        // and cropped nothing.
        guard horizontal.isFinite, vertical.isFinite, horizontal > 0, vertical > 0 else {
            return nil
        }

        return CaptureTile(
            pageIndex: page.pageIndex,
            crop: PageRect(left: (visible.left - page.bounds.left) / horizontal,
                           top: (visible.top - page.bounds.top) / vertical,
                           right: (visible.right - page.bounds.left) / horizontal,
                           bottom: (visible.bottom - page.bounds.top) / vertical),
            // Relative to the drag, because the picture's origin is the drag's
            // top-left and not the reader's.
            dest: PageRect(left: visible.left - drag.left, top: visible.top - drag.top,
                           right: visible.right - drag.left, bottom: visible.bottom - drag.top))
    }
}

extension PageRect {
    /// The overlap, or nil when there is none.
    func intersection(_ other: PageRect) -> PageRect? {
        let box = PageRect(left: max(left, other.left), top: max(top, other.top),
                           right: min(right, other.right), bottom: min(bottom, other.bottom))
        return box.right > box.left && box.bottom > box.top ? box : nil
    }
}

/// Where the magnified page sits on screen, in the zoomed view's own points.
///
/// Trivial arithmetic, pulled out of the view so it can be tested at all. It is
/// the one place the zoomed capture can be wrong, and a capture whose crop is
/// derived from the wrong rectangle still produces a plausible picture — of the
/// wrong part of the page.
func zoomedPageBounds(offset: CGPoint, baseWidth: CGFloat, baseHeight: CGFloat,
                      scale: CGFloat) -> PageRect {
    PageRect(left: offset.x, top: offset.y,
             right: offset.x + baseWidth * scale, bottom: offset.y + baseHeight * scale)
}

/// The rectangle a drawn ring will be captured as.
///
/// An image is a rectangle, so a lasso still produces one: its bounding box. What
/// the ring changes is what survives *inside* that box.
///
/// Nil for anything that does not enclose a usable area, which is a stricter
/// question than "is the box big enough": a drag straight across the page in
/// lasso mode passes every size check and encloses nothing at all, so the capture
/// came back blank and the editor opened on an empty grey rectangle. The area is
/// what the ring means.
func lassoBounds(_ outline: [CGPoint]) -> PageRect? {
    guard outline.count >= 3 else { return nil }

    var left = CGFloat.greatestFiniteMagnitude
    var top = CGFloat.greatestFiniteMagnitude
    var right = -CGFloat.greatestFiniteMagnitude
    var bottom = -CGFloat.greatestFiniteMagnitude
    for point in outline {
        guard point.x.isFinite, point.y.isFinite else { return nil }
        left = min(left, point.x)
        top = min(top, point.y)
        right = max(right, point.x)
        bottom = max(bottom, point.y)
    }

    let box = PageRect(left: left, top: top, right: right, bottom: bottom)
    guard box.isWorthCapturing else { return nil }

    let enclosed = enclosedArea(outline)
    // Two floors, because they catch different mistakes. The absolute one rejects
    // a ring around nothing; the proportional one rejects a long thin smear,
    // which has plenty of absolute area and still shows almost nothing.
    guard enclosed >= minimumCapturePoints * minimumCapturePoints else { return nil }
    guard enclosed >= box.width * box.height * minimumRingFullness else { return nil }
    return box
}

/// How much a drawn ring actually encloses, by the shoelace formula.
///
/// Unsigned, because a ring drawn anticlockwise encloses exactly as much as the
/// same ring drawn clockwise. Closed for us, as everywhere else the ring is
/// treated: the last point joins the first.
private func enclosedArea(_ outline: [CGPoint]) -> CGFloat {
    var twiceArea: CGFloat = 0
    for index in outline.indices {
        let here = outline[index]
        let next = outline[(index + 1) % outline.count]
        twiceArea += here.x * next.y - next.x * here.y
    }
    return abs(twiceArea) / 2
}

/// The least of its own bounding box a ring may enclose.
///
/// Low on purpose: a ring around an L-shaped detail, or a crescent along the edge
/// of a drawing, is a legitimate shape that fills very little of its box. This is
/// only here to reject the shapes that enclose *nothing* — a straight drag, a
/// scribble back and forth — which no amount of size checking catches.
private let minimumRingFullness: CGFloat = 0.02

/// A drawn ring in the picture's own coordinates.
///
/// The same move `captureTiles` makes for a tile's destination: the picture's
/// origin is the drag's top-left, not the reader's, so every point shifts by the
/// bounding box's corner. Without it the ring would be masked against the
/// top-left of the screen and the capture would come back almost entirely blank.
/// The ring, as the engine reads it: a flat array of `{x, y}` in capture units.
func maskWireJSON(_ outline: [CGPoint]) -> String {
    let points = outline.map { ["x": $0.x, "y": $0.y] }
    guard let data = try? JSONSerialization.data(withJSONObject: points),
          let text = String(data: data, encoding: .utf8) else { return "[]" }
    return text
}

func captureMask(for drag: PageRect, outline: [CGPoint]) -> [CGPoint] {
    guard outline.count >= 3 else { return [] }
    return outline.map { CGPoint(x: $0.x - drag.left, y: $0.y - drag.top) }
}

// ---------------------------------------------------------------- the gesture --

/// Drag around what to capture.
///
/// Deliberately **over the whole reader** rather than inside a page. A capture is
/// whatever was framed on screen, and the reader stacks pages in a column, so the
/// interesting box very often takes the bottom of one page, the gap below it and
/// the top of the next. Living inside one page's layer made that impossible to
/// express — the drag was clipped to the page it started on — and it also put the
/// gesture behind the scroll container, which sometimes took it instead.
///
/// Two shapes, one gesture. A box is the default. With `lasso` the same drag
/// traces a ring instead, for the thing a box cannot say: a detail on a busy
/// drawing with a title block beside it, one fitting out of a schedule. The
/// picture is still the ring's bounding box, because an image is a rectangle —
/// what the ring buys is that everything outside it comes back blank.
///
/// Both are reported in this view's own points; turning that into per-page tiles
/// and a mask is `captureTiles` and `captureMask`.
extension View {
    /// - Parameters:
    ///   - active: whether the snapshot tool is held. False leaves every touch
    ///     alone, so the reader scrolls normally — a disabled handler that
    ///     consumed events and did nothing would still swallow the scroll.
    ///   - lasso: whether the drag traces a ring rather than dragging a box.
    ///   - onCapture: the region, and the ring that framed it — empty for a box.
    func captureOverlay(active: Bool, lasso: Bool,
                        onCapture: @escaping (PageRect, [CGPoint]) -> Void) -> some View {
        modifier(CaptureOverlayModifier(active: active, lasso: lasso, onCapture: onCapture))
    }
}

private struct CaptureOverlayModifier: ViewModifier {
    let active: Bool
    let lasso: Bool
    let onCapture: (PageRect, [CGPoint]) -> Void

    @State private var box: PageRect?
    /// The ring is appended to in place and redrawn off a counter, rather than
    /// being a new list every frame. A slow, careful drag around a detail is
    /// hundreds of samples, and copying the list per sample is quadratic in
    /// exactly the case the tool exists for.
    @State private var ring = RingSamples()
    @State private var ringRevision = 0
    @State private var start = CGPoint.zero
    /// Whether this drag has already begun. `DragGesture` has no start callback,
    /// and reading a zero translation as "the beginning" restarts the drag every
    /// time a finger pauses on the pixel it set out from.
    @State private var dragging = false

    func body(content: Content) -> some View {
        content.overlay {
            if active {
                Canvas { context, size in
                    // The revision is read so the draw is invalidated as the ring
                    // grows: the ring itself is a plain list behind a reference
                    // and cannot invalidate anything on its own.
                    if ringRevision >= 0, ring.points.count > 1 {
                        draw(ring: ring.points, in: context, size: size)
                    } else if let box {
                        draw(marquee: box, in: context, size: size)
                    }
                }
                .contentShape(Rectangle())
                .gesture(drag)
            }
        }
        // The two shapes read the same gesture differently, so a half-drawn one
        // must not survive the switch: a box left over from before would still be
        // there to commit once the ring was chosen.
        .onChange(of: lasso) { _, _ in
            dragging = false
            box = nil
            ring.points.removeAll()
            ringRevision += 1
        }
    }

    private var drag: some Gesture {
        DragGesture(minimumDistance: 0)
            .onChanged { value in
                if !dragging {
                    dragging = true
                    start = value.startLocation
                    box = nil
                    ring.points.removeAll()
                    if lasso { ring.points.append(value.startLocation) }
                    ringRevision += 1
                }
                if lasso {
                    // Thinned as it goes: touch reports far more samples than the
                    // shape needs, and every one of them is a point the engine has
                    // to fill a path with later.
                    let last = ring.points.last
                    if last == nil || hypot(value.location.x - last!.x,
                                            value.location.y - last!.y) >= ringStep {
                        ring.points.append(value.location)
                        ringRevision += 1
                    }
                } else {
                    box = PageRect(from: start, to: value.location)
                }
            }
            .onEnded { _ in
                dragging = false
                if lasso {
                    // `lassoBounds` decides whether it encloses anything; a ring
                    // too small to mean it is a tap that wandered.
                    let outline = ring.points
                    if let bounds = lassoBounds(outline) { onCapture(bounds, outline) }
                } else if let box, box.isWorthCapturing {
                    // A drag too small to mean it is a tap that moved. Capturing
                    // it would put a postage stamp and an editor in front of
                    // somebody who was trying to scroll.
                    onCapture(box, [])
                }
                box = nil
                ring.points.removeAll()
                ringRevision += 1
            }
    }

    /// The region a capture will take.
    ///
    /// Dimming everything outside it rather than only outlining it: the outline
    /// says where the edges are, the dimming says what will be in the picture,
    /// and the second is the question somebody dragging this actually has.
    private func draw(marquee: PageRect, in context: GraphicsContext, size: CGSize) {
        let shade = Color.black.opacity(shadeAlpha)
        let inner = CGRect(x: marquee.left, y: marquee.top,
                           width: marquee.width, height: marquee.height)

        // Four bands around the selection: cheaper and more predictable than a
        // clipped full-size rectangle, which fights the layer's own clip.
        context.fill(Path(CGRect(x: 0, y: 0, width: size.width, height: max(0, inner.minY))),
                     with: .color(shade))
        context.fill(Path(CGRect(x: 0, y: inner.maxY, width: size.width,
                                 height: max(0, size.height - inner.maxY))),
                     with: .color(shade))
        context.fill(Path(CGRect(x: 0, y: inner.minY, width: max(0, inner.minX),
                                 height: inner.height)),
                     with: .color(shade))
        context.fill(Path(CGRect(x: inner.maxX, y: inner.minY,
                                 width: max(0, size.width - inner.maxX), height: inner.height)),
                     with: .color(shade))

        context.stroke(Path(inner), with: .color(.white), lineWidth: borderWidth)
        // A dark hairline inside the white one, so the edge is visible against
        // both a white page and a dark image.
        context.stroke(Path(inner), with: .color(.black.opacity(0.55)), lineWidth: borderWidth / 2)
    }

    /// The ring, and the same promise the marquee makes: what is dimmed is what
    /// will not be in the picture.
    private func draw(ring points: [CGPoint], in context: GraphicsContext, size: CGSize) {
        let path = ringPath(points)
        var shaded = context
        // Everything but the ring, knocked back. `.evenOdd` over the whole
        // rectangle plus the ring is what leaves the inside untouched.
        var hole = Path(CGRect(origin: .zero, size: size))
        hole.addPath(path)
        shaded.clip(to: hole, style: FillStyle(eoFill: true))
        shaded.fill(Path(CGRect(origin: .zero, size: size)),
                    with: .color(.black.opacity(shadeAlpha)))

        context.stroke(path, with: .color(.white), lineWidth: borderWidth)
        context.stroke(path, with: .color(.black.opacity(0.55)), lineWidth: borderWidth / 2)
    }
}

/// The ring's samples, held behind a reference so appending one does not copy the
/// list. See the comment on `CaptureOverlayModifier.ring`.
private final class RingSamples {
    var points: [CGPoint] = []
}

/// The ring as a closed curve through its samples.
///
/// Quadratics through the midpoints rather than lines between the points: touch
/// reports samples several points apart on a quick drag, and joining those with
/// straight edges shows the drag's sampling rate as a row of facets along the one
/// edge the eye is drawn to. The engine draws the exported mask the same way, so
/// what is dragged is what comes back.
///
/// Below a handful of samples it stays a polygon, matching the engine again: too
/// few points are a shape rather than a stroke, and a curve fitted through four
/// of them is an invention.
private func ringPath(_ points: [CGPoint]) -> Path {
    var path = Path()
    guard let first = points.first else { return path }

    if points.count < smoothedRingPoints {
        path.move(to: first)
        for point in points.dropFirst() { path.addLine(to: point) }
        path.closeSubpath()
        return path
    }

    func at(_ index: Int) -> CGPoint { points[index % points.count] }
    func midpoint(_ a: CGPoint, _ b: CGPoint) -> CGPoint {
        CGPoint(x: (a.x + b.x) / 2, y: (a.y + b.y) / 2)
    }

    // Started on the closing edge's midpoint so the ring joins itself mid-curve
    // rather than at a sample, which is where a corner would otherwise show.
    path.move(to: midpoint(at(points.count - 1), at(0)))
    for index in points.indices {
        let through = at(index)
        path.addQuadCurve(to: midpoint(through, at(index + 1)), control: through)
    }
    path.closeSubpath()
    return path
}

/// How far the page outside a capture selection is knocked back.
private let shadeAlpha: CGFloat = 0.45

/// Marquee border. A screen-space affordance, not part of the page.
private let borderWidth: CGFloat = 3

/// The closest two ring samples that are kept.
///
/// Small enough that a curve still reads as a curve, large enough that a slow
/// drag around a detail does not hand the engine thousands of points to fill a
/// path with.
private let ringStep: CGFloat = 3

/// How many samples make a ring a stroke rather than a stated shape. Matches the
/// engine's own figure, so the preview and the exported mask agree about when a
/// ring is smoothed.
private let smoothedRingPoints = 8
