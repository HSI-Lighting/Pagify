import CoreGraphics
import Foundation

// Where every letter of a caption goes. Android's `core/TextLayout.kt`.
//
// In the app's space — page points from the top left, y downwards, angles
// clockwise. The flip to PDF's bottom-left convention happens once, at the
// PDFium boundary, exactly as it does for every other mark.

/// One placed glyph: what it is, where it goes, and which way it leans.
struct GlyphPlacement {
    /// The character as a string: one glyph, but not always one `Character`.
    let text: String
    /// The glyph's id in an embedded font, from the shaper. Zero where there is
    /// none — which is both "no embedded font" and glyph 0, the notdef box.
    var id: UInt32 = 0
    let origin: CGPoint
    let radians: CGFloat
}

/// What is drawn around the words, if anything.
enum TextFrame: String, CaseIterable, Identifiable, Codable {
    case none = "None"
    /// The drawing-office revision cloud, scalloped outward.
    case cloud = "Cloud"
    case box = "Box"
    /// An ellipse wide enough that the words sit inside it, not across it.
    case ellipse = "Ellipse"

    var id: String { rawValue }
}

/// Cap height and descender as fractions of the point size. This decides where a
/// box sits around a line of type, not where a glyph goes, so the standard-14
/// faces differing by a percent or two does not matter.
private let capHeight: CGFloat = 0.72
private let descender: CGFloat = 0.21
/// The margin round the words, per point of type.
let cloudTextMarginFraction: CGFloat = 0.45
/// How thick a stroke the cloud's bumps are sized as, per point of type.
private let cloudTextBump: CGFloat = 0.17
/// How far past half the box the ellipse reaches. Root two puts the corners
/// exactly *on* the curve, and a curve drawn as 64 chords cuts inside itself
/// between samples — so the extra few percent is what makes the drawn ring
/// actually enclose the drawn words.
private let ellipseReach: CGFloat = 1.47
private let textEllipseSegments = 64
/// How far apart the lines sit, per point of type. Ordinary leading.
private let lineHeight: CGFloat = 1.25
/// How thick the ring around a caption is drawn, per point of type.
let textFrameStroke: CGFloat = 0.08

/// Every glyph of one line, along the baseline it sits on.
///
/// A bundled font is walked as **glyphs**, not characters. In most of the world's
/// scripts those are not the same list: Arabic letters join into forms that have
/// no character of their own, Devanagari reorders, and a right-to-left line comes
/// back from the shaper already in the order it is drawn. Laying out character by
/// character is what made Persian come out as a row of isolated letters running
/// backwards.
func layOutText(_ text: String, font: PagifyFont, size: CGFloat,
                path: [CGPoint]) -> [GlyphPlacement] {
    guard !text.isEmpty, path.count >= 2 else { return [] }

    var placements: [GlyphPlacement] = []
    var travelled: CGFloat = 0

    if let shaped = PagifyFonts.shape(font, text), !shaped.glyphs.isEmpty {
        for glyph in shaped.glyphs {
            guard let at = pointAlong(path, distance: travelled) else { return placements }
            placements.append(GlyphPlacement(
                text: glyph.text,
                id: glyph.id,
                origin: CGPoint(x: at.point.x + glyph.offsetX * size,
                                y: at.point.y - glyph.offsetY * size),
                radians: at.radians))
            travelled += glyph.advance * size
        }
        return placements
    }

    for character in text {
        guard let at = pointAlong(path, distance: travelled) else { return placements }
        placements.append(GlyphPlacement(text: String(character),
                                         origin: at.point, radians: at.radians))
        travelled += font.advance(of: character, size: size)
    }
    return placements
}

/// The point `distance` along the path, and the direction the path runs there.
///
/// Nil once the path runs out, which is what stops text overflowing the line it
/// was drawn on.
private func pointAlong(_ path: [CGPoint], distance: CGFloat) -> (point: CGPoint, radians: CGFloat)? {
    guard distance >= 0 else { return nil }
    var travelled: CGFloat = 0

    for (from, to) in zip(path, path.dropFirst()) {
        let span = CGPoint(x: to.x - from.x, y: to.y - from.y)
        let length = hypot(span.x, span.y)
        guard length > 0 else { continue }

        if travelled + length >= distance {
            let along = (distance - travelled) / length
            return (CGPoint(x: from.x + span.x * along, y: from.y + span.y * along),
                    atan2(span.y, span.x))
        }
        travelled += length
    }
    return nil
}

