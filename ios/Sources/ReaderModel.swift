import Foundation
import SwiftUI

@MainActor
final class ReaderModel: ObservableObject {
    struct Check: Identifiable {
        let id = UUID()
        let label: String
        let detail: String
        let passed: Bool
    }

    @Published private(set) var checks: [Check] = []
    @Published private(set) var phase: ReaderPhase = .empty
    @Published private(set) var document: PagifyDocument?
    /// The file a password is being asked for, held so the retry can reuse it.
    private var lockedURL: (url: URL, scoped: Bool)?
    @Published private(set) var editState = EditState()
    // Three counters, deliberately disjoint. One counter meant every pen stroke
    // re-rasterised the whole page through PDFium — tens of milliseconds of work
    // to redraw something the app had already drawn itself.

    /// Bumped when the app's own marks change. Only the overlay redraws.
    @Published private(set) var annotationRevision = 0
    /// Bumped when the *page* changes — an edit to the page tree, a rotation, a
    /// save. Only this forces a re-raster.
    @Published private(set) var pageContentRevision = 0

    /// A revision per page, so one page's edit does not restart every other
    /// page's render.
    ///
    /// A single global counter meant that moving a caption on page 0 changed the
    /// task id of every built row: a recording caught three full-page renders
    /// firing on the same millisecond, 69ms each, for one caption that touched
    /// exactly one of them.
    @Published private(set) var pageRevisions: [Int: Int] = [:]

    /// What a page's renderer should key on: its own revision, plus the global
    /// one so a reorder or a reload still invalidates everything.
    func contentRevision(page: Int) -> Int {
        pageContentRevision &+ (pageRevisions[page] ?? 0)
    }
    /// Bumped when a different document is opened. Per-page load work keys off it.
    @Published private(set) var documentRevision = 0
    @Published var settings = AnnotationSettings()

    /// The marks on each page, as the app knows them.
    ///
    /// **Held here rather than read back off the page**, because the engine
    /// cannot give them back faithfully: an ink annotation's nib lives in `/BS`,
    /// which PDFium neither writes nor reads, so `getAnnotationsJson` reports
    /// every stroke at `DEFAULT_INK_WIDTH_POINTS` however thick it was drawn.
    /// PDFium's own appearance stream has the same default in it, which is why a
    /// committed stroke came back thin.
    ///
    /// So the app draws its marks itself, over the page, at the width it holds —
    /// exactly as Android's `AnnotationLayer` does. PDFium's thin version stays
    /// underneath, covered by the heavier one.
    @Published private(set) var marks: [Int: [MarkRecord]] = [:]

    /// Snapshots of `marks`, mirroring the engine's own history so undo and redo
    /// move both together.
    ///
    /// Capped. An unbounded stack of whole-dictionary snapshots grows with every
    /// stroke and is never released for as long as the document is open — a long
    /// markup session is a slow leak that ends in the trim callback throwing the
    /// document away.
    private var markUndo: [[Int: [MarkRecord]]] = []
    private var markRedo: [[Int: [MarkRecord]]] = []
    private static let historyLimit = 200
    @Published var failure: String?
    @Published var notice: String?

    /// Where the open document lives. A save writes back here, via a scratch
    /// file — never in place.
    private var source: URL?
    private weak var recents: RecentDocumentsStore?
    /// How many of the marks on screen came out of the file rather than out of
    /// this session.
    private var savedMarkCount = 0

    /// Whether the page rail is showing. Reader state, not a persisted setting:
    /// it is switched off automatically on a narrow screen, and a preference that
    /// changed itself when the phone was turned would not be a preference.
    @Published var showThumbnails = true
    /// Each page's width ÷ height, learned as pages are measured.
    ///
    /// The reader needs the height of the whole document before it has drawn any
    /// of it — a scroll view has to know how far it scrolls. Measuring lazily and
    /// falling back to A4 keeps that honest without opening every page up front.
    @Published private(set) var pageAspects: [Int: CGFloat] = [:]

    func noteAspect(_ aspect: CGFloat, page: Int) {
        guard aspect > 0, pageAspects[page] != aspect else { return }
        pageAspects[page] = aspect
    }

    /// A4 until the real thing is known.
    func aspect(page: Int) -> CGFloat { pageAspects[page] ?? (595.0 / 842.0) }

    /// This page's real size, cached, answered synchronously.
    ///
    /// A row that lays itself out at a guessed A4 shape and corrects once its
    /// raster arrives changes its own height — which moves every page below it,
    /// which moves the scroll position, which republishes the page frames. With
    /// pages built lazily as you scroll, that correction never stops arriving, and
    /// the list creeps under your finger for the whole life of the document. The
    /// engine can answer this before the first layout, so nothing ever has to be
    /// corrected.
    private var pageSizeCache: [Int: CGSize] = [:]
    func pageSize(_ index: Int) -> CGSize {
        if let cached = pageSizeCache[index] { return cached }
        guard let size = try? document?.pageSize(index), size.width > 0 else {
            return CGSize(width: 595, height: 842)
        }
        pageSizeCache[index] = size
        return size
    }

    /// Measure every page up front, in one state update.
    ///
    /// Without it a page laid out at a guessed A4 aspect on a landscape document
    /// is roughly a two-fold error *per page*, and those errors accumulate above
    /// the viewport and drag the list's anchor out from under the reader — which
    /// is what makes the rail land pages away from the one being read.
    ///
    /// Affordable because measuring a page reads the page tree; it does not load
    /// the page.
    private func measureAllPages(_ document: PagifyDocument) {
        let count = document.pageCount
        Task.detached(priority: .utility) { [weak self] in
            var measured: [Int: CGFloat] = [:]
            for index in 0..<count {
                guard let size = try? document.pageSize(index), size.height > 0 else { continue }
                measured[index] = size.width / size.height
            }
            await MainActor.run { [weak self] in
                guard let self else { return }
                // Applied as one update, and anything already measured wins —
                // a real measurement must not be replaced by a stale one.
                self.pageAspects = measured.merging(self.pageAspects) { _, existing in existing }
            }
        }
    }

