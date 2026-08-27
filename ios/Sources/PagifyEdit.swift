import CoreGraphics
import Foundation

// The editing wire format, mirroring `core/PdfEdit.kt` on Android and the
// `Command`/`Annotation` types in `rust/pdf_core/src/command` and
// `src/document`.
//
// **Coordinates cross unchanged.** Both sides measure page points from the
// top-left with y increasing downwards; the engine flips to PDF's bottom-left
// convention once, at the PDFium boundary. Flipping here as well would put every
// saved mark on the wrong half of its page — and it would still look right on
// screen, because the app would flip it back on the way in.

/// A rectangle in page points, top-left origin.
struct PageRect: Equatable {
    var left: CGFloat
    var top: CGFloat
    var right: CGFloat
    var bottom: CGFloat

    init(left: CGFloat, top: CGFloat, right: CGFloat, bottom: CGFloat) {
        self.left = left
        self.top = top
        self.right = right
        self.bottom = bottom
    }

    /// Normalised, so a rectangle dragged up-and-left is the same as one dragged
    /// down-and-right.
    init(from: CGPoint, to: CGPoint) {
        self.left = min(from.x, to.x)
        self.top = min(from.y, to.y)
        self.right = max(from.x, to.x)
        self.bottom = max(from.y, to.y)
    }

    var cgRect: CGRect {
        CGRect(x: left, y: top, width: right - left, height: bottom - top)
    }

    var json: [String: Any] {
        ["left": left, "top": top, "right": right, "bottom": bottom]
    }
}

/// A colour as the engine reads it: separate 0–255 channels.
///
/// Alpha is carried rather than dropped — a highlight is translucent, and writing
/// it opaque would black out the words underneath it.
struct MarkColor: Equatable {
    var r: Int
    var g: Int
    var b: Int
    var a: Int

    init(r: Int, g: Int, b: Int, a: Int = 255) {
        self.r = r; self.g = g; self.b = b; self.a = a
    }

    /// From the packed `0xAARRGGBB` the app stores colours as, matching Android.
    init(argb: UInt32) {
        self.a = Int((argb >> 24) & 0xFF)
        self.r = Int((argb >> 16) & 0xFF)
        self.g = Int((argb >> 8) & 0xFF)
        self.b = Int(argb & 0xFF)
    }

    var argb: UInt32 {
        (UInt32(a) << 24) | (UInt32(r) << 16) | (UInt32(g) << 8) | UInt32(b)
    }

    var json: [String: Any] { ["r": r, "g": g, "b": b, "a": a] }

    var cgColor: CGColor {
        CGColor(srgbRed: CGFloat(r) / 255, green: CGFloat(g) / 255,
                blue: CGFloat(b) / 255, alpha: CGFloat(a) / 255)
    }

    func withAlpha(_ alpha: Int) -> MarkColor {
        MarkColor(r: r, g: g, b: b, a: alpha)
    }
}

/// A mark in the engine's wire form, without the command wrapper.
///
/// Note there is no `shape` case, and that is not an omission: a line, an arrow,
/// a box, a circle and a cloud are all saved as **ink** — a set of polylines,
/// which is what a signature already is and what every viewer draws correctly.
/// The line type is baked into the strokes when the shape is committed, because a
/// dash array on an ink annotation is a thing most viewers ignore. What is drawn
/// is what is in the file.
enum WireAnnotation {
    case highlight(rects: [PageRect], color: MarkColor)
    case ink(strokes: [[CGPoint]], color: MarkColor, width: CGFloat)
    case note(rect: PageRect, contents: String, color: MarkColor)
    /// Words written onto the page — the one case here that is **not** an
    /// annotation. The others are marks laid over a page; this becomes page
    /// content, real text objects, so a reader can select it, search it and copy
    /// it out. That is the whole reason for writing text rather than drawing
    /// letters.
    case text(TextMark)