/// How far along the baseline the text reaches.
func baselineLength(_ path: [CGPoint]) -> CGFloat {
    zip(path, path.dropFirst()).reduce(0) { $0 + hypot($1.1.x - $1.0.x, $1.1.y - $1.0.y) }
}

/// A baseline for text placed by a single tap, running to the right of it.
///
/// Long enough for the text and no longer. The baseline is what the layout walks,
/// so it has to exist even when nobody drew one.
func straightBaseline(anchor: CGPoint, text: String, font: PagifyFont,
                      size: CGFloat) -> [CGPoint] {
    [anchor, CGPoint(x: anchor.x + font.width(of: text, size: size), y: anchor.y)]
}

/// A baseline that bends, for text placed by a single tap.
///
/// The reader no longer draws the curve: they say how much bend they want and the
/// app makes the arc. Drawing it by hand looked broken for a reason that could not
/// be fixed by drawing more carefully — a short caption covers only the first part
/// of a long stroke, and the first part of any hand-drawn arc is its straightest.
///
/// `degrees` is how far the line turns from end to end: 0 is straight, positive
/// arches upward, negative sags. The arc is exactly as long as the words, so every
/// letter lands on it however far it bends.
/// A page point turned about `centre`.
///
/// In the app's own space — y downward, so a positive angle turns clockwise on
/// screen, the same sense as every other angle in this file.
func turning(_ point: CGPoint, about centre: CGPoint, by radians: CGFloat) -> CGPoint {
    guard radians != 0 else { return point }
    let sine = sin(radians), cosine = cos(radians)
    let dx = point.x - centre.x, dy = point.y - centre.y
    return CGPoint(x: centre.x + dx * cosine - dy * sine,
                   y: centre.y + dx * sine + dy * cosine)
}

func curvedBaseline(anchor: CGPoint, text: String, font: PagifyFont,
                    size: CGFloat, degrees: CGFloat) -> [CGPoint] {
    let width = font.width(of: text, size: size)
    let turn = degrees * .pi / 180
    // Straight enough that an arc would only add rounding error.
    guard width > 0, abs(turn) >= 0.02 else {
        return straightBaseline(anchor: anchor, text: text, font: font, size: size)
    }

    let radius = width / turn
    let start = -turn / 2
    // About three degrees a segment, so no flat spot shows at any size.
    let steps = min(max(Int(abs(turn) / 0.05), 2), 128)

    return (0...steps).map { step in
        let along = start + turn * CGFloat(step) / CGFloat(steps)
        return CGPoint(x: anchor.x + radius * (sin(along) - sin(start)),
                       y: anchor.y + radius * (cos(start) - cos(along)))
    }
}

/// The largest point size at which this text still fits across `availableWidth`.
///
/// One division rather than a search: the run is linear in the point size, so the
/// width at one point is all that has to be measured. The widest line decides it,
/// not the whole string as one run — a two-line caption measured end to end comes
/// out half the size it should be.
///
/// Shared by the reader and the capture editor, which cap a caption against
/// different things — a page and a picture — by the same arithmetic.
func sizeThatFits(_ text: String, font: PagifyFont, availableWidth: CGFloat,
                  lines: Int = 1) -> CGFloat {
    let ceiling = AnnotationMetrics.textRange.upperBound
    guard !text.isEmpty, availableWidth > 0 else { return ceiling }
    let atOnePoint = captionLines(text, into: lines).map { font.width(of: $0, size: 1) }.max() ?? 0
    guard atOnePoint > 0 else { return ceiling }
    return min(max(availableWidth / atOnePoint, AnnotationMetrics.textRange.lowerBound), ceiling)
}

func captionLines(_ text: String) -> [String] {
    text.components(separatedBy: "\n")
}