    /// The page pinned into the magnified view, if any.
    ///
    /// Zooming leaves the list behind entirely rather than scaling it: panning a
    /// magnified page should never wander into its neighbours, and a list scaled
    /// by its layout width re-measures every row on every pinch.
    /// Captions currently taken **out** of the page, and the page each belongs to.
    ///
    /// A caption is page content, so every change to one rewrites the page and
    /// re-rasterises it. While a caption is in hand it is therefore lifted out of
    /// the document altogether and drawn by the overlay alone: moving and resizing
    /// it then cost nothing at all, and it is written back — once — when it is put
    /// down. Without this, a recording caught a full-page render landing in the
    /// middle of a pinch, 113ms, while the fingers were still moving.
    ///
    /// Anything that reads the document rather than the overlay must settle these
    /// first; `save` and `saveCopy` do.
    private var liftedText: [Int32: Int] = [:]

    /// Rewrite one caption where it now sits: out of the page, and back in.
    ///
    /// Captions are page **content**, so this re-rasterises the page. That is far
    /// too expensive to do on every frame of a pinch — it is what made resizing
    /// crawl — so during a gesture the overlay alone is updated and this is left
    /// until the fingers lift.
    private func writeText(_ mark: TextMark, page: Int) {
        run(.removeText(pageIndex: page, id: Int(mark.id)))
        run(.addAnnotation(pageIndex: page, annotation: .text(mark)))
    }

    /// Two fingers are down. Idempotent: the pinch reports this on every frame,
    /// and re-publishing an unchanged value re-renders the whole reader each time.
    func beginPinch() {
        guard !isPinching else { return }
        isPinching = true
    }

    /// The fingers have lifted. The caption stays out of the page while it is
    /// still in hand — it is put back when it is put down.
    func endPinch() {
        isPinching = false
    }

    /// Take one caption out of the page so the overlay alone draws it.
    private func liftText(id: Int32) {
        guard liftedText[id] == nil, let page = pageOfTextMark(id: id) else { return }
        liftedText[id] = page
        run(.removeText(pageIndex: page, id: Int(id)))
    }

    /// Put every lifted caption back where it now sits.
    ///
    /// Must run before anything reads the document — a save above all, or the
    /// caption would simply not be in the file.
    func settleLiftedText() {
        guard !liftedText.isEmpty else { return }
        let owed = liftedText
        liftedText = [:]
        for (id, page) in owed {
            guard let mark = textMark(id: id) else { continue }
            run(.addAnnotation(pageIndex: page, annotation: .text(mark)))
        }
    }

    /// A two-finger gesture is in progress.
    ///
    /// The caption drag cannot work this out for itself: a recogniser is only
    /// handed the touches that land on **its own** view, and the second finger of
    /// a pinch almost always lands on bare page. So the drag would go on carrying
    /// the words around while the other hand was trying to resize them.
    @Published var isPinching = false

    @Published var zoomedPage: Int?
    /// Where the entering gesture was aimed, as a 0…1 fraction of that page.
    @Published var zoomFocus: CGPoint?
    @Published var zoomTarget: CGFloat = 1

    func enterZoom(page: Int, focus: CGPoint, target: CGFloat) {
        zoomFocus = focus
        zoomTarget = Zoom.clamp(target)
        zoomedPage = page
        currentPage = page
    }

