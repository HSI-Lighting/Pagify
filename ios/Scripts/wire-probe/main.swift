// Does a mark drawn in the app actually reach the document?
//
// The house rule for this project is measure, do not infer, and the wire format
// is exactly the kind of thing that fails silently: a mistyped JSON key is not a
// compile error, and the engine answers a malformed command with an error string
// nobody reads. `rename_all_fields` on the Rust side once broke only the two
// command variants with multi-word fields — the rest kept working, so the bug
// looked like "some edits don't apply".
//
// So this drives the app's **real** encoder (`PagifyEdit.swift`, `ShapeStrokes.swift`,
// `PagifyDocument.swift` — the same files the app builds) against the same engine
// the app calls, on the host, and asserts on pixels and on state rather than on
// the absence of an error.
//
// Build and run with ios/Scripts/run-wire-probe.sh.
import CoreGraphics
import Foundation

var failures = 0

func check(_ condition: Bool, _ what: String) {
    print("  \(condition ? "ok  " : "FAIL") \(what)")
    if !condition { failures += 1 }
}

/// How much of the page is not white, as a fraction. A mark makes this go up and
/// an undo has to put it back.
func inkCoverage(_ image: CGImage) -> Double {
    let w = image.width, h = image.height
    var buffer = [UInt8](repeating: 0, count: w * h * 4)
    guard let context = CGContext(data: &buffer, width: w, height: h, bitsPerComponent: 8,
                                  bytesPerRow: w * 4, space: CGColorSpaceCreateDeviceRGB(),
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
        return 0
    }
    context.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))

    var marked = 0
    for pixel in stride(from: 0, to: buffer.count, by: 4) where
        buffer[pixel] < 240 || buffer[pixel + 1] < 240 || buffer[pixel + 2] < 240 {
        marked += 1
    }
    return Double(marked) / Double(w * h)
}

func ellipseRing(centre: CGPoint, rx: CGFloat, ry: CGFloat) -> [CGPoint] {
    (0...64).map { step in
        let angle = CGFloat(step) / 64 * 2 * .pi
        return CGPoint(x: centre.x + rx * cos(angle), y: centre.y + ry * sin(angle))
    }
}

let root = URL(fileURLWithPath: CommandLine.arguments[1])
let pdfium = root.appendingPathComponent("third_party/pdfium/pdfium-mac-arm64/lib/libpdfium.dylib")
let fixture = root.appendingPathComponent("rust/pdf_core/fixtures/text-lines.pdf")

pagify_init()
guard pagify_set_pdfium_library_path(pdfium.path) == PAGIFY_OK else {
    print("could not bind PDFium at \(pdfium.path)")
    exit(1)
}

// Copied, because the probe saves over it.
let working = FileManager.default.temporaryDirectory.appendingPathComponent("wire-probe.pdf")
try? FileManager.default.removeItem(at: working)
try FileManager.default.copyItem(at: fixture, to: working)

let document = try PagifyDocument(path: working.path, name: "wire-probe.pdf")
let size = try document.pageSize(0)
print("page 0 is \(Int(size.width))x\(Int(size.height))pt, \(document.pageCount) page(s)")

let before = inkCoverage(try document.render(page: 0, scale: 2))

// --------------------------------------------------------------------- ink --
print("\nink (a pen stroke, through the app's own encoder)")
var settings = AnnotationSettings()
settings.tool = .pen
settings.strokeWidth = 4
settings.select(.pen)

let trace = (0...40).map { step in
    CGPoint(x: 40 + CGFloat(step) * 6, y: 60 + sin(CGFloat(step) / 4) * 22)
}
var state = try document.execute(.addAnnotation(
    pageIndex: 0,
    annotation: .ink(strokes: dashed(trace, style: .solid, width: settings.strokeWidth),
                     color: settings.penColor, width: settings.strokeWidth)))

check(state.canUndo, "the engine accepted it and can undo")
check(state.dirty, "the document is dirty")

let afterInk = inkCoverage(try document.render(page: 0, scale: 2))
check(afterInk > before, "the page has more ink on it than before (\(String(format: "%.4f", before)) -> \(String(format: "%.4f", afterInk)))")

// ------------------------------------------------------------------- undo --
print("\nundo")
state = try document.undo()
check(!state.canUndo, "the history is empty again")
let afterUndo = inkCoverage(try document.render(page: 0, scale: 2))
check(abs(afterUndo - before) < 0.0001, "the page is back exactly as it was")