    var json: [String: Any] {
        switch self {
        case .highlight(let rects, let color):
            return ["kind": "highlight",
                    "rects": rects.map(\.json),
                    "color": color.json]
        case .ink(let strokes, let color, let width):
            return ["kind": "ink",
                    "strokes": strokes.map { $0.map { ["x": $0.x, "y": $0.y] } },
                    "color": color.json,
                    "width": width]
        case .note(let rect, let contents, let color):
            return ["kind": "note",
                    "rect": rect.json,
                    "contents": contents,
                    "color": color.json]

        case .text(let mark):
            return mark.wireJSON(withRestore: true)
        }
    }
}

extension TextMark {
    /// The caption in the engine's wire form.
    ///
    /// The glyphs arrive already placed. The app walks the baseline with the
    /// font's own metrics and so does its preview; only one side can be the
    /// authority on where a letter sits, and it has to be the side the person was
    /// looking at when they put it there.
    func wireJSON(withRestore: Bool) -> [String: Any] {
        var body: [String: Any] = [
            "kind": "text",
            "text": text,
            "font": font.wireName,
            "size": size,
            "color": color.json,
            "id": Int(id),
            "glyphs": layOutBlock().map { glyph in
                ["ch": glyph.text, "id": Int(glyph.id),
                 "x": glyph.origin.x, "y": glyph.origin.y, "radians": glyph.radians]
            },
            // The ring goes *with* the words rather than beside them as its own
            // mark. Written separately it was separate once the file was
            // reopened, and the eraser took the ring off a clouded caption and
            // left the words in place.
            "frame": textFrameOutline().map { ["x": $0.x, "y": $0.y] },
            "frameWidth": size * textFrameStroke,
        ]

        // The file to embed, when this font is one. Absent for a standard-14,
        // which is named rather than embedded and written by character.
        if let asset = font.asset { body["fontAsset"] = asset }

        // The app's half. The engine ignores fields it does not know, which is
        // what lets one object be both the instruction to write and the record of
        // what was written.
        body["argb"] = Int(color.argb)
        body["textFrame"] = frame.rawValue
        body["path"] = path.map { ["x": $0.x, "y": $0.y] }
        body["fontId"] = font.rawValue
        body["curveDegrees"] = curveDegrees

        // Stored beside the words and handed back untouched. It is what makes a
        // saved caption a mark again rather than part of the page, and what lets
        // erasing one be undone.
        if withRestore,
           let data = try? JSONSerialization.data(withJSONObject: wireJSON(withRestore: false)),
           let encoded = String(data: data, encoding: .utf8) {
            body["restore"] = encoded
        }
        return body
    }
}

/// Every mutation of a document, as a value.
///
/// One rule governs the write path: a document is only ever changed by running a
/// command. That is what makes undo, redo and (later) scripting fall out rather
/// than having to be retrofitted onto every operation — and it is why adding a
/// tool needs no new FFI entry point.
enum PagifyCommand {
    /// `order[i]` is the index the page currently at `i` moves to.
    case reorderPages(order: [Int])
    case deletePage(index: Int)
    case insertBlankPage(at: Int, widthPt: CGFloat, heightPt: CGFloat,
                         fill: MarkColor?, ruling: Int)
    case setPageRotation(index: Int, quarterTurns: Int)
    case addAnnotation(pageIndex: Int, annotation: WireAnnotation)
    /// `index` is **PDFium's** index for the mark, not a position in any list the
    /// app holds. A page can carry form widgets and links this engine does not
    /// model; addressing by list position would delete somebody's form field.
    case removeAnnotation(pageIndex: Int, index: Int)
    /// Text is page content and has no annotation index, so it goes by the app's
    /// own id — which is tagged onto every object the write put there.
    case removeText(pageIndex: Int, id: Int)