    func exitZoom() {
        // The page that was pinned stays the current one, and the list is asked
        // to go there. Without this the ScrollView is rebuilt from scratch and
        // the reader is dropped at the top of the document.
        let wasPinned = zoomedPage
        zoomedPage = nil
        zoomFocus = nil
        zoomTarget = 1

        guard let page = wasPinned else { return }
        currentPage = page
        restoreOnAppear = page
        jumpRequested = true
        settleJump?.cancel()
        settleJump = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(250))
            guard !Task.isCancelled else { return }
            self?.jumpRequested = false
        }
    }

    /// Move to the next page (+1) or the previous (−1), staying magnified.
    /// False when there is nowhere to go, which springs the pull back instead.
    func turnZoomedPage(_ delta: Int) -> Bool {
        guard let current = zoomedPage, let document else { return false }
        let next = current + delta
        guard next >= 0, next < document.pageCount else { return false }
        zoomedPage = next
        currentPage = next
        loadMarks(page: next)
        loadSegments(page: next)
        return true
    }

    /// Bumped when **the reader** is scrolled by hand, and only then.
    ///
    /// The rail follows this rather than `currentPage`, so choosing a page from
    /// the rail leaves the rail exactly where it is: you were looking at a
    /// particular run of pages, and moving it out from under you loses your place
    /// and makes picking a second page from that run needlessly awkward.
    @Published private(set) var readerFollowTick = 0

    /// The reader scrolled under its own steam onto `page`.
    func readerScrolled(to page: Int) {
        guard page != currentPage else { return }
        currentPage = page
        readerFollowTick += 1
    }

    @Published var currentPage = 0
    /// Set when the jump came from the strip or the navigator rather than from
    /// the reader scrolling past a page, so the two do not fight each other.
    @Published var jumpRequested = false

    /// A page the list should open on, consumed once.
    ///
    /// Leaving the magnified page builds the list again from nothing, and a new
    /// ScrollView opens at the top of the document — so the restore has to happen
    /// when the list appears. It must happen **once**: an unconditional
    /// `scrollTo` on appearance re-fires whenever anything rebuilds that view,
    /// and drags the reader back up the document mid-scroll.
    var restoreOnAppear: Int?

    /// Take the pending restore, if there is one. Reading it clears it.
    func takeRestore() -> Int? {
        defer { restoreOnAppear = nil }
        return restoreOnAppear
    }

    /// How many marks are on screen that the file does not have yet.
    var unsavedMarkCount: Int {
        marks.values.reduce(0) { $0 + $1.count } - savedMarkCount
    }

    /// Whether there is anything at all to write.
    var hasUnsavedWork: Bool { editState.dirty || unsavedMarkCount > 0 }

    /// Save tests **both** halves. Testing only the engine's dirty flag left the
    /// button greyed out with a page full of fresh marks on it.
    var canSave: Bool { hasUnsavedWork && source != nil }
    var isEditable: Bool { editState.editable }

    // ----------------------------------------------------------- lifecycle --

    func start(recents: RecentDocumentsStore) {
        self.recents = recents
        guard checks.isEmpty else { return }
        do {
            try PagifyEngine.start()
            record("engine", "pdf_core \(PagifyEngine.version)", true)
            record("PDFium", "chromium/7881, embedded and bound", true)

            for outcome in try PixelOrderProbe.run() {
                let name = outcome.requested == .rgba ? "RGBA" : "BGRA"
                record("render as \(name)", "\(outcome.hex)  (\(outcome.reading))",
                       outcome.isAsRequested)
            }
        } catch {
            failure = error.localizedDescription
        }
    }

    /// Build a document that does not exist yet, in the app's own Documents
    /// folder — so it is a real file from the moment it exists at all, visible in
    /// Files and openable by the ordinary path like any other.
    func createBlank(pages: Int, size: PaperSize, ruling: Ruling, fill: MarkColor?) {
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let url = uniqueURL(in: documents, named: "Untitled")

        // `Darwin.open`, spelled out: the model has its own `open(url:scoped:)`
        // and the unqualified name resolves to that instead.
        let fd = Darwin.open(url.path, O_CREAT | O_WRONLY | O_TRUNC, 0o644)
        guard fd >= 0 else {
            failure = "could not create \(url.lastPathComponent) (errno \(errno))"
            return
        }
        // Ownership of `fd` transfers to the engine on every path, including the
        // failing ones — there is nothing to close here.
        guard pagify_create_blank_document(fd, Int32(pages), Float(size.size.width),
                                           Float(size.size.height),
                                           Int32(bitPattern: fill?.argb ?? 0),
                                           Int32(ruling.rawValue)) == PAGIFY_OK else {
            failure = PagifyEngine.lastError() ?? "could not build the document"
            return
        }

        do {
            try load(url: url, name: url.lastPathComponent, scopedURL: nil)
        } catch {
            failure = error.localizedDescription
        }
    }

    private func uniqueURL(in directory: URL, named base: String) -> URL {
        var candidate = directory.appendingPathComponent("\(base).pdf")
        var suffix = 2
        while FileManager.default.fileExists(atPath: candidate.path) {
            candidate = directory.appendingPathComponent("\(base) \(suffix).pdf")
            suffix += 1
        }
        return candidate
    }

    /// Open a file the reader chose.
    ///
    /// The security-scoped grant is taken *before* the open and handed to the
    /// document, which holds it until it closes — reading a page later is just as
    /// much an access as opening was, and dropping the scope in between is the
    /// bug that only shows up on the second page.
    func open(picked url: URL) {
        open(url: url, scoped: url.startAccessingSecurityScopedResource())
    }

    /// Open a file, with the security-scoped access already taken if it needed
    /// one — as it does when it came from the picker or from a stored bookmark.
    func open(url: URL, scoped: Bool) {
        phase = .loading
        do {
            try load(url: url, name: url.lastPathComponent, scopedURL: scoped ? url : nil)
        } catch PagifyError.needsPassword(let retry) {
            // Not a failure and not an empty reader — the document is fine, it
            // just will not open without a word. Treating it as either is how an
            // encrypted PDF became simply un-openable.
            lockedURL = (url, scoped)
            phase = .passwordRequired(retry: retry)
        } catch {
            if scoped { url.stopAccessingSecurityScopedResource() }
            phase = .failed(error.localizedDescription)
        }
    }

    /// Try the password the reader typed.
    func unlock(with password: String) {
        guard let locked = lockedURL else { return }
        phase = .loading
        do {
            try load(url: locked.url, name: locked.url.lastPathComponent,
                     scopedURL: locked.scoped ? locked.url : nil, password: password)
            lockedURL = nil
        } catch PagifyError.needsPassword {
            phase = .passwordRequired(retry: true)
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }

    private func load(url: URL, name: String, scopedURL: URL?,
                      password: String? = nil) throws {
        // Released before the next one opens: two documents' caches at once is
        // the memory the trim callback exists to fight.
        document = nil

        let opened = try PagifyDocument(path: url.path, password: password,
                                        name: name, scopedURL: scopedURL)
        opened.setCacheBudget(bytes: 96 * 1024 * 1024)
        document = opened
        source = url
        marks = [:]
        markUndo = []
        markRedo = []
        savedMarkCount = 0
        segments = [:]
        pagesWithoutSelectableText = []
        pageAspects = [:]
        currentPage = 0
        editState = opened.editState()
        documentRevision += 1
        pageContentRevision += 1
        annotationRevision += 1
        recents?.remember(url: url, name: name, pageCount: opened.pageCount)
        measureAllPages(opened)
        phase = .ready
    }

    /// Go to a page, and hold off the geometry watcher while the scroll runs.
    ///
    /// The settle window is owned here rather than by whoever observes
    /// `currentPage`. It used to be cleared from an `onChange` on that property —
    /// which never fires when the jump lands on the page already showing, so
    /// tapping the current thumbnail left the flag stuck true for the rest of the
    /// session. Everything gated on it then stopped: the rail no longer followed
    /// the reader, and the zoom refused every pinch because the page frames it
    /// reads were behind the same gate.
    func jumpTo(_ page: Int) {
        guard page != currentPage else { return }
        jumpRequested = true
        currentPage = page

        settleJump?.cancel()
        settleJump = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(250))
            guard !Task.isCancelled else { return }
            self?.jumpRequested = false
        }
    }

    private var settleJump: Task<Void, Never>?

    // -------------------------------------------------------------- editing --

    /// Commit a mark drawn on `page`.
    func commit(_ annotation: WireAnnotation, page: Int) {
        rememberMarks()
        run(.addAnnotation(pageIndex: page, annotation: annotation))
        marksChanged()

        // The engine's index is read back rather than guessed. A page can carry
        // form widgets and links the engine does not model, so "one more than
        // last time" is not the same as PDFium's index — and erasing by a guessed
        // index deletes somebody's form field.
        var record = MarkRecord(annotation: annotation, engineIndex: nil)
        if case .text = annotation {
            // Captions have no annotation index at all; they are addressed by id.
        } else {
            record.engineIndex = document?.annotations(page: page).last?.index
        }
        marks[page, default: []].append(record)
    }

    /// Where a caption is being placed, on which page, and along what.
    ///
    /// `to` is the far end of the drag when there was one — a caption is dragged
    /// to say where it runs, and only falls back to a metrics-measured baseline
    /// when the drag had no length.
    @Published var placingText: (page: Int, from: CGPoint, to: CGPoint?)?

    func beginText(page: Int, from: CGPoint, to: CGPoint?) {
        placingText = (page, from, to)
    }

    /// Take a caption in hand, and pull its style into the ribbon.
    ///
    /// Selection is **by id**, never by position: a list index changes under the
    /// mark the moment anything else on the page is added or erased, and the
    /// ribbon would then be restyling somebody else's words.
    func selectText(_ id: Int32?) {
        // Whatever was in hand goes back into the page before anything else does.
        if settings.selectedTextId.map(Int32.init) != id { settleLiftedText() }

        settings.selectedTextId = id.map(Int64.init)
        if let id { liftText(id: id) }

        guard let id, let mark = textMark(id: id) else {
            settings.textSizeCeiling = AnnotationMetrics.textRange.upperBound
            return
        }
        settings.textSizeCeiling = ceiling(for: mark, on: pageOfTextMark(id: id))
        settings.font = mark.font
        settings.textSize = mark.size
        settings.penColor = mark.color
        settings.curveDegrees = mark.curveDegrees
        // A block does not bend, so the ribbon stops offering it.
        settings.textBendApplies = !mark.isMultiLine
    }

    /// The largest point size this caption may take on this page.
    ///
    /// The ceiling is the sheet, not a number: it is the size at which *these
    /// words in this face* still fit across the page, so a long caption is held
    /// down further than a short one. Measured against the page it actually sits
    /// on, not the one on screen. 400 is only the backstop for when there is no
    /// page to ask.
    func ceiling(for mark: TextMark?, on page: Int?) -> CGFloat {
        guard let mark, let page, let document,
              let size = try? document.pageSize(page), size.width > 0 else {
            return AnnotationMetrics.textRange.upperBound
        }
        return sizeThatFits(mark.text, font: mark.font,
                            availableWidth: size.width * AnnotationMetrics.textPageFraction)
    }

    /// The caption with this id, wherever it is.
    func textMark(id: Int32) -> TextMark? {
        for (_, records) in marks {
            for record in records {
                if case .text(let mark) = record.annotation, mark.id == id { return mark }
            }
        }
        return nil
    }

    private func pageOfTextMark(id: Int32) -> Int? {
        for (page, records) in marks {
            for record in records {
                if case .text(let mark) = record.annotation, mark.id == id { return page }
            }
        }
        return nil
    }

    /// Move a caption by `delta` page points.
    ///
    /// Replaced **at its existing index**, never appended — a mark that jumped to
    /// the end of the list every time it was nudged would climb over everything
    /// drawn on top of it.
    func moveMark(id: Int32, delta: CGSize) {
        guard delta != .zero,
              let page = pageOfTextMark(id: id),
              let index = marks[page]?.firstIndex(where: {
                  if case .text(let m) = $0.annotation { return m.id == id }
                  return false
              }),
              case .text(var mark) = marks[page]![index].annotation else { return }

        rememberMarks()
        // Held on the sheet. Words dragged past the edge are drawn outside the
        // page and clipped away — at which point they cannot be tapped, moved or
        // erased, because there is nothing left on screen to aim at. The whole
        // frame is kept inside, not just the baseline, so a boxed or clouded
        // caption keeps its ring too.
        mark.path = mark.path.map {
            CGPoint(x: $0.x + delta.width, y: $0.y + delta.height)
        }
        if let size = try? document?.pageSize(page) {
            // What is actually drawn, and nothing more.
            //
            // `textFrameBounds` is the block inflated by nearly half the point
            // size on every side — the room a cloud or a box needs. Clamping a
            // *plain* caption against it fenced the words off from that much of
            // the sheet on all four sides, which at a large point size is a broad
            // band of page nothing can be placed in. An unframed caption is only
            // its words, so it is held to those.
            let box = mark.frame == .none ? mark.textBlockBounds() : mark.textFrameBounds()
            let nudge = CGSize(
                width: max(-box.left, min(0, size.width - box.right)),
                height: max(-box.top, min(0, size.height - box.bottom)))
            if nudge != .zero {
                mark.path = mark.path.map {
                    CGPoint(x: $0.x + nudge.width, y: $0.y + nudge.height)
                }
            }
        }

        marks[page]![index] = MarkRecord(annotation: .text(mark), engineIndex: nil)

        if liftedText[mark.id] != nil {
            // In hand, so out of the page: the overlay already draws it where the
            // finger left it, and the document hears about it on put-down.
            marksChanged()
        } else {
            // `apply` bumps the page's revision, which is what redraws it.
            writeText(mark, page: page)
        }
    }

    /// Rebuild the caption in hand with whatever the ribbon now says.
    ///
    /// One funnel for all five style controls, so a caption cannot be re-fonted
    /// by one path and resized by another that forgot to remeasure the baseline.
    /// The label is what undo will be called.
    /// Restyle the caption in hand — the one funnel every style control goes
    /// through, so they cannot disagree about what a change means.
    ///
    /// The label names the control, for the session log. With nothing held this
    /// does nothing at all, which is what makes every control both sticky and
    /// selective: it always sets what the *next* caption will look like, and it
    /// additionally restyles this one.
    func restyleSelected(_ label: String) {
        restyleSelected(label) { mark in
            // Only the field the control actually moved. Rewriting all of them
            // from the ribbon made a pinch quietly repaint the caption in
            // whatever colour and face the ribbon happened to be showing.
            switch label {
            case "font":   return mark.rebuilt(font: settings.font)
            case "size":   return mark.rebuilt(size: settings.textSize)
            case "bend":   return mark.rebuilt(curveDegrees: settings.curveDegrees)
            case "colour": return mark.rebuilt(color: settings.penColor)
            default:       return mark
            }
        }
    }

    func restyleSelected(_ label: String, _ change: (TextMark) -> TextMark) {
        guard let id = settings.selectedTextId.map(Int32.init),
              let page = pageOfTextMark(id: id),
              let index = marks[page]?.firstIndex(where: {
                  if case .text(let m) = $0.annotation { return m.id == id }
                  return false
              }),
              case .text(let before) = marks[page]![index].annotation else { return }

        let mark = change(before)
        // A control moved back where it was must not leave an undo step that
        // undoes nothing.
        guard mark != before else { return }

        rememberMarks()
        marks[page]![index] = MarkRecord(annotation: .text(mark), engineIndex: nil)
        if liftedText[mark.id] != nil {
            // Already out of the page: the overlay is the only thing drawing it,
            // so there is nothing to write and nothing to re-rasterise.
            marksChanged()
        } else {
            writeText(mark, page: page)
        }
        // No redraw is asked for here. `apply` already bumps the revision on every
        // engine command, so `writeText` covers the case that changed the page —
        // and asking for one *unconditionally* asked for it on the deferred path
        // too. A recording caught the result: the write was held back for the
        // whole pinch, as intended, while the page re-rasterised 3321x4724 anyway,
        // two and three renders deep, 135-362ms each, on every frame.

        // Recomputed from the mark as it now stands: it may be longer, or in a
        // wider face, so the size that still fits the sheet has moved with it —
        // and a second line takes the bend away.
        settings.textSizeCeiling = ceiling(for: mark, on: page)
        settings.textBendApplies = !mark.isMultiLine
        settings.textSize = mark.size

        notice = nil
        SessionRecorder.shared.record("TEXT_RESTYLE", "id=\(mark.id) what=\(label)")
    }

    /// Resize the caption in hand by pinching it.
    ///
    /// The reached size is mirrored back into the ribbon, so the slider and the
    /// number agree with the words on the page — and so the *next* caption is
    /// written at the size the last one ended up.
    func scaleSelectedText(_ factor: CGFloat) {
        guard factor != 1, factor.isFinite, factor > 0,
              settings.selectedTextId != nil else { return }

        // Measured from **the caption's own** size, not the ribbon's number: the
        // two drift apart the moment anything else is picked up, and scaling the
        // slider instead makes the first pinch jump the caption to whatever the
        // last one was set to.
        restyleSelected("size") { mark in
            mark.rebuilt(size: min(mark.size * factor, settings.textSizeCeiling))
        }
        // The slider follows the pinch, so the two controls never disagree about
        // how big the caption in hand is.
        if let id = settings.selectedTextId.map(Int32.init), let mark = textMark(id: id) {
            settings.textSize = mark.size
        }
    }

    /// Open a caption for re-wording.
    func editText(id: Int32) {
        guard let page = pageOfTextMark(id: id), let mark = textMark(id: id) else { return }
        editingText = (page: page, mark: mark)
    }

    /// The caption being re-worded, if any.
    @Published var editingText: (page: Int, mark: TextMark)?

    /// A note waiting for its words, and where it goes.
    @Published var pendingNote: (page: Int, at: CGPoint)?
    /// A note being read.
    @Published var openNote: (page: Int, index: Int, contents: String)?

    func requestNote(page: Int, at point: CGPoint) {
        pendingNote = (page, point)
    }

    func openNote(page: Int, index: Int) {
        guard case .note(_, let contents, _)? = marks[page]?[index].annotation else { return }
        openNote = (page, index, contents)
    }

    /// Commit a note once its words are known. Blank words are not a note.
    func commitNote(_ words: String) {
        guard let pending = pendingNote else { return }
        pendingNote = nil
        guard !words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }

        let radius = AnnotationMetrics.noteMarkerRadius
        commit(.note(rect: PageRect(left: pending.at.x - radius, top: pending.at.y - radius,
                                    right: pending.at.x + radius, bottom: pending.at.y + radius),
                     contents: words,
                     color: settings.penColor),
               page: pending.page)
    }

    /// Replace a caption's words, keeping everything else about it.
    ///
    /// Emptying the words **is** the delete gesture — one ordinary undo step,
    /// rather than a separate control that has to be found.
    /// Take one caption off a page by its id, wherever it sits in the stack.
    ///
    /// Addressed by id rather than by a point, because emptying the words is a
    /// delete with no finger anywhere near the caption.
    func erase(textId: Int32, page: Int) {
        guard let index = marks[page]?.firstIndex(where: {
            if case .text(let m) = $0.annotation { return m.id == textId }
            return false
        }) else { return }

        let wasLifted = liftedText[textId] != nil
        liftedText[textId] = nil
        if settings.selectedTextId == Int64(textId) { selectText(nil) }
        rememberMarks()
        marks[page]?.remove(at: index)
        // Already out of the page if it was in hand; removing again would fail.
        if !wasLifted { run(.removeText(pageIndex: page, id: Int(textId))) }
    }

    func commitEdit(_ words: String) {
        guard let editing = editingText else { return }
        editingText = nil

        // Emptying the words *is* the delete. There is no other way to remove a
        // caption without reaching for the eraser, and an empty one cannot be
        // seen — so it could only ever be found by erasing at random.
        guard !words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            erase(textId: editing.mark.id, page: editing.page)
            return
        }

        restyleSelected("words") { $0.rebuilt(text: words) }
    }

    /// Commit a caption once its words are known.
    ///
    /// The baseline is built here, from the same font metrics the preview walks —
    /// only one side can be the authority on where a letter sits, and it has to be
    /// the side the person was looking at when they put it there.
    func commitText(_ words: String) {
        guard let placing = placingText else { return }
        placingText = nil

        // Trimmed only to decide whether there is anything there. The mark is
        // built from `words` verbatim — a deliberate blank first line is part of
        // the caption, and cutting it changes where the block sits.
        guard !words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        let trimmed = words

        // A block does not bend: stacked arcs curl into each other as the bend
        // grows, and there is no answer for where the second arc should sit.
        let bends = settings.tool.bendsText && !trimmed.contains("\n")
        let first = captionLines(trimmed).first ?? trimmed

        // Measured from the font's own metrics, never drawn. The baseline is
        // exactly as long as the words, so every glyph lands on it however far it
        // bends — a line whose length came from a finger is unrelated to the text
        // and truncates anything longer.
        let path = bends
            ? curvedBaseline(anchor: placing.from, text: first, font: settings.font,
                             size: settings.textSize, degrees: settings.curveDegrees)
            : straightBaseline(anchor: placing.from, text: first, font: settings.font,
                               size: settings.textSize)

        let mark = TextMark(text: trimmed,
                            path: path,
                            font: settings.font,
                            size: settings.textSize,
                            color: settings.penColor,
                            frame: settings.tool.textFrame,
                            curveDegrees: bends ? settings.curveDegrees : 0,
                            id: Int32.random(in: 1...Int32.max))

        commit(.text(mark), page: placing.page)
        // Taken in hand the moment it lands. Without this the ribbon is not about
        // the caption you just wrote: changing the size, the font or the colour
        // silently changes only the default for the *next* one.
        selectText(mark.id)
    }

    /// The runs of text on each page, loaded when the page is first shown.
    @Published private(set) var segments: [Int: [TextSegment]] = [:]
    /// Pages the engine found no selectable text on — a scan, or a drawing.
    @Published private(set) var pagesWithoutSelectableText: Set<Int> = []

    func loadSegments(page: Int) {
        guard segments[page] == nil, let document else { return }
        let runs = document.textSegments(page: page)
        segments[page] = runs
        if runs.isEmpty { pagesWithoutSelectableText.insert(page) }
    }

    /// The highlighter swept across something that was not text.
    func highlightMissed(page: Int) {
        notice = pagesWithoutSelectableText.contains(page)
            ? "There is no selectable text on this page."
            : "Nothing to highlight there — sweep across some words."
    }

    /// What is already on a page when it is first shown.
    ///
    /// Read once. After that the app's own list is the truth — reloading would
    /// pull our own marks back in at the engine's default nib and duplicate them.
    func loadMarks(page: Int) {
        guard marks[page] == nil, let document else { return }
        var loaded = document.annotations(page: page).map {
            MarkRecord(annotation: $0.annotation, engineIndex: $0.index)
        }
        // Captions are page content, so they come back through their own door —
        // the blob stored beside the words, not the annotation list. Without this
        // a saved caption is unerasable forever: the ring goes and the words stay.
        loaded.append(contentsOf: document.textMarks(page: page).map {
            MarkRecord(annotation: .text($0), engineIndex: nil)
        })
        marks[page] = loaded
        savedMarkCount += loaded.count
        marksChanged()
    }

    /// Rub out whatever is under `point` on `page`.
    ///
    /// Addressed by the engine's own index, never by position in this list: a page
    /// can carry form widgets and links the engine does not model, and numbering
    /// our own results would delete somebody's form field instead of their
    /// highlight.
    /// Whether a rub-out sweep is in progress, and the state it started from.
    private var eraseSweep: [Int: [MarkRecord]]?

    /// One sweep of the eraser is **one** undo step.
    ///
    /// Without the bracket, dragging across six marks costs six taps of undo to
    /// put back — and the reader has no way to know how many it was.
    func beginErase() {
        eraseSweep = marks
    }

    func endErase() {
        defer { eraseSweep = nil }
        guard let before = eraseSweep, before.mapValues(\.count) != marks.mapValues(\.count) else {
            // Nothing was taken, so nothing is recorded. An undo step that undoes
            // nothing is worse than no step at all.
            return
        }
        markUndo.append(before)
        if markUndo.count > Self.historyLimit { markUndo.removeFirst() }
        markRedo.removeAll()
    }

    func erase(at point: CGPoint, page: Int) {
        // Hit-tested against the app's own list, not the engine's read-back.
        // The engine cannot report an ink nib (`/BS` does not round-trip) and
        // does not report captions at all, so its list is both thinner and
        // differently shaped — reconciling the two by comparing bounds was how
        // erasing a highlight removed it from the page but left it on screen.
        guard let hit = marks[page]?.lastIndex(where: {
            $0.annotation.isHitBy(point, tolerance: eraserTouchRadius)
        }) else { return }

        guard let record = marks[page]?[hit],
              let removal = record.removal(page: page) else { return }

        // Nothing is held any more if what was held is what was erased —
        // otherwise the ribbon goes on restyling a mark that is gone, and the
        // next caption inherits the edits meant for it.
        if case .text(let mark) = record.annotation,
           settings.selectedTextId == Int64(mark.id) {
            selectText(nil)
        }

        if eraseSweep == nil { rememberMarks() }
        marks[page]?.remove(at: hit)

        // Everything after it on the page shuffles down one in PDFium's numbering.
        if let index = record.engineIndex {
            for position in marks[page]!.indices {
                if let other = marks[page]![position].engineIndex, other > index {
                    marks[page]![position].engineIndex = other - 1
                }
            }
        }
        run(removal)
    }

    func undo() {
        guard let document else { return }
        if let previous = markUndo.popLast() {
            markRedo.append(marks)
            marks = previous
        }
        apply { try document.undo() }
    }

    func redo() {
        guard let document else { return }
        if let next = markRedo.popLast() {
            markUndo.append(marks)
            marks = next
        }
        apply { try document.redo() }
    }

    /// Snapshot before a change, so undo has something to go back to. A new edit
    /// discards the redo branch, as every history does.
    private func rememberMarks() {
        markUndo.append(marks)
        if markUndo.count > Self.historyLimit { markUndo.removeFirst() }
        markRedo.removeAll()
    }

    func deletePage(_ index: Int) {
        // Everything after it shifts down one; the page itself takes its marks
        // with it.
        runPageEdit(.deletePage(index: index)) { page in
            page == index ? nil : (page > index ? page - 1 : page)
        }
        if currentPage >= (document?.pageCount ?? 1) {
            currentPage = max(0, (document?.pageCount ?? 1) - 1)
        }
    }

    func movePage(from: Int, to: Int) {
        guard let document else { return }
        let order = reorderForMove(pageCount: document.pageCount, from: from, to: to)
        runPageEdit(.reorderPages(order: order)) { page in
            order.indices.contains(page) ? order[page] : page
        }
    }

    func insertBlankPage(after index: Int) {
        guard let document, let size = try? document.pageSize(max(0, index)) else { return }
        let at = index + 1
        runPageEdit(.insertBlankPage(at: at, widthPt: size.width, heightPt: size.height,
                                     fill: nil, ruling: 0)) { page in
            page >= at ? page + 1 : page
        }
    }

    /// Run an edit that changes the page tree, moving the marks with it.
    ///
    /// **Without this every mark after a deleted page draws on the wrong page.**
    /// The marks are keyed by page index, and the page tree renumbers underneath
    /// them — so a highlight on page 5 silently becomes a highlight on page 4's
    /// words. Rotation is the same problem in a different axis.
    ///
    /// The mark history is discarded rather than remapped: an undo record holds
    /// positions in a page tree that no longer exists, and replaying one against
    /// the new tree puts marks somewhere nobody chose.
    private func runPageEdit(_ command: PagifyCommand,
                             remap: (Int) -> Int?) {
        var moved: [Int: [MarkRecord]] = [:]
        var lost = 0
        for page in marks.keys.sorted() {
            guard let records = marks[page] else { continue }
            if let destination = remap(page) {
                moved[destination, default: []].append(contentsOf: records)
            } else {
                lost += records.count
            }
        }

        var movedSegments: [Int: [TextSegment]] = [:]
        for page in segments.keys.sorted() {
            if let destination = remap(page), let runs = segments[page] {
                movedSegments[destination] = runs
            }
        }

        marks = moved
        segments = movedSegments
        markUndo = []
        markRedo = []

        run(command)
        if lost > 0 {
            notice = "\(lost) mark\(lost == 1 ? "" : "s") removed with the page."
        }
    }

    func rotatePage(_ index: Int) {
        guard let document else { return }
        let current = max(0, pagify_get_page_rotation(document.handle, Int32(index)))
        let quarter = Int((current + 1) % 4)
        // A page's marks are in that page's coordinates, and turning the sheet
        // turns them with it. Measured before the turn, because afterwards the
        // page reports its new size.
        if let size = try? document.pageSize(index), var records = marks[index] {
            for position in records.indices {
                records[position] = MarkRecord(
                    annotation: turn(records[position].annotation, in: size),
                    engineIndex: records[position].engineIndex)
            }
            marks[index] = records
        }
        segments[index] = nil
        markUndo = []
        markRedo = []
        run(.setPageRotation(index: index, quarterTurns: quarter))
    }

    /// One quarter turn clockwise, in page points: `(x, y)` becomes
    /// `(height - y, x)`.
    private func turn(_ annotation: WireAnnotation, in size: CGSize) -> WireAnnotation {
        func point(_ p: CGPoint) -> CGPoint { CGPoint(x: size.height - p.y, y: p.x) }
        func box(_ r: PageRect) -> PageRect {
            PageRect(from: point(CGPoint(x: r.left, y: r.top)),
                     to: point(CGPoint(x: r.right, y: r.bottom)))
        }

        switch annotation {
        case .highlight(let rects, let colour):
            return .highlight(rects: rects.map(box), color: colour)
        case .ink(let strokes, let colour, let width):
            return .ink(strokes: strokes.map { $0.map(point) }, color: colour, width: width)
        case .note(let rect, let contents, let colour):
            return .note(rect: box(rect), contents: contents, color: colour)
        case .text(var mark):
            mark.path = mark.path.map(point)
            return .text(mark)
        }
    }

    private func run(_ command: PagifyCommand) {
        guard let document else { return }
        apply(changing: command.affectedPage) { try document.execute(command) }
    }

    private func apply(changing page: Int? = nil, _ body: () throws -> EditState) {
        do {
            editState = try body()
            // The engine's own state moved, so the rasters are stale — but only
            // the page that actually changed, when the command named one.
            if let page {
                pageRevisions[page, default: 0] += 1
            } else {
                pageContentRevision += 1
            }
            annotationRevision += 1
        } catch {
            failure = error.localizedDescription
        }
    }

    /// The app's marks changed, but nothing PDFium drew did.
    private func marksChanged() {
        annotationRevision += 1
    }

    // ------------------------------------------------------- recording --

    @Published private(set) var isRecording = false
    /// A finished recording, waiting to be shared. On Android the file comes off
    /// by cable; on iOS that is a much worse assumption, so it is offered.
    @Published var recordingToShare: URL?

    /// Start or stop the render timeline, returning what to tell the reader.
    @discardableResult
    func toggleRecording() -> String {
        if isRecording {
            isRecording = false
            guard let file = SessionRecorder.shared.stop(directory: SessionRecorder.directory) else {
                return "Nothing was recorded."
            }
            recordingToShare = file
            notice = "Saved \(file.lastPathComponent)"
            return notice ?? ""
        }

        SessionRecorder.shared.start(documentName: document?.name ?? "(none)",
                                     pageCount: document?.pageCount ?? 0,
                                     engineVersion: PagifyEngine.version)
        isRecording = true
        notice = "Recording"
        return "Recording"
    }

    // ------------------------------------------------------ import / export --

    /// Write chosen pages out as their own PDF, in the order they were chosen.
    func exportPages(_ indices: [Int], to destination: URL) {
        guard let document, !indices.isEmpty else { return }
        do {
            try document.exportPages(indices, to: destination)
            notice = "Exported \(indices.count) page\(indices.count == 1 ? "" : "s")."
        } catch {
            failure = error.localizedDescription
        }
    }

    /// Bring pages from another PDF into this one, after `at`.
    ///
    /// The source is opened, read and closed here: the pages travel as their own
    /// small PDF inside the command, so a redo does not depend on the file still
    /// being reachable.
    func importPages(from url: URL, indices: [Int]?, at: Int) {
        guard let document else { return }
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }

        do {
            let source = try PagifyDocument(path: url.path, name: url.lastPathComponent)
            let chosen = indices ?? Array(0..<source.pageCount)
            guard !chosen.isEmpty else { return }

            // Marks live on page indices, and an import renumbers everything from
            // `at` onward.
            let inserted = chosen.count
            var moved: [Int: [MarkRecord]] = [:]
            for page in marks.keys.sorted() {
                moved[page >= at ? page + inserted : page] = marks[page]
            }
            marks = moved
            segments = [:]
            markUndo = []
            markRedo = []

            editState = try document.importPages(from: source, indices: chosen, at: at)
            pageContentRevision += 1
            annotationRevision += 1
            notice = "Imported \(inserted) page\(inserted == 1 ? "" : "s")."
        } catch {
            failure = error.localizedDescription
        }
    }

    // ---------------------------------------------------------------- save --

    /// Write the document out beside the original, leaving that one untouched.
    func saveCopy() {
        // Everything in hand goes back into the page first, or it is not in the
        // file that gets written.
        settleLiftedText()

        guard let document, let source else { return }
        let copy = source.deletingLastPathComponent()
            .appendingPathComponent(source.deletingPathExtension().lastPathComponent + " copy.pdf")
        do {
            // A full copy rather than an incremental one: the point of a copy is a
            // standalone file, and an incremental save of a document whose base
            // bytes live elsewhere is not one.
            try document.save(to: copy, incremental: false)
            notice = "Saved \(copy.lastPathComponent)"
        } catch {
            failure = error.localizedDescription
        }
    }

    /// Write the file and report whether it is safe to leave.
    ///
    /// Deliberately not `save()`, which reopens the document and jumps back to
    /// the page so the reader can carry on — pointless when they are on their way
    /// out, and it costs a reload of a 149-page file on the way to the library.
    /// A failure keeps them here with the marks intact, exactly as Android does.
    func saveBeforeLeaving() -> Bool {
        settleLiftedText()
        guard let document, let source else { return true }
        do {
            try document.save(to: source)
            return true
        } catch {
            failure = error.localizedDescription
            return false
        }
    }

    func save() {
        // Everything in hand goes back into the page first, or it is not in the
        // file that gets written.
        settleLiftedText()

        guard let document, let source else { return }
        do {
            try document.save(to: source)

            // Reopened, not merely re-read. PDFium holds the descriptor the
            // document was opened from and reads objects out of it lazily for the
            // document's whole life — after the file underneath has been replaced,
            // that descriptor points at bytes that no longer describe this file.
            // Everything still works until a page nobody has looked at yet is
            // scrolled to, and then it does not.
            let page = currentPage
            let keptMarks = marks
            let keptSaved = marks.values.reduce(0) { $0 + $1.count }
            try load(url: source, name: source.lastPathComponent, scopedURL: nil)

            // The marks are the ones already carried across; reloading them from
            // the reopened file would read back every ink nib as the engine's
            // default, because `/BS` does not round-trip.
            marks = keptMarks
            savedMarkCount = keptSaved
            jumpTo(page)
            notice = "Saved."
        } catch {
            failure = error.localizedDescription
        }
    }

    // -------------------------------------------------------------- memory --

    /// `didReceiveMemoryWarning` is the low case: drop the cached rasters and keep
    /// the reader's document. Levels at or above 80 close documents outright,
    /// which is not what a foreground warning means.
    func trimMemory() {
        PagifyEngine.trimMemory(level: 40)
    }

    private func record(_ label: String, _ detail: String, _ passed: Bool) {
        checks.append(Check(label: label, detail: detail, passed: passed))
        // Also to stdout, so `devicectl device process launch --console` can read
        // them off a physical device — which cannot be screenshotted the way the
        // simulator can.
        print("[pagify] \(passed ? "ok " : "FAIL") \(label): \(detail)")
    }
}