state = try document.redo()
check(state.canUndo, "redo put it back")

// ------------------------------------------------------------- every shape --
print("\nevery shape tool, each committed and counted")
for tool in [AnnotationTool.line, .arrow, .rectangle, .ellipse] {
    let strokes = shapeStrokes(tool: tool, start: CGPoint(x: 60, y: 150),
                               end: CGPoint(x: 300, y: 250), style: .dash1, width: 3)
    check(!strokes.isEmpty, "\(tool.label) produced strokes")
    let coverage = inkCoverage(try document.render(page: 0, scale: 2))
    _ = try document.execute(.addAnnotation(
        pageIndex: 0, annotation: .ink(strokes: strokes, color: settings.penColor, width: 3)))
    check(inkCoverage(try document.render(page: 0, scale: 2)) > coverage, "\(tool.label) drew on the page")
}

let cloud = cloudOutline((0..<24).map { step in
    let angle = CGFloat(step) / 24 * 2 * .pi
    return CGPoint(x: 200 + 70 * cos(angle), y: 200 + 45 * sin(angle))
}, width: 3)
check(cloud.count > 24, "the cloud scalloped (\(cloud.count) points)")

// -------------------------------------------------------------- highlight --
print("\nhighlight")
let coverageBeforeHighlight = inkCoverage(try document.render(page: 0, scale: 2))
_ = try document.execute(.addAnnotation(pageIndex: 0, annotation: .highlight(
    rects: [PageRect(left: 30, top: 30, right: 240, bottom: 52)],
    color: MarkColor(argb: 0x80_FF_E0_2B))))
check(inkCoverage(try document.render(page: 0, scale: 2)) > coverageBeforeHighlight,
      "the highlight washed over the words")

// ------------------------------------------------------------ page edits --
//
// On its own copy, deliberately. Run against the marked document these reorder
// page 0 to index 1 and then delete index 1 — which is the marked page. The
// first version of this probe did exactly that and then reported that the marks
// had not survived the save, which was true and had nothing to do with saving.
print("\npage commands (on a separate copy)")
do {
    let scratch = FileManager.default.temporaryDirectory.appendingPathComponent("wire-probe-pages.pdf")
    try? FileManager.default.removeItem(at: scratch)
    try FileManager.default.copyItem(at: fixture, to: scratch)
    let pages = try PagifyDocument(path: scratch.path, name: "pages")

    let pagesBefore = pages.pageCount
    var edit = try pages.execute(.insertBlankPage(at: 1, widthPt: size.width,
                                                  heightPt: size.height, fill: nil, ruling: 0))
    check(edit.pageCount == pagesBefore + 1, "insertBlankPage added a page")
    edit = try pages.execute(.setPageRotation(index: 0, quarterTurns: 1))
    check(edit.canUndo, "setPageRotation was accepted")
    edit = try pages.execute(.reorderPages(order: reorderForMove(pageCount: edit.pageCount, from: 0, to: 1)))
    check(edit.canUndo, "reorderPages was accepted")
    edit = try pages.execute(.deletePage(index: 1))
    check(edit.pageCount == pagesBefore, "deletePage removed it again")
}

// -------------------------------------------------- read back and persist --
print("\nthe marks are readable back off the page")
let placed = document.annotations(page: 0)
check(!placed.isEmpty, "getAnnotationsJson decoded \(placed.count) mark(s)")
check(placed.allSatisfy { $0.bounds != nil }, "every mark has a box the eraser can hit")

// ------------------------------------------------------------------- text --
print("\ntext, through the shaper and the layout")
PagifyFonts.directoryOverride = root.appendingPathComponent("ios/Resources/fonts")

check(PagifyFonts.register(.notoSans), "an embedded face registers with the engine")

// Latin through a bundled face: the shaper has to hand back one glyph per letter.
let latin = PagifyFonts.shape(.notoSans, "Hello")
check(latin?.glyphs.count == 5, "Noto Sans shaped 'Hello' into \(latin?.glyphs.count ?? 0) glyphs")
check(latin?.rightToLeft == false, "and left to right")

// Arabic is the case the whole font stack exists for: the letters join into
// forms that have no character of their own, and the line runs the other way.
// Laying out character by character is what made Persian come out as a row of
// isolated letters running backwards.
let arabic = PagifyFonts.shape(.notoNaskhArabic, "سلام")
check((arabic?.glyphs.count ?? 0) > 0, "Naskh shaped Persian into \(arabic?.glyphs.count ?? 0) glyphs")
check(arabic?.rightToLeft == true, "and said it runs right to left")
check(PagifyFonts.covers(.notoNaskhArabic, "سلام"), "and the face covers every character")
check(!PagifyFonts.covers(.notoSans, "سلام"), "while Noto Sans does not — so the app can say so")