    /// The one page this changes, when it changes only one.
    ///
    /// `nil` means every page has to be treated as stale — reordering or deleting
    /// renumbers the rest, so no cached raster keyed by index can be trusted.
    var affectedPage: Int? {
        switch self {
        case .reorderPages, .deletePage, .insertBlankPage: return nil
        case .setPageRotation(let index, _): return index
        case .addAnnotation(let pageIndex, _): return pageIndex
        case .removeAnnotation(let pageIndex, _): return pageIndex
        case .removeText(let pageIndex, _): return pageIndex
        }
    }


    var json: [String: Any] {
        switch self {
        case .reorderPages(let order):
            return ["op": "reorderPages", "order": order]
        case .deletePage(let index):
            return ["op": "deletePage", "index": index]
        case .insertBlankPage(let at, let w, let h, let fill, let ruling):
            var body: [String: Any] = ["op": "insertBlankPage", "at": at,
                                       "widthPt": w, "heightPt": h, "ruling": ruling]
            if let fill { body["fill"] = fill.json }
            return body
        case .setPageRotation(let index, let quarterTurns):
            return ["op": "setPageRotation", "index": index, "quarterTurns": quarterTurns]
        case .addAnnotation(let pageIndex, let annotation):
            // The mark is a *nested* object, not merged into the command.
            // Merging produces `{"op":"addAnnotation","kind":"highlight",…}`,
            // which the engine rejects as `missing field 'annotation'`.
            return ["op": "addAnnotation", "pageIndex": pageIndex,
                    "annotation": annotation.json]
        case .removeAnnotation(let pageIndex, let index):
            return ["op": "removeAnnotation", "pageIndex": pageIndex, "index": index]
        case .removeText(let pageIndex, let id):
            return ["op": "removeText", "pageIndex": pageIndex, "id": id]
        }
    }

    func encoded() throws -> String {
        let data = try JSONSerialization.data(withJSONObject: json, options: [])
        guard let text = String(data: data, encoding: .utf8) else {
            throw PagifyError.engine("could not encode the command")
        }
        return text
    }
}

/// Everything the UI needs to draw its editing controls, fetched as one value —
/// asking for the parts separately would let the UI paint a state the document
/// was never actually in.
struct EditState: Equatable {
    var pageCount: Int = 0
    var canUndo: Bool = false
    var canRedo: Bool = false
    /// Already phrased as a user action ("Delete page 5"), so a button can be
    /// labelled with no lookup table.
    var undoLabel: String?
    var redoLabel: String?
    /// Whether a save would have anything to write.
    var dirty: Bool = false
    /// False for a document that cannot be edited at all, so the UI can hide the
    /// controls rather than offer them and then fail.
    var editable: Bool = false

    init() {}

    init(json: String) {
        guard let data = json.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            return
        }
        pageCount = o["pageCount"] as? Int ?? 0
        canUndo = o["canUndo"] as? Bool ?? false
        canRedo = o["canRedo"] as? Bool ?? false
        undoLabel = o["undoLabel"] as? String
        redoLabel = o["redoLabel"] as? String
        dirty = o["dirty"] as? Bool ?? false
        editable = o["editable"] as? Bool ?? false
    }
}

/// One of the app's marks, and how the engine addresses it.
///
/// Text is the case that forces this to be two things. A caption is page
/// *content*, not an annotation — it never appears in `getAnnotationsJson` and
/// has no annotation index — so it is addressed by the app's own id through
/// `removeText`. Everything else is addressed by PDFium's index.
struct MarkRecord: Identifiable {
    let annotation: WireAnnotation
    /// PDFium's index for it, or nil for a caption.
    var engineIndex: Int?
    let id = UUID()

    /// The command that takes this mark off the page.
    func removal(page: Int) -> PagifyCommand? {
        if case .text(let mark) = annotation {
            return .removeText(pageIndex: page, id: Int(mark.id))
        }
        guard let engineIndex else { return nil }
        return .removeAnnotation(pageIndex: page, index: engineIndex)
    }
}