/// The caption's words, broken into `lines` of roughly even length.
///
/// A caption is laid out along a baseline, so without this a paragraph is one
/// baseline — a single line of type running off the sheet and out of the page,
/// which is what a pasted paragraph looked like. Nothing in the text says where
/// it should break: the newlines a page's own text layer reports are about how
/// that page was typeset, not about how these words should sit here, and a
/// paragraph copied off a page usually has none at all.
///
/// So the reader says how many lines they want, and the words are dealt out to
/// fill them evenly. Balanced by character count rather than by measured width:
/// the difference is a few points on a ragged edge, and this runs on every frame
/// of a drag.
///
/// Explicit newlines still win where there are any — someone who typed their own
/// break meant it — and each of those paragraphs is then wrapped in turn.
func captionLines(_ text: String, into lines: Int) -> [String] {
    let paragraphs = captionLines(text)
    guard lines > 1 else { return paragraphs }

    // Shared out between the paragraphs by length, so a long one gets the room.
    let total = max(paragraphs.reduce(0) { $0 + $1.count }, 1)
    var out: [String] = []
    for (index, paragraph) in paragraphs.enumerated() {
        let share = index == paragraphs.count - 1
            ? max(1, lines - out.count)
            : max(1, Int((Double(paragraph.count) / Double(total) * Double(lines)).rounded()))
        out += wrap(paragraph, into: min(share, max(1, lines - out.count)))
        if out.count >= lines { break }
    }
    return out
}

/// One paragraph, dealt into `lines` lines on word boundaries.
private func wrap(_ paragraph: String, into lines: Int) -> [String] {
    let words = paragraph.split(separator: " ", omittingEmptySubsequences: true).map(String.init)
    guard lines > 1, words.count > 1 else { return [paragraph] }

    // The length each line is aiming for, spaces included.
    let target = Double(paragraph.count) / Double(lines)
    var out: [String] = []
    var current = ""
    for word in words {
        let remaining = lines - out.count
        // The last line takes whatever is left, however long: breaking early
        // would leave a word stranded past the end of the caption.
        if remaining <= 1 {
            current = current.isEmpty ? word : current + " " + word
            continue
        }
        let grown = current.isEmpty ? word : current + " " + word
        // Taken if it gets closer to the target than stopping short does — which
        // is what keeps the last word of a line from always falling to the next.
        if current.isEmpty || abs(Double(grown.count) - target) <= abs(Double(current.count) - target) {
            current = grown
        } else {
            out.append(current)
            current = word
        }
    }
    if !current.isEmpty { out.append(current) }
    return out
}

/// A caption, as the app holds it before it becomes glyphs.
///
/// Held as the string and the baseline it sits on rather than as the glyphs it
/// will become, because it is still text right up until it is saved: the size can
/// change, the font can change, and none of that should mean re-deriving shapes.
struct TextMark: Equatable {
    var text: String
    /// The baseline, in page points. Two points for straight text.
    var path: [CGPoint]
    var font: PagifyFont
    var size: CGFloat
    var color: MarkColor
    var frame: TextFrame = .none
    /// How far the baseline turns from end to end, in degrees.
    var curveDegrees: CGFloat = 0
    /// How many lines the words are broken into.
    ///
    /// One means "however many newlines were typed", which is what a caption has
    /// always been. Anything more wraps the words to fill that many lines — a
    /// paragraph pasted in from a page has no newlines at all, and without this
    /// it became a single line of type running clean off the sheet.
    var lines: Int = 1
    /// How far the whole caption is turned on the page, clockwise, in radians.
    ///
    /// Not part of the baseline, and deliberately so. The baseline stays in the
    /// caption's own upright frame — it is what the block stacks its lines down,
    /// what the bounds measure, and what the bend bends — and the turn is applied
    /// once, at the end, to whatever those produced.
    ///
    /// Folding it into `path` instead would work for a single straight line and
    /// nothing else: a block's second line is built from the anchor with an
    /// axis-aligned drop, so a tilted path would stack its lines vertically
    /// anyway, and every box in this file is an upright rectangle. Kept out of
    /// the path it also costs nothing to change — turning a caption never
    /// re-measures its line, so it cannot lose the letters off the end of it.
    var rotationRadians: CGFloat = 0
    var id: Int32 = 0

    var isMultiLine: Bool { text.contains("\n") }