let caption = TextMark(text: "Reviewed",
                       path: straightBaseline(anchor: CGPoint(x: 60, y: 120),
                                              text: "Reviewed", font: .notoSans, size: 18),
                       font: .notoSans, size: 18,
                       color: MarkColor(argb: 0xFF_1E_88_E5),
                       frame: .cloud, id: 4242)
check(caption.layOutBlock().count == 8, "the caption laid out 8 glyphs")
check(caption.layOutBlock().allSatisfy { $0.id != 0 },
      "every glyph carries a real id, not notdef")
check(caption.textFrameOutline().count > 20,
      "the cloud round it scalloped (\(caption.textFrameOutline().count) points)")

let coverageBeforeText = inkCoverage(try document.render(page: 0, scale: 2))
state = try document.execute(.addAnnotation(pageIndex: 0, annotation: .text(caption)))
check(state.canUndo, "the engine accepted the caption")
check(inkCoverage(try document.render(page: 0, scale: 2)) > coverageBeforeText,
      "and the words are on the page")

let bent = curvedBaseline(anchor: CGPoint(x: 60, y: 220), text: "Bent", font: .helvetica,
                          size: 16, degrees: 60)
check(bent.count > 2, "a bent baseline is an arc, not a segment (\(bent.count) points)")
check(abs(baselineLength(bent) - PagifyFont.helvetica.width(of: "Bent", size: 16)) < 1,
      "and is exactly as long as the words, so no letter runs off it")

// ------------------------------------------------------------------ erase --
print("\nerase: the right mark, and captions too")

// Per-shape hit testing, not bounding boxes.
let ring = WireAnnotation.ink(strokes: [ellipseRing(centre: CGPoint(x: 200, y: 200), rx: 60, ry: 40)],
                              color: settings.penColor, width: 3)
check(ring.isHitBy(CGPoint(x: 260, y: 200), tolerance: 6), "a tap ON a circle's edge finds it")
check(!ring.isHitBy(CGPoint(x: 200, y: 200), tolerance: 6),
      "a tap in its empty MIDDLE does not — a bounding box would have said yes")

let twoLines = WireAnnotation.highlight(
    rects: [PageRect(left: 40, top: 40, right: 200, bottom: 56),
            PageRect(left: 40, top: 80, right: 200, bottom: 96)],
    color: AnnotationColors.yellow)
check(twoLines.isHitBy(CGPoint(x: 100, y: 48), tolerance: 2), "a tap on a highlighted line finds it")
check(!twoLines.isHitBy(CGPoint(x: 100, y: 68), tolerance: 2),
      "a tap in the GAP between two lines does not")

// A caption is addressed by id, because it has no annotation index at all.
let erasable = TextMark(text: "Erase me",
                        path: straightBaseline(anchor: CGPoint(x: 40, y: 260),
                                               text: "Erase me", font: .helvetica, size: 14),
                        font: .helvetica, size: 14, color: settings.penColor, id: 777)
_ = try document.execute(.addAnnotation(pageIndex: 0, annotation: .text(erasable)))
let withCaption = inkCoverage(try document.render(page: 0, scale: 2))

let textRecord = MarkRecord(annotation: .text(erasable), engineIndex: nil)
if case .removeText(let page, let id)? = textRecord.removal(page: 0) {
    check(page == 0 && id == 777, "a caption's removal is removeText(id), not removeAnnotation")
    _ = try document.execute(.removeText(pageIndex: 0, id: 777))
} else {
    check(false, "a caption's removal is removeText(id), not removeAnnotation")
}
check(inkCoverage(try document.render(page: 0, scale: 2)) < withCaption,
      "and the words actually came off the page")

let inkRecord = MarkRecord(annotation: ring, engineIndex: 3)
if case .removeAnnotation(_, let index)? = inkRecord.removal(page: 0) {
    check(index == 3, "everything else goes by the engine's own index")
} else {
    check(false, "everything else goes by the engine's own index")
}

// ------------------------------------------------------- select and move --
print("\nselecting and moving a caption")