/// A mark already in the document, with the index the engine addresses it by.
struct PlacedAnnotation: Identifiable {
    let index: Int
    let annotation: WireAnnotation
    var id: Int { index }

    /// The box the mark occupies, for eraser hit-testing.
    var bounds: CGRect? {
        switch annotation {
        case .highlight(let rects, _):
            return rects.map(\.cgRect).reduce(nil) { $0?.union($1) ?? $1 }
        case .ink(let strokes, _, let width):
            let points = strokes.flatMap { $0 }
            guard let first = points.first else { return nil }
            var box = CGRect(origin: first, size: .zero)
            for p in points.dropFirst() { box = box.union(CGRect(origin: p, size: .zero)) }
            return box.insetBy(dx: -width, dy: -width)
        case .note(let rect, _, _):
            return rect.cgRect

        case .text(let mark):
            // The frame's box when there is one, so rubbing out a clouded caption
            // works by touching the ring as well as the words.
            return mark.frame == .none ? mark.textBlockBounds().cgRect
                                       : mark.textFrameBounds().cgRect
        }
    }
}

/// Decode the marks already on a page, as `getAnnotationsJson` returns them.
///
/// Marks this app does not model are skipped rather than guessed at — and the
/// engine's own index is carried through, because it is not the position in this
/// list.
func placedAnnotations(fromJSON json: String) -> [PlacedAnnotation] {
    guard let data = json.data(using: .utf8),
          let items = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] else {
        return []
    }

    return items.compactMap { item -> PlacedAnnotation? in
        guard let index = item["index"] as? Int,
              let kind = item["kind"] as? String else { return nil }

        func colour(_ key: String) -> MarkColor {
            // Absent means "we were not told", not "black". A mark that arrives
            // without a colour object used to black out the page it was on.
            guard let c = item[key] as? [String: Any] else { return AnnotationColors.yellow }
            return MarkColor(r: c["r"] as? Int ?? 255, g: c["g"] as? Int ?? 214,
                             b: c["b"] as? Int ?? 0, a: c["a"] as? Int ?? 255)
        }
        func rect(_ o: [String: Any]) -> PageRect {
            PageRect(left: o["left"] as? CGFloat ?? 0, top: o["top"] as? CGFloat ?? 0,
                     right: o["right"] as? CGFloat ?? 0, bottom: o["bottom"] as? CGFloat ?? 0)
        }

        switch kind {
        case "highlight":
            let rects = (item["rects"] as? [[String: Any]] ?? []).map(rect)
            return PlacedAnnotation(index: index, annotation: .highlight(rects: rects, color: colour("color")))
        case "ink":
            let strokes = (item["strokes"] as? [[[String: Any]]] ?? []).map { stroke in
                stroke.map { CGPoint(x: $0["x"] as? CGFloat ?? 0, y: $0["y"] as? CGFloat ?? 0) }
            }
            return PlacedAnnotation(index: index, annotation: .ink(
                strokes: strokes, color: colour("color"), width: item["width"] as? CGFloat ?? 2))
        case "note":
            guard let r = item["rect"] as? [String: Any] else { return nil }
            return PlacedAnnotation(index: index, annotation: .note(
                rect: rect(r), contents: item["contents"] as? String ?? "", color: colour("color")))
        default:
            return nil
        }
    }
}

/// The permutation that moves the page at `from` to position `to`.
///
/// `order[i]` is where the page currently at `i` ends up, which is not the same
/// as a list of destinations — getting the direction wrong reverses the move for
/// every page in between.
func reorderForMove(pageCount: Int, from: Int, to: Int) -> [Int] {
    var pages = Array(0..<pageCount)
    guard pages.indices.contains(from), pages.indices.contains(to) else { return pages }
    let moved = pages.remove(at: from)
    pages.insert(moved, at: to)

    var order = [Int](repeating: 0, count: pageCount)
    for (destination, original) in pages.enumerated() { order[original] = destination }
    return order
}