    /// The same caption with one thing changed, and the baseline rebuilt for it.
    ///
    /// Never adjusted in place: the run is a different width in a different face
    /// at a different size, and a baseline that no longer matches silently drops
    /// the glyphs that run off its end. Only what is named changes — the rest is
    /// carried across, so resizing a caption cannot also restyle it.
    func rebuilt(text: String? = nil, font: PagifyFont? = nil,
                 size: CGFloat? = nil, curveDegrees: CGFloat? = nil,
                 rotationRadians: CGFloat? = nil,
                 color: MarkColor? = nil, frame: TextFrame? = nil) -> TextMark {
        let words = text ?? self.text
        let face = font ?? self.font
        let requested = curveDegrees ?? self.curveDegrees
        let points = min(max(size ?? self.size, AnnotationMetrics.textRange.lowerBound),
                         AnnotationMetrics.textRange.upperBound)

        // A block never bends: stacked arcs curl into each other and there is no
        // answer for where the second one sits. The requested bend is still
        // stored, so losing the extra line brings it back.
        let bend = words.contains("\n") ? 0 : requested

        var next = self
        next.text = words
        next.font = face
        next.size = points
        next.color = color ?? self.color
        next.frame = frame ?? self.frame
        next.curveDegrees = requested
        // Carried, never rebuilt from: a turn does not change the width of the
        // run, so the line the layout walks is the same line.
        next.rotationRadians = rotationRadians ?? self.rotationRadians
        next.path = curvedBaseline(anchor: path.first ?? .zero,
                                   text: captionLines(words).first ?? "",
                                   font: face, size: points, degrees: bend)
        return next
    }

    /// Every glyph of a caption, over however many lines it has.
    ///
    /// One line goes along the stored baseline, bent or straight. More than one is
    /// laid as a block: each line on its own baseline, one line height below the
    /// last, every line centred on the block — centred because a multi-line
    /// caption is usually inside a cloud or a box, and a frame drawn round
    /// ragged-left lines reads as a mistake.
    ///
    /// The bend is not applied to a block. Stacked arcs curl into each other as
    /// the bend grows, and there is no answer for where the second arc should sit.
    func layOutBlock() -> [GlyphPlacement] {
        let upright = layOutUpright()
        guard rotationRadians != 0 else { return upright }
        // Turned as one about the block's own centre, after the words are laid
        // out — never before. Turning the *baseline* first and laying out along
        // it would tilt one straight line correctly and get everything else
        // wrong: a block's lines are stacked with an axis-aligned drop.
        //
        // Each glyph's own lean is added to rather than replaced, so a bent
        // caption that is also turned keeps its bend and takes the turn on top.
        let centre = uprightCentre
        return upright.map {
            GlyphPlacement(text: $0.text, id: $0.id,
                           origin: turning($0.origin, about: centre, by: rotationRadians),
                           radians: $0.radians + rotationRadians)
        }
    }

    /// The block's centre before it is turned — the pivot everything turns about.
    var uprightCentre: CGPoint {
        let box = uprightBounds()
        return CGPoint(x: (box.left + box.right) / 2, y: (box.top + box.bottom) / 2)
    }

    /// The same as `layOutBlock`, before the turn. The layout proper: everything
    /// else here measures against this.
    private func layOutUpright() -> [GlyphPlacement] {
        let lines = captionLines(text, into: self.lines)
        if lines.count <= 1 {
            return layOutText(text, font: font, size: size, path: path)
        }
        guard let anchor = path.first else { return [] }

        let widest = lines.map { font.width(of: $0, size: size) }.max() ?? 0
        let leading = size * lineHeight

        return lines.enumerated().flatMap { index, line -> [GlyphPlacement] in
            let width = font.width(of: line, size: size)
            let start = CGPoint(x: anchor.x + (widest - width) / 2,
                                y: anchor.y + CGFloat(index) * leading)
            return layOutText(line, font: font, size: size,
                              path: straightBaseline(anchor: start, text: line,
                                                     font: font, size: size))
        }
    }

    /// The box a caption occupies, before any margin. The widest line decides the
    /// width and the number of lines the height, which is what makes a frame fit a
    /// block rather than only its first line.
    func textBlockBounds() -> PageRect {
        // The upright box, still. A turned caption's true outline is not a
        // rectangle, and everything that asks for this — clamping a drag to the
        // page, sizing a frame, hit-testing a tap — wants the block's own extent
        // rather than the axis-aligned hull of a tilted one. The ring is turned
        // where it is drawn, in `textFrameOutline`, which is the one place the
        // shape has to sit on the page.
        uprightBounds()
    }