let movable = TextMark(text: "Move me",
                       path: straightBaseline(anchor: CGPoint(x: 80, y: 150),
                                              text: "Move me", font: .helvetica, size: 16),
                       font: .helvetica, size: 16, color: settings.penColor, id: 555)

// Hit testing a caption: on the words, not on the empty page beside them.
check(WireAnnotation.text(movable).isHitBy(CGPoint(x: 90, y: 150), tolerance: 4),
      "a tap on the words finds the caption")
check(!WireAnnotation.text(movable).isHitBy(CGPoint(x: 90, y: 300), tolerance: 4),
      "a tap well below them does not")

// Moving keeps the id, so undo restores rather than duplicating.
var moved = movable
let delta = CGSize(width: 40, height: -25)
moved.path = moved.path.map { CGPoint(x: $0.x + delta.width, y: $0.y + delta.height) }
check(moved.id == movable.id, "a moved caption keeps its id")
check(moved.path[0].x == movable.path[0].x + 40 && moved.path[0].y == movable.path[0].y - 25,
      "and every point of its baseline moved by the drag")
check(moved.layOutBlock().count == movable.layOutBlock().count,
      "with the same glyphs, just placed elsewhere")
let glyphBefore = movable.layOutBlock()[0].origin
let glyphAfter = moved.layOutBlock()[0].origin
check(abs((glyphAfter.x - glyphBefore.x) - 40) < 0.01
        && abs((glyphAfter.y - glyphBefore.y) + 25) < 0.01,
      "and the glyphs followed the baseline exactly")

// The caption survives a move through the engine.
_ = try document.execute(.addAnnotation(pageIndex: 0, annotation: .text(movable)))
let atFirstPlace = inkCoverage(try document.render(page: 0, scale: 2))
_ = try document.execute(.removeText(pageIndex: 0, id: 555))
_ = try document.execute(.addAnnotation(pageIndex: 0, annotation: .text(moved)))
let atSecondPlace = inkCoverage(try document.render(page: 0, scale: 2))
check(abs(atSecondPlace - atFirstPlace) < 0.002,
      "moving it re-draws the same amount of ink somewhere else")

// Re-wording rebuilds the baseline from the same anchor.
var reworded = moved
reworded.text = "Moved and reworded"
reworded.path = straightBaseline(anchor: moved.path[0], text: reworded.text,
                                 font: reworded.font, size: reworded.size)
check(reworded.path[0] == moved.path[0], "re-wording keeps the caption where it was")
check(baselineLength(reworded.path) > baselineLength(moved.path),
      "and the baseline grew with the words")

print("\ndisarming")
var armed = AnnotationSettings()
armed.select(.pen)
check(armed.tool == .pen, "a tool arms")
armed.select(.none)
check(armed.tool == .none, "and can be put down again")
armed.select(.highlight)
check(AnnotationColors.highlightPalette.contains(armed.penColor),
      "arming the highlighter snaps the colour into its own palette")
armed.select(.pen)
check(AnnotationColors.markerPalette.contains(armed.penColor),
      "and going back to the pen snaps it into the marker palette")

// ----------------------------------------------- the highlighter snaps --
print("\nthe highlighter snaps to runs of text")

// On a clean copy: the working document has this probe's own captions written
// onto it by now, and their fragments overlap the prose they sit on.
let clean = FileManager.default.temporaryDirectory.appendingPathComponent("wire-probe-text.pdf")
try? FileManager.default.removeItem(at: clean)
try FileManager.default.copyItem(at: fixture, to: clean)
let prose = try PagifyDocument(path: clean.path, name: "prose")

let runs = prose.textSegments(page: 0)
check(runs.count == 3, "the engine reports the page's \(runs.count) lines of text")

