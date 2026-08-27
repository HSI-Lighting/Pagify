import CoreGraphics
import Foundation

/// A page's text with a box for every character of it.
///
/// The foundation of selecting text, and the reason `TextSegment` is not: a run
/// is a whole line, so a selection built from runs can only begin and end at a
/// line. Dragging across half a sentence would copy both lines it touched, whole.
///
/// `boxes` holds four numbers per character of `text` — left, top, right, bottom,
/// in page points from the top-left — aligned to it by construction, since the
/// engine builds both from one walk of the page.
///
/// Indexed in UTF-16 code units, which is what the engine counts and what the
/// boxes are aligned to. Swift's `String` will not be indexed that way, so the
/// text is held as an `Array<UTF16.CodeUnit>` and turned back into a `String`
/// only when a selection is actually taken. Android gets this for free — Kotlin
/// indexes strings in UTF-16 — and doing it by hand here is what keeps a box and
/// its character the same character on a page with an emoji or a surrogate pair
/// on it.
struct PageCharacters {
    let units: [UTF16.CodeUnit]
    let boxes: [CGFloat]

    static let empty = PageCharacters(units: [], boxes: [])

    var count: Int { units.count }
    var isEmpty: Bool { count == 0 }

    func box(at index: Int) -> CGRect {
        let at = index * 4
        guard at + 3 < boxes.count else { return .zero }
        return CGRect(x: boxes[at], y: boxes[at + 1],
                      width: boxes[at + 2] - boxes[at],
                      height: boxes[at + 3] - boxes[at + 1])
    }

    /// The character nearest `point`, or nil on a page with no text.
    ///
    /// Vertical distance is weighted, for the same reason it is when hit-testing
    /// a run: a touch that misses the text almost always missed along the line it
    /// was aiming at, rather than meaning the line above or below.
    func indexNear(_ point: CGPoint) -> Int? {
        guard !isEmpty else { return nil }

        var best = -1
        var bestScore = CGFloat.greatestFiniteMagnitude
        for index in 0..<count {
            let box = self.box(at: index)
            let dy: CGFloat = point.y < box.minY ? box.minY - point.y
                            : point.y > box.maxY ? point.y - box.maxY : 0
            let dx: CGFloat = point.x < box.minX ? box.minX - point.x
                            : point.x > box.maxX ? point.x - box.maxX : 0
            let score = dy * Self.verticalWeight + dx
            if score < bestScore {
                bestScore = score
                best = index
            }
        }
        return best >= 0 ? best : nil
    }

    /// The word around `index`, as a closed range.
    ///
    /// What a long press should select. Landing on a single character is almost
    /// never what someone meant — they pointed at a word — and starting from the
    /// whole word means the handles are usually only nudged rather than dragged
    /// across the line.
    func wordAround(_ index: Int) -> ClosedRange<Int>? {
        guard !isEmpty else { return nil }
        let at = min(max(index, 0), count - 1)
        if isWhitespace(at) { return at...at }

        var start = at
        while start > 0, !isWhitespace(start - 1) { start -= 1 }
        var end = at
        while end < count - 1, !isWhitespace(end + 1) { end += 1 }
        return start...end
    }

    /// The text of a selection, exactly as the page holds it.
    func text(of range: ClosedRange<Int>) -> String {
        guard !isEmpty else { return "" }
        let from = min(max(range.lowerBound, 0), count - 1)
        let to = min(max(range.upperBound, from), count - 1)
        return String(decoding: units[from...to], as: UTF16.self)
    }

    /// The rectangles to paint over a selection: one per line, not one per
    /// character.
    ///
    /// Consecutive characters that share a line are merged, which is what makes a
    /// selection look like a band over the text rather than a row of separate
    /// boxes — and it is also far less to draw on a page where a selection can
    /// run to thousands of characters.
    ///
    /// A gap in the middle of a line is kept as a gap. Two columns of a table can
    /// sit on one line with empty space between them, and painting across that
    /// space would claim text that is not selected.
    func rects(of range: ClosedRange<Int>) -> [CGRect] {
        guard !isEmpty else { return [] }
        let from = min(max(range.lowerBound, 0), count - 1)
        let to = min(max(range.upperBound, from), count - 1)

        var rects: [CGRect] = []
        var current: CGRect?

        for index in from...to {
            let unit = units[index]
            if unit == 0x000A || unit == 0x000D { continue }
            let box = self.box(at: index)
            if box.width <= 0, box.height <= 0 { continue }

            if let previous = current, joins(box, previous) {
                current = previous.union(box)
            } else {
                if let previous = current { rects.append(previous) }
                current = box
            }
        }
        if let previous = current { rects.append(previous) }
        return rects
    }

    /// Whether a character continues the band being built.
    ///
    /// Two tests, and both are needed. The vertical one catches a new line; the
    /// horizontal one catches a jump across a page — the next column, or the far
    /// side of a table — which sits on the same line and must not be painted
    /// through.
    private func joins(_ box: CGRect, _ previous: CGRect) -> Bool {
        let sameLine = abs(box.minY - previous.minY) <= previous.height * Self.lineTolerance
        let adjacent = box.minX <= previous.maxX + previous.height * Self.gapTolerance
            && box.maxX >= previous.minX - previous.height * Self.gapTolerance
        return sameLine && adjacent
    }

    private func isWhitespace(_ index: Int) -> Bool {
        guard let scalar = Unicode.Scalar(units[index]) else { return false }
        return CharacterSet.whitespacesAndNewlines.contains(scalar)
    }

    /// How much more a touch missing vertically counts than missing sideways.
    private static let verticalWeight: CGFloat = 4
    /// How far two characters' tops may differ and still be one line.
    private static let lineTolerance: CGFloat = 0.5
    /// How wide a gap may be, as a multiple of the line height, before it is
    /// treated as a jump rather than a space.
    ///
    /// A space is well under one line height; the gutter between two columns is
    /// several.
    private static let gapTolerance: CGFloat = 1.5
}

func decodePageCharacters(_ json: String) -> PageCharacters {
    guard let data = json.data(using: .utf8),
          let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    else { return .empty }

    let text = root["text"] as? String ?? ""
    let boxes = (root["boxes"] as? [Double] ?? []).map { CGFloat($0) }
    return PageCharacters(units: Array(text.utf16), boxes: boxes)
}

/// One selection, and the page it belongs to.
///
/// Selecting across a page break is not offered: the text of two pages is two
/// walks of two different page objects, and a range that spanned them would have
/// no single set of boxes to paint.
struct PageTextSelection: Equatable {
    let pageIndex: Int
    let range: ClosedRange<Int>
    let rects: [CGRect]
    let text: String

    /// Where the handles go: the start of the first band and the end of the last.
    var startHandle: CGPoint { rects.first.map { CGPoint(x: $0.minX, y: $0.maxY) } ?? .zero }
    var endHandle: CGPoint { rects.last.map { CGPoint(x: $0.maxX, y: $0.maxY) } ?? .zero }
}