    /// The block's extent in its own upright frame.
    func uprightBounds() -> PageRect {
        let anchor = path.first ?? .zero
        let lines = captionLines(text, into: self.lines)
        let widest = lines.map { font.width(of: $0, size: size) }.max() ?? 0
        let leading = size * lineHeight
        return PageRect(left: anchor.x,
                        top: anchor.y - size * capHeight,
                        right: anchor.x + widest,
                        bottom: anchor.y + size * descender + CGFloat(lines.count - 1) * leading)
    }

    func textFrameBounds() -> PageRect {
        let margin = size * cloudTextMarginFraction
        let box = textBlockBounds()
        return PageRect(left: box.left - margin, top: box.top - margin,
                        right: box.right + margin, bottom: box.bottom + margin)
    }

    /// The ring drawn around a framed mark, as one closed polyline.
    ///
    /// The cloud comes through the same `cloudOutline` the cloud tool uses, so a
    /// cloud round words and a cloud drawn by hand are the same notation.
    func textFrameOutline() -> [CGPoint] {
        let box = textFrameBounds()
        let upright = [CGPoint(x: box.left, y: box.top),
                       CGPoint(x: box.right, y: box.top),
                       CGPoint(x: box.right, y: box.bottom),
                       CGPoint(x: box.left, y: box.bottom)]
        // Turned about the same pivot the words are, so the ring stays around
        // them. Turned *before* the cloud is built rather than after: a cloud is
        // a run of arcs along each edge, and turning the finished arcs is the
        // same shape, but turning the corners first keeps the bumps square to
        // the edge they belong to.
        let corners = rotationRadians == 0
            ? upright
            : upright.map { turning($0, about: uprightCentre, by: rotationRadians) }
        switch frame {
        case .none: return []
        case .cloud: return cloudOutline(corners, width: size * cloudTextBump)
        case .box: return corners + [corners[0]]
        case .ellipse: return ellipseThrough(box)
        }
    }
}

/// An ellipse the size of the box's circumscribing one, as a closed polyline.
private func ellipseThrough(_ box: PageRect) -> [CGPoint] {
    let cx = (box.left + box.right) / 2
    let cy = (box.top + box.bottom) / 2
    let rx = (box.right - box.left) / 2 * ellipseReach
    let ry = (box.bottom - box.top) / 2 * ellipseReach
    return (0...textEllipseSegments).map { step in
        let angle = CGFloat(step) / CGFloat(textEllipseSegments) * 2 * .pi
        return CGPoint(x: cx + cos(angle) * rx, y: cy + sin(angle) * ry)
    }
}


extension TextMark {
    /// Rebuild a caption from the blob stored beside it.
    ///
    /// Nil for anything it cannot read, rather than a half-built mark: a caption
    /// that comes back with the wrong words in the wrong place is worse than one
    /// that stays part of the page.
    init?(restoreJSON json: String) {
        guard let data = json.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let points = o["path"] as? [[String: Any]], !points.isEmpty,
              let text = o["text"] as? String,
              let size = o["size"] as? CGFloat,
              let argb = o["argb"] as? Int else {
            return nil
        }

        let path = points.map {
            CGPoint(x: $0["x"] as? CGFloat ?? 0, y: $0["y"] as? CGFloat ?? 0)
        }
        let wire = o["font"] as? String ?? PagifyFont.helvetica.wireName
        let asset = o["fontAsset"] as? String
        let face = PagifyFont.allCases.first {
            asset != nil ? $0.asset == asset : ($0.asset == nil && $0.wireName == wire)
        } ?? .helvetica

        self.init(text: text,
                  path: path,
                  font: face,
                  size: size,
                  color: MarkColor(argb: UInt32(bitPattern: Int32(truncatingIfNeeded: argb))),
                  frame: TextFrame(rawValue: o["textFrame"] as? String ?? "None") ?? .none,
                  curveDegrees: 0,
                  // Kept so reopening a caption offers the count it was written
                  // with, rather than resetting a wrapped paragraph to one line
                  // the first time anyone edits it.
                  lines: max(1, o["lines"] as? Int ?? 1),
                  rotationRadians: CGFloat(o["rotationRadians"] as? Double ?? 0),
                  id: Int32(truncatingIfNeeded: o["id"] as? Int ?? 0))
    }
}