if runs.count >= 3 {
    let top = runs[0], bottom = runs[2]
    func centre(_ r: TextSegment) -> CGFloat { (r.top + r.bottom) / 2 }

    // A sweep along one line takes that line, trimmed to where it started.
    let along = TextSelection.rectsBetween(runs,
        anchor: CGPoint(x: top.left + 20, y: centre(top)),
        focus: CGPoint(x: top.right - 2, y: centre(top)))
    check(along.count == 1, "a sweep along one line selects exactly that line")
    check(along[0].left >= top.left + 19, "trimmed to where the finger went down")
    check(along[0].bottom - along[0].top < 20,
          "and its height is the line's, not the drag's")

    // A sweep down the page takes each line it crossed, as its own rect.
    let across = TextSelection.rectsBetween(runs,
        anchor: CGPoint(x: top.left + 2, y: centre(top)),
        focus: CGPoint(x: bottom.right - 2, y: centre(bottom)))
    check(across.count == 3, "a sweep down the page selects all three lines")
    check(across.allSatisfy { $0.bottom - $0.top < 20 },
          "each at its own line's height — never the height of the gaps between them")
    let covered = across.reduce(CGFloat(0)) { $0 + ($1.bottom - $1.top) }
    let dragHeight = centre(bottom) - centre(top)
    check(covered < dragHeight,
          "so the wash covers less than the drag did — a box would have covered more")

    // One annotation for the whole sweep, so erasing it takes one action.
    _ = try prose.execute(.addAnnotation(pageIndex: 0,
        annotation: .highlight(rects: across, color: AnnotationColors.yellow)))
    check(prose.annotations(page: 0).count == 1,
          "three lines highlighted, one annotation")
}

// Nothing to snap to means nothing committed.
check(TextSelection.rectsBetween([], anchor: .zero, focus: CGPoint(x: 50, y: 50)).isEmpty,
      "a page with no text yields no highlight rather than a stray box")

print("\npage edits carry their marks")
// Deleting a page renumbers everything after it.
func remapForDelete(_ index: Int) -> (Int) -> Int? {
    { page in page == index ? nil : (page > index ? page - 1 : page) }
}
let remap = remapForDelete(1)
check(remap(0) == 0, "a page before the deleted one keeps its number")
check(remap(1) == nil, "the deleted page's own marks go with it")
check(remap(2) == 1, "and a page after it shifts down by one")
check(remap(4) == 3, "however far after it sits")

// ------------------------------------------------- scaling and transfer --
print("\nscaling a caption")

var sized = AnnotationSettings()
sized.select(.text)
sized.textSize = 12
sized.textSizeCeiling = 400
sized.selectedTextId = 99

// The reached size is what the ribbon must end up showing.
let grown = min(max(sized.textSize * 1.5, AnnotationMetrics.textRange.lowerBound),
                sized.textSizeCeiling)
check(grown == 18, "pinching out 1.5x on a 12pt caption reaches 18pt")
let squashed = min(max(sized.textSize * 0.1, AnnotationMetrics.textRange.lowerBound),
                   sized.textSizeCeiling)
check(squashed == AnnotationMetrics.textRange.lowerBound,
      "and pinching far in stops at the six-point floor rather than vanishing")

// Resizing rebuilds the baseline, or the last letters fall off the end.
var big = TextMark(text: "Bigger now",
                   path: straightBaseline(anchor: CGPoint(x: 40, y: 200),
                                          text: "Bigger now", font: .helvetica, size: 12),
                   font: .helvetica, size: 12, color: AnnotationColors.markerPalette[0], id: 99)
let shortLine = baselineLength(big.path)
big.size = 24
big.path = straightBaseline(anchor: big.path[0], text: big.text, font: big.font, size: big.size)
check(baselineLength(big.path) > shortLine * 1.9,
      "doubling the size roughly doubles the baseline, so every glyph still lands on it")
check(big.layOutBlock().count == 10, "and all ten glyphs are still placed")

print("\nexporting and importing pages")
let ladder = FileManager.default.temporaryDirectory.appendingPathComponent("probe-ladder.pdf")
try? FileManager.default.removeItem(at: ladder)
try FileManager.default.copyItem(
    at: root.appendingPathComponent("rust/pdf_core/fixtures/pages-ladder.pdf"), to: ladder)
let five = try PagifyDocument(path: ladder.path, name: "ladder")
check(five.pageCount == 5, "the fixture has five pages")

// Order is the order chosen, not sorted — "page 3, then page 1" is a real ask.
let picked = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("probe-picked.pdf")
try? FileManager.default.removeItem(at: picked)
try five.exportPages([2, 0], to: picked)
let exported = try PagifyDocument(path: picked.path, name: "picked")
check(exported.pageCount == 2, "exporting two pages writes a two-page PDF")
let widths = (0..<2).compactMap { try? exported.pageSize($0).width }
let sources = (0..<5).compactMap { try? five.pageSize($0).width }
check(widths == [sources[2], sources[0]],
      "in the order they were chosen — \(widths) matches page 3 then page 1, not sorted")

let target = try PagifyDocument(path: ladder.path, name: "target")
let pagesBeforeImport = target.pageCount
_ = try target.importPages(from: exported, indices: [0, 1], at: 1)
check(target.pageCount == pagesBeforeImport + 2, "importing two pages adds two")

// ----------------------------------------------------- session recorder --
print("\nthe session recorder")

let recorder = SessionRecorder.shared
check(!recorder.isRecording, "it is off until it is switched on")
recorder.record("SHOULD_NOT_APPEAR", "page=0")

recorder.start(documentName: "probe.pdf", pageCount: 5, engineVersion: "0.1.0")
check(recorder.isRecording, "and on once started")

recorder.record("PAGE_ENTER", "page=0 pts=360 zoom=1.00")
recorder.record("PAGE_READABLE", "page=0 px=720x1012 scale=2.00", durationMillis: 61)
recorder.record("PAGE_READABLE", "page=1 px=720x1012 scale=2.00", durationMillis: 210)
recorder.record("PAGE_READABLE", "page=2 px=720x1012 scale=2.00", durationMillis: 318)
recorder.record("THUMB_HIT", "page=4")
recorder.record("ZOOM_ENTER", "page=2 target=1.40 at=180,400 base=360")

let out = FileManager.default.temporaryDirectory
guard let written = recorder.stop(directory: out) else {
    check(false, "it wrote a file"); exit(1)
}
check(!recorder.isRecording, "and is off again afterwards")
check(recorder.stop(directory: out) == nil, "stopping twice writes nothing the second time")

let text = try String(contentsOf: written, encoding: .utf8)
check(written.lastPathComponent.hasPrefix("pagify-session-")
        && written.pathExtension == "txt",
      "named \(written.lastPathComponent)")
check(!text.contains("SHOULD_NOT_APPEAR"),
      "events recorded while it was off are not in the file")

// The header carries what makes a file sent by somebody else self-describing.
check(text.contains("Pagify session recording"), "the header names the app")
check(text.contains("document : probe.pdf"), "and the document")
check(text.contains("pages    : 5"), "and the page count")
check(text.contains("engine   : pdf_core 0.1.0"), "and the engine version")
check(text.contains("t(ms)   event            detail"), "and the column heading")

// Fixed columns: a timeline is read by eye and by awk, and both need the
// columns to line up.
// `took=` rather than the kind: the summary carries a PAGE_READABLE row too.
let timeline = text.split(separator: "\n").filter {
    $0.contains("PAGE_READABLE") && $0.contains("took=")
}
check(timeline.count == 3, "all three timed events are in the timeline")
if let row = timeline.first {
    let columns = row.split(separator: " ", omittingEmptySubsequences: true)
    check(String(row.prefix(6)).trimmingCharacters(in: .whitespaces).allSatisfy(\.isNumber),
          "the timestamp occupies the first six columns")
    check(columns.dropFirst().first == "PAGE_READABLE", "then the kind")
    check(row.contains("took=61ms") || row.contains("took=210ms") || row.contains("took=318ms"),
          "and a timed event carries its duration")
}

// The summary is what makes hundreds of lines readable.
check(text.contains("Summary"), "there is a summary")
check(text.contains("total events : 6"), "counting exactly the events recorded")
check(text.contains("event            count   median    p95     max"), "with its heading")

let summaryRow = text.split(separator: "\n").first { $0.hasPrefix("PAGE_READABLE") && $0.contains("210") }
check(summaryRow != nil, "PAGE_READABLE's row reports median 210 of 61/210/318")
check(text.split(separator: "\n").contains { $0.hasPrefix("THUMB_HIT") && $0.contains("-") },
      "and an untimed kind shows dashes rather than invented numbers")

print("\n--- the file it wrote ---\n" + text + "--- end ---")
try? FileManager.default.removeItem(at: written)

print("\nsave and reopen")
try document.save(to: working, incremental: true)
let reopened = try PagifyDocument(path: working.path, name: "wire-probe.pdf")
let persisted = reopened.annotations(page: 0)
check(!persisted.isEmpty, "\(persisted.count) mark(s) survived the save")
let textMarks = PagifyEngine.string(pagify_get_text_marks_json(reopened.handle, 0)) ?? "[]"
check(textMarks.contains("Reviewed"),
      "the caption came back as a mark, not as part of the page")
check(inkCoverage(try reopened.render(page: 0, scale: 2)) > before,
      "the reopened file draws them")

print(failures == 0 ? "\nall checks passed" : "\n\(failures) CHECK(S) FAILED")
exit(failures == 0 ? 0 : 1)
