import SwiftUI

/// The reader. Android's `ui/reader/PdfReaderScreen.kt`.
struct ReaderView: View {
    @ObservedObject var model: ReaderModel
    @EnvironmentObject private var appSettings: AppSettingsStore

    @Environment(\.colorScheme) private var scheme
    @Environment(\.dismiss) private var dismiss
    @State private var showingOrganiser = false
    @State private var showingMetadata = false

    /// Page width as a multiple of the viewport's. Zooming resizes the pages
    /// rather than transforming them, so the engine re-renders at the new scale
    /// and the text stays sharp — and page-space hit-testing stays a division.
    /// Running product of an in-progress pinch, before the pinned view exists.
    ///
    /// The pinned view is a different view, so handing over ends the gesture this
    /// one is receiving — the rest of the pinch never arrives. Handing over on the
    /// first event would therefore let one tiny movement decide the whole zoom,
    /// and answer it with a fixed jump regardless of how far the fingers moved.
    @State private var pinchProgress: CGFloat = 1
    /// Cumulative magnification of the pinch in progress, so each event can be
    /// turned into the step since the last one.
    @State private var lastPinch: CGFloat = 1
    /// What the list last drew for the page being handed over, so the pinned view
    /// has pixels on its very first frame instead of flashing blank.
    @State private var handoverImage: CGImage?

    var body: some View {
        VStack(spacing: 0) {
            topBar

            switch model.phase {
            case .empty:
                EmptyState(onPickDocument: { dismiss() })
            case .loading:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            case .passwordRequired(let retry):
                PasswordPrompt(retry: retry) { model.unlock(with: $0) }
            case .failed(let detail):
                ReaderMessage(title: "Could not open this file",
                              detail: detail,
                              actionLabel: "Choose another") { dismiss() }
            case .ready where model.zoomedPage != nil:
                if let document = model.document, let page = model.zoomedPage {
                    ZoomedPageView(
                        document: document,
                        pageIndex: page,
                        initialZoom: model.zoomTarget,
                        initialFocus: model.zoomFocus,
                        basePageWidth: lastListPageWidth,
                        initialImage: handoverImage,
                        onZoomSettled: { _ in },
                        onTurnPage: { model.turnZoomedPage($0) },
                        onExit: { model.exitZoom() },
                        // Every tool the list has. Magnifying a page is exactly
                        // when someone wants to place a mark precisely, so the
                        // whole layer comes with it rather than a picture of it.
                        settings: model.settings,
                        committed: (model.marks[page] ?? []).map(\.annotation),
                        annotationRevision: model.annotationRevision,
                        contentRevision: model.contentRevision(page: page),
                        segments: model.segments[page] ?? [],
                        selectedText: model.settings.selectedTextId.map(Int32.init),
                        onCommit: { model.commit($0, page: page) },
                        onErase: { model.erase(at: $0, page: page) },
                        onEraseStart: { model.beginErase() },
                        onEraseEnd: { model.endErase() },
                        onPlaceText: { model.beginText(page: page, from: $0, to: $1) },
                        onSelectText: { model.selectText($0) },
                        onMoveText: { model.moveMark(id: $0, delta: $1) },
                        onEditText: { model.editText(id: $0) },
                        onHighlightMissed: { model.highlightMissed(page: page) },
                        onScrollBlocked: { model.scrollBlocked() },
                        twoFingersDown: model.twoFingersDown,
                        onRequestNote: { model.requestNote(page: page, at: $0) },
                        onOpenNote: { model.openNote(page: page, index: $0) },
                        onScaleText: { model.scaleSelectedText($0) },
                        selection: model.selection,
                        onSelectWord: { model.selectWord(page: page, at: $0) },
                        onMoveSelectionHandle: { isStart, at in
                            model.moveSelectionHandle(isStart: isStart, to: at)
                        },
                        onClearSelection: { model.clearSelection() },
                        selectedMark: model.settings.selectedMark
                            .flatMap { $0.page == page ? $0.index : nil },
                        onSelectMark: { model.selectMark($0, page: page) },
                        onMoveMark: { at, delta, done in
                            guard done else { return }
                            model.moveMark(at, on: page, by: delta)
                        },
                        onPageFrame: { zoomedPageFrame.frames = [page: $0] },
                        // The magnified page's pinch, told to the model, so a
                        // caption resize is held in the overlay and written to the
                        // document once — when the fingers lift.
                        onPinching: { raised in
                            if raised { model.beginPinch() } else { model.endPinch() }
                        })
                    // The snapshot tool, here too.
                    //
                    // It was applied only to the list, so with it armed a drag on
                    // a magnified page fell through to nothing: the annotation
                    // canvas stands down for `.snapshot` on purpose, and there was
                    // nothing above it to catch the drag. Magnifying a page is
                    // exactly when someone wants a piece of it, so the tool that
                    // takes one has to come along.
                    //
                    // The same overlay and the same render path as the list. Only
                    // the page's rectangle differs, and the page itself reports
                    // that — see `onPageFrame`.
                    .captureOverlay(active: model.settings.tool == .snapshot,
                                    lasso: model.settings.captureLasso) { drag, ring in
                        takeCapture(drag, ring: ring,
                                    viewportWidth: lastListPageWidth,
                                    frames: zoomedPageFrame.frames)
                    }
                }
            case .ready:
            HStack(spacing: 0) {
            if model.showThumbnails, let document = model.document, document.pageCount > 1 {
                ThumbnailRail(document: document,
                              revision: model.pageContentRevision,
                              currentPage: model.currentPage,
                              followTick: model.readerFollowTick,
                              onSelectPage: { model.jumpTo($0) })
                Divider()
            }

            GeometryReader { geometry in
                pages(width: geometry.size.width, viewportHeight: geometry.size.height)
            }
            }
            }
        }
        .background(PagifyColor.background(scheme))
        .navigationBarBackButtonHidden(true)
        .sheet(item: $capturedFile) { file in
            ShareSheet(items: [file.url])
        }
        .fullScreenCover(item: $capture) { taken in
            if let document = model.document {
                CaptureEditor(
                    capture: taken,
                    readerBackground: AnnotationColors.captureBackground,
                    render: { request in
                        await renderCapture(request, document: document)
                    },
                    export: { action, request, marks in
                        await exportCapture(action, request: request, marks: marks,
                                            document: document)
                    },
                    onDismiss: { capture = nil })
            }
        }
        .overlay {
            if askingToLeave {
                LeavePrompt(
                    onSave: {
                        askingToLeave = false
                        // Stays put if the write failed: the marks are still here
                        // and the message says why.
                        if model.saveBeforeLeaving() { dismiss() }
                    },
                    onSaveAs: {
                        askingToLeave = false
                        // The copy is written and the reader stays in the
                        // document, as on Android — between answering and the
                        // file being written there is a whole screen they can
                        // back out of, so leaving on their behalf is presumptuous.
                        model.saveCopy()
                    },
                    onExit: {
                        askingToLeave = false
                        dismiss()
                    },
                    onClose: { askingToLeave = false })
                .transition(.opacity)
            }
        }
        .animation(.easeOut(duration: 0.15), value: askingToLeave)
        .toolbar(.hidden, for: .navigationBar)
        .overlay(alignment: .top) { notice }
        // The bands float over the pages rather than insetting them: Android
        // draws them in a Box aligned to the bottom, and a page that shrank every
        // time a panel opened would reflow under the reader's finger.
        // Above the ribbon, not over the words. A menu at the finger covers the
        // text it belongs to, and a selection can run over several lines, so
        // there is no "beside it" that is not on top of something.
        .overlay(alignment: .bottom) {
            if let selection = model.selection, !selection.rects.isEmpty {
                TextSelectionBar(characters: selection.text.count,
                                 // The selection this bar was built for, handed
                                 // back — not whatever is live by the time the
                                 // finger lifts.
                                 onCopy: { model.copy(selection) },
                                 onHighlight: { model.highlight(selection) },
                                 onDismiss: { model.clearSelection() })
                    .padding(.bottom, ribbonHeight + 12)
                    .transition(.opacity)
            }
        }
        .overlay(alignment: .bottom) {
            if model.isEditable {
                ToolRibbon(settings: $model.settings,
                               onRestyle: { model.restyleSelected($0) },
                               // One undo step and one write for a whole slider
                               // drag, rather than one of each per frame.
                               onEditing: { down in
                                   if down { model.beginRestyle() } else { model.endRestyle() }
                               })
                    // How much of the reader it stands in front of.
                    //
                    // Floating over the pages is deliberate — see above — but it
                    // still hides what is behind it, and a page centred in the
                    // whole scroll view puts its lower half under here. Measured
                    // rather than assumed: the bands change height when a colour
                    // strip opens.
                    .background {
                        GeometryReader { bar in
                            Color.clear.preference(key: ToolRibbonHeightKey.self,
                                                   value: bar.size.height)
                        }
                    }
            }
        }
        .onPreferenceChange(ToolRibbonHeightKey.self) { height in
            if height != ribbonHeight { ribbonHeight = height }
        }
        .sheet(isPresented: $showingOrganiser) {
            if let document = model.document {
                PageOrganiser(document: document, revision: model.pageContentRevision, model: model)
            }
        }
        .sheet(item: Binding(get: { model.recordingToShare.map(ShareableFile.init) },
                             set: { if $0 == nil { model.recordingToShare = nil } })) { item in
            ShareSheet(items: [item.url])
        }
        .sheet(isPresented: Binding(get: { model.pendingNote != nil },
                                    set: { if !$0 { model.pendingNote = nil } })) {
            NoteSheet(existing: nil) { model.commitNote($0) }
                .presentationDetents([.medium])
        }
        .sheet(isPresented: Binding(get: { model.openNote != nil },
                                    set: { if !$0 { model.openNote = nil } })) {
            if let note = model.openNote {
                NoteSheet(existing: note.contents) { _ in model.openNote = nil }
                    .presentationDetents([.medium])
            }
        }
        .sheet(isPresented: Binding(get: { model.editingText != nil },
                                    set: { if !$0 { model.editingText = nil } })) {
            if let editing = model.editingText {
                TextEditorSheet(font: editing.mark.font,
                                size: editing.mark.size,
                                color: editing.mark.color,
                                initial: editing.mark.text) { words, lines in
                    model.commitEdit(words, lines: lines)
                }
                .presentationDetents([.medium])
            }
        }
        .sheet(isPresented: Binding(get: { model.placingText != nil },
                                    set: { if !$0 { model.placingText = nil } })) {
            TextEditorSheet(font: model.settings.font,
                            size: model.settings.textSize,
                            color: model.settings.penColor) { words, lines in
                model.commitText(words, lines: lines)
            }
            .presentationDetents([.medium])
        }
        .sheet(isPresented: $showingMetadata) {
            if let document = model.document {
                MetadataSheet(document: document)
            }
        }
        .onReceive(NotificationCenter.default.publisher(
            for: UIApplication.didReceiveMemoryWarningNotification)
        ) { _ in
            model.trimMemory()
        }
    }

    /// The middle of each page, in the reader's own coordinate space.
    /// The reader's own visible height.
    private /// The measured height of the floating tool ribbon.
struct ToolRibbonHeightKey: PreferenceKey {
    static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

struct ReaderViewportKey: PreferenceKey {
        static var defaultValue: CGFloat { 0 }
        static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
            value = max(value, nextValue())
        }
    }

    /// The gap around and between pages.
    private let pageGap: CGFloat = 12

    /// How tall the whole document is at this width, before the zoom transform.
    ///
    /// Computed from the page shapes rather than measured, because a `LazyVStack`
    /// only measures the rows it has built — asking it how tall the document is
    /// gets the height of the handful on screen.

    /// One row of the list.
    ///
    /// Extracted because the call is large enough that the type checker gives
    /// up on it inline — and it grows every time the reader learns something.
    @ViewBuilder
    private func pageRow(_ index: Int, document: PagifyDocument, width: CGFloat) -> some View {
                                PageView(document: document,
                                         index: index,
                                         width: width,
                                         zoom: 1,
                                         onAspect: { model.noteAspect($0, page: index) },
                                         revision: model.contentRevision(page: index),
                                         annotationRevision: model.annotationRevision,
                                         settings: model.settings,
                                         committed: (model.marks[index] ?? []).map(\.annotation),
                                         onCommit: { model.commit($0, page: index) },
                                         onErase: { model.erase(at: $0, page: index) },
                                         onEraseStart: { model.beginErase() },
                                         onEraseEnd: { model.endErase() },
                                         onPlaceText: { model.beginText(page: index, from: $0, to: $1) },
                                         selectedText: model.settings.selectedTextId.map(Int32.init),
                                         onSelectText: { model.selectText($0) },
                                         onMoveText: { model.moveMark(id: $0, delta: $1) },
                                         onEditText: { model.editText(id: $0) },
                                         segments: model.segments[index] ?? [],
                                         onHighlightMissed: { model.highlightMissed(page: index) },
                                         onScrollBlocked: { model.scrollBlocked() },
                                         twoFingersDown: model.twoFingersDown,
                                         onRequestNote: { model.requestNote(page: index, at: $0) },
                                         onOpenNote: { model.openNote(page: index, index: $0) },
                                         onAppearPage: {
                                             model.loadMarks(page: index)
                                             model.loadSegments(page: index)
                                         },
                                         selection: model.selection,
                                         onSelectWord: { model.selectWord(page: index, at: $0) },
                                         onMoveSelectionHandle: { isStart, at in
                                             model.moveSelectionHandle(isStart: isStart, to: at)
                                         },
                                         onClearSelection: { model.clearSelection() },
                                         selectedMark: model.settings.selectedMark
                                             .flatMap { $0.page == index ? $0.index : nil },
                                         onSelectMark: { model.selectMark($0, page: index) },
                                         onMoveMark: { at, delta, done in
                                             // Written only when the finger lifts.
                                             // Every frame of a drag would be an
                                             // undo step, and a re-render of the
                                             // page under the finger.
                                             guard done else { return }
                                             model.moveMark(at, on: index, by: delta)
                                         },
                                         pageSize: model.pageSize(index))
                                    .id(index)
    }

    /// Work out where every page now sits, and which one is being read.
    ///
    /// Called from the scroll observer, outside SwiftUI's update cycle. It writes
    /// only the frame box — which invalidates nothing — and hands the page to the
    /// model, which publishes only when it has actually changed.
    private func readerScrolled(toContentY offset: CGFloat,
                                viewportWidth: CGFloat,
                                viewportHeight: CGFloat) {
        guard let document = model.document, document.pageCount > 0 else { return }
        let width = viewportWidth - pageGap * 2
        guard width > 0 else { return }

        let extent = readerHeight > 0 ? readerHeight : viewportHeight
        var frames: [Int: CGRect] = [:]
        var chosen: Int?
        var nearest: (page: Int, distance: CGFloat)?
        var firstBuilt: Int?
        var lastBuilt = 0
        let middle = extent / 2
        let tops = tops(width: width, count: document.pageCount)

        for page in 0..<document.pageCount {
            let height = rowHeight(page, width: width)
            let y = tops[page] - offset

            // A screenful of margin either side, so a flick arrives at pages that
            // are already drawn instead of at grey rectangles.
            if y < extent * 2 && y + height > -extent {
                if firstBuilt == nil { firstBuilt = page }
                lastBuilt = page
            }
            // Only what is on screen, or near enough to be scrolled onto it. The
            // zoom needs the frames of pages a finger could actually land on.
            if y < extent && y + height > 0 {
                frames[page] = CGRect(x: pageGap, y: y, width: width, height: height)
                let distance = abs(y + height / 2 - middle)
                if nearest == nil || distance < nearest!.distance {
                    nearest = (page, distance)
                }
            }
        }

        // Nothing overrides this any more.
        //
        // The pages used to be asked where they were, because the sum above was
        // right near the front of a document and wrong by dozens of pages near
        // the back — a lazy stack holding estimated space for rows it had not
        // built. The board holds no estimates: every page is drawn at exactly
        // `tops[page]`, so the sum *is* the position, and asking the rows cost a
        // `GeometryReader` and a preference in every one of them, on every scroll
        // frame, to be told what was already known.
        pageFramesBox.frames = frames

        if let firstBuilt {
            let wanted = firstBuilt...max(firstBuilt, lastBuilt)
            if wantedWindow.range != wanted {
                wantedWindow.range = wanted
                // Off this turn of the run loop on purpose. The observer's first
                // report comes from `didMoveToWindow`, which UIKit runs inside
                // SwiftUI's own layout pass — writing view state there is a write
                // during an update, and this write changes the row count, which
                // changes the content size, which reports another offset. The hop
                // breaks that re-entrancy, and the box carries the *latest* want,
                // so a hop that lands late cannot install a stale window.
                DispatchQueue.main.async {
                    guard let want = wantedWindow.range, want != buildWindow else { return }
                    buildWindow = want
                }
            }
        }

        guard let nearest else { return }

        // Both ends get an explicit case, the same way they always did: at the top
        // a first page taller than the viewport has its middle *below* the
        // viewport's, so the page after it scores closer; at the bottom the last
        // page can fill the screen while the page starting highest is still the
        // one before it.
        let last = document.pageCount - 1
        if let first = frames[0], first.minY >= -1 {
            chosen = frames.keys.min()
        } else if let final = frames[last], final.maxY <= extent + 1 {
            chosen = frames.keys.max()
        } else {
            chosen = nearest.page
        }

        SessionRecorder.shared.record("PAGE_ENTER",
            String(format: "in-view page=%d visible=%d extent=%.0f offset=%.0f",
                   chosen ?? -1, frames.count, extent, offset))

        // Only while the reader is not being scrolled *to* somewhere: during a
        // jump every page sweeps past the middle, and publishing each one fights
        // the jump it is part of.
        guard !model.jumpRequested, let chosen else { return }
        model.readerScrolled(to: chosen)
    }

    /// Turn a drag into a picture, and open the editor on it.
    private func takeCapture(_ drag: PageRect, ring: [CGPoint], viewportWidth: CGFloat,
                             frames: [Int: CGRect]? = nil) {
        guard let document = model.document else { return }

        // Every page the drag touched, with where it sits and how big it is in its
        // own points — the frames are already known, so nothing is measured here.
        let placed = (frames ?? pageFramesBox.frames).map { page, frame in
            PlacedPage(pageIndex: page,
                       bounds: PageRect(left: frame.minX, top: frame.minY,
                                        right: frame.maxX, bottom: frame.maxY),
                       sizePoints: model.pageSize(page))
        }

        let tiles = captureTiles(for: drag, pages: placed)
        guard !tiles.isEmpty else {
            notice("Nothing to capture there.")
            return
        }

        var request = CaptureRequest(
            tiles: tiles,
            width: drag.right - drag.left,
            height: drag.bottom - drag.top,
            background: AnnotationColors.captureBackground,
            originPage: tiles.map(\.pageIndex).min() ?? model.currentPage)
        request.mask = ring.isEmpty ? [] : captureMask(for: drag, outline: ring)
        // The reader's own preferences, so a capture comes out at the sharpness
        // and in the format the settings screen promised.
        request.scale = appSettings.settings.captureScale
        request.format = appSettings.settings.captureFormat

        Task { @MainActor in
            guard let preview = await renderCapture(request, document: document) else {
                notice("That capture could not be drawn.")
                return
            }
            capture = preview
            // The tool is put down with the picture taken: Android does the same,
            // and an armed marquee over the editor's own canvas is nonsense.
            model.settings.select(.none)
        }
    }

    /// Draw a request, off the main thread, and wrap it for the editor.
    private func renderCapture(_ request: CaptureRequest,
                               document: PagifyDocument,
                               markup: [Markup] = []) async -> CapturePreview? {
        let format = request.format
        let scale = request.scale
        let bytes = await Task.detached(priority: .userInitiated) { () -> Data? in
            try? document.captureRegion(request, markup: markup, format: format, scale: scale)
        }.value

        guard let bytes,
              let source = CGImageSourceCreateWithData(bytes as CFData, nil),
              let picture = CGImageSourceCreateImageAtIndex(source, 0, nil) else { return nil }

        return CapturePreview(
            request: request,
            bytes: bytes,
            fileName: captureFileName(documentName: model.document?.name ?? "Pagify",
                                      pageIndex: request.originPage,
                                      format: format,
                                      timestamp: captureTimestamp()),
            picture: picture)
    }

    /// Hand the finished picture — marks drawn in by the engine — to Files, a
    /// share sheet or the pasteboard.
    private func exportCapture(_ action: CaptureExportAction,
                               request: CaptureRequest,
                               marks: [Markup],
                               document: PagifyDocument) async {
        guard let preview = await renderCapture(request, document: document, markup: marks) else {
            notice("That capture could not be drawn.")
            return
        }

        switch action {
        case .copy:
            UIPasteboard.general.setData(preview.bytes,
                                         forPasteboardType: request.format.pasteboardType)
            notice("Copied.")
        case .save, .share:
            // Written to a real file first: both a share sheet and Files hand over
            // a URL, and a picture that exists only in memory has none.
            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent(preview.fileName)
            do {
                try preview.bytes.write(to: url, options: .atomic)
                capturedFile = CapturedFile(url: url)
            } catch {
                model.failure = error.localizedDescription
            }
        }
    }

    private func captureTimestamp() -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd HHmmss"
        return formatter.string(from: Date())
    }

    private func notice(_ text: String) {
        model.noticeIsWarning = false
        model.notice = text
    }

    /// Where a page begins, in content coordinates.
    /// The part of the reader a page can actually be centred in.
    ///
    /// The scroll view runs to the bottom of the screen; the tool ribbon floats
    /// in front of its last inch. Centring in the full height therefore puts the
    /// chosen page's lower half behind the ribbon — which is what "the page is at
    /// the bottom" looks like — so the ribbon comes off the extent first.
    private func usableExtent(fallback: CGFloat) -> CGFloat {
        let full = readerHeight > 0 ? readerHeight : fallback
        return max(1, full - ribbonHeight)
    }

    /// Where a page begins, in content coordinates. `page == pageCount` answers
    /// the height of the whole document — the top gap, every page, and the gap
    /// under each.
    private func contentTop(of page: Int, width: CGFloat) -> CGFloat? {
        guard let document = model.document, page >= 0, page <= document.pageCount,
              width > 0 else { return nil }
        return tops(width: width, count: document.pageCount)[page]
    }

    /// The sum, cached.
    ///
    /// Two answers to one question is how a reserved height and a set of page
    /// positions come to disagree — which is what the old `contentHeight` would
    /// have done to whoever wired it up, summing `model.aspect(page:)` against a
    /// `contentTop` summing `model.pageSize`. There is one answer now, and it is
    /// this array.
    private func tops(width: CGFloat, count: Int) -> [CGFloat] {
        if pageTops.width == width, pageTops.count == count { return pageTops.tops }
        var out: [CGFloat] = []
        out.reserveCapacity(count + 1)
        var top = pageGap
        for index in 0..<count {
            out.append(top)
            top += rowHeight(index, width: width) + pageGap
        }
        out.append(top)
        pageTops.width = width
        pageTops.count = count
        pageTops.tops = out
        return out
    }


    /// Hand the reader over to the pinned page view.
    ///
    /// The focus is converted to a fraction of the page under the fingers, so the
    /// magnified view opens centred on whatever was being looked at rather than
    /// on the middle of the document.
    private func handOver(to position: CGPoint, target: CGFloat, viewport: CGSize) {
        guard let document = model.document, document.pageCount > 0 else {
            SessionRecorder.shared.record("ZOOM_ENTER", "refused reason=no-document")
            return
        }

        // The page the touch actually landed on, and where within it — not
        // whichever page happens to be nearest the middle of the reader. Using
        // the middle is why the first and last page could never be zoomed into:
        // neither is ever the nearest to the centre when the list is at one of
        // its ends.
        guard let hit = pageFramesBox.frames.first(where: {
            position.y >= $0.value.minY && position.y < $0.value.maxY
        }), hit.value.height > 0, viewport.width > 0 else {
            SessionRecorder.shared.record("ZOOM_ENTER",
                String(format: "refused reason=no-page-at y=%.0f frames=%d",
                       position.y, pageFramesBox.frames.count))
            return
        }

        let page = min(max(hit.key, 0), document.pageCount - 1)
        // Handed over as the initial recentre, so the zoom lands on what was
        // touched instead of the page's top-left corner.
        let fraction = CGPoint(
            x: min(max(position.x / viewport.width, 0), 1),
            y: min(max((position.y - hit.value.minY) / hit.value.height, 0), 1))
        SessionRecorder.shared.record("ZOOM_ENTER",
           String(format: "page=%d target=%.2f at=%.0f,%.0f base=%.0f",
                  page, target, position.x, position.y, lastListPageWidth))
        model.enterZoom(page: page, focus: fraction, target: target)
    }

    /// The width the list last laid a page out at — what scale 1.0 must mean in
    /// the pinned view, so handing over is not a visible jump. The pinned view
    /// replaces the rail too, so its own viewport is wider than this.
    @State private var lastListPageWidth: CGFloat = 0
    /// Where each page sits in the reader, so a touch can be matched to one.
    /// Where every built page currently sits, for the zoom hand-over.
    ///
    /// Written **only when it changes**, and that guard is the whole point.
    ///
    /// SwiftUI calls `onPreferenceChange` on every render pass here, not only when
    /// the value differs. Assigning unconditionally therefore re-ran the body,
    /// which re-measured the pages, which called the closure again — a loop that
    /// ran at sixty frames a second for the entire life of the reader. A recording
    /// off the phone showed 479 byte-identical `in-view` scans in 12.7 seconds,
    /// one every 16ms, while nothing on screen moved. That churn is what made the
    /// reader crawl, what fought a scroll and snapped it back, and what rebuilt
    /// the gesture layers often enough to leave five pinch recognisers stacked on
    /// one page, none of which ever saw a second finger.
    ///
    /// Taking the frames out of view state altogether stops the loop — and stops
    /// the tracking with it, because that render is the only thing that keeps the
    /// geometry live as the list scrolls. The equality check keeps both: a real
    /// scroll moves the frames and publishes, a still list writes nothing.
    /// A capture that has been taken and is waiting to be marked up.
    @State private var capture: CapturePreview?
    /// A finished picture on its way to Files or a share sheet.
    @State private var capturedFile: CapturedFile?
    /// The way out is being asked about.
    @State private var askingToLeave = false
    /// Where each page sits, computed from the scroll offset and the page sizes
    /// the engine already knows — **not** measured.
    ///
    /// Held in a reference so that learning it costs nothing: writing it into
    /// view state re-ran the body, which rebuilt every visible row, forty-two
    /// times a second through a scroll. Nothing reads this during layout; the
    /// zoom hand-over asks for it at the moment two fingers land.
    @State private var pageFramesBox = PageFrameBox()
    /// The reader's scroll view, so a jump lands exactly where it was asked to.
    @State private var commander = ScrollCommander()
    /// The scroll viewport's height, measured rather than inherited.
    @State private var readerHeight: CGFloat = 0
    /// The height of the floating tool ribbon, which stands in front of the
    /// bottom of the reader and of the rail.
    @State private var ribbonHeight: CGFloat = 0
    /// Where the magnified page is drawn, so a capture there knows what it hit.
    /// A reference, like the list's own frames: written on every frame of a pinch
    /// and read only when a drag ends.
    @State private var zoomedPageFrame = PageFrameBox()
    /// Which pages exist in the board, in page indices. Written from the scroll
    /// observer, and only when it actually changes — a window recomputed per
    /// frame would rebuild the list sixty times a second.
    @State private var buildWindow: ClosedRange<Int> = 0...0
    /// The window the latest scroll wants, before it reaches `buildWindow`.
    @State private var wantedWindow = WindowBox()
    /// Every page's top, summed once per width and page count.
    @State private var pageTops = PageTopsCache()

    /// Two fingers dragging the list.
    ///
    /// SwiftUI offers no programmatic scroll by an offset, so this moves by the
    /// page the drag has carried the reader onto — coarser than Android's
    /// pixel-for-pixel pan, and the one place this is knowingly not a faithful
    /// port.
    /// Two fingers scroll the list, point for point.
    ///
    /// It used to turn *pages*: the travel was accumulated and, once it passed a
    /// third of the viewport, `jumpTo` moved to the next page and reset. That is
    /// not scrolling — it ignored two thirds of every drag, then leapt — and with
    /// a tool armed it is the only way through, which is what the scroll hint now
    /// points people at. A hint that names a gesture has to name one that works.
    ///
    /// Android pans the list by the delta (`.twoFingerPan`), and so does this:
    /// the scroll view is right there, and moving its offset is what the fingers
    /// asked for.
    private func twoFingerScroll(by delta: CGFloat, viewportHeight: CGFloat) {
        guard delta != 0, let scroll = commander.scrollView else { return }
        let limit = max(0, scroll.contentSize.height - scroll.bounds.height)
        // Down-drag carries the content down, which is a smaller offset.
        let target = min(max(scroll.contentOffset.y - delta, 0), limit)
        guard abs(target - scroll.contentOffset.y) > 0.01 else { return }
        scroll.setContentOffset(CGPoint(x: scroll.contentOffset.x, y: target),
                                animated: false)
    }

    /// The pages, placed where the arithmetic says they are rather than where a
    /// lazy stack has got round to putting them.
    ///
    /// **This is what was moving.** A `LazyVStack` reserves *estimated* space for
    /// the rows it has not built — one recording had it guessing 40,549 points of
    /// content where the built rows add up to 31,024 — and it revises those
    /// estimates continuously as rows build and fall away. A SwiftUI `ScrollView`
    /// is anchored to its content *offset*, not to an item, so every revision
    /// above the viewport slides everything below it while `contentOffset` holds
    /// perfectly still. Nothing commands a scroll, so nothing appears in a
    /// recording: the page simply is not where it was a frame ago. That is why
    /// fixing the jump-settle loop and the reorder hold changed nothing — neither
    /// was ever involved.
    ///
    /// Compose's `LazyColumn` anchors on `firstVisibleItemIndex`, which is why the
    /// same list holds still on Android and why this needed no answer there.
    ///
    /// So the stack goes. Every page's height is known from the engine before a
    /// single row is built, so the content is given its true height once and each
    /// page is drawn at its own `contentTop` — absolute, the way the organiser
    /// draws a page under a finger. Neither number can change afterwards, so a row
    /// building or leaving moves nothing at all.
    @ViewBuilder
    private func pageBoard(width: CGFloat, viewportWidth: CGFloat) -> some View {
        if let document = model.document, document.pageCount > 0 {
            ZStack(alignment: .topLeading) {
                // The whole document's height, reserved up front. The top of the
                // page *after* the last one is exactly that: the top gap, every
                // page, and the gap under each. It also gives the content its
                // width, which the rows, inset by `pageGap`, do not supply.
                Color.clear
                    .frame(width: viewportWidth,
                           height: contentTop(of: document.pageCount, width: width) ?? 0)

                // Only the rows near the viewport are built, which is all the
                // stack was ever wanted for. `.offset` rather than layout, so a
                // row's position is a constant and not a running sum.
                ForEach(builtPages(pageCount: document.pageCount), id: \.self) { index in
                    pageRow(index, document: document, width: width)
                        .frame(width: width, height: rowHeight(index, width: width),
                               alignment: .top)
                        .offset(x: pageGap, y: contentTop(of: index, width: width) ?? 0)
                }
            }
        }
    }

    /// How tall page `index` is drawn at this width. The engine answers this
    /// before the page is rasterised, and `PageView.displayedSize` computes the
    /// identical number — so a row exactly fills the space reserved for it
    /// whether or not its raster has arrived.
    private func rowHeight(_ index: Int, width: CGFloat) -> CGFloat {
        let size = model.pageSize(index)
        return size.width > 0 ? width * size.height / size.width : width
    }

    /// Clamped rather than trusted: a delete or an import can shorten the
    /// document between the scroll that chose this window and the body that
    /// reads it.
    private func builtPages(pageCount: Int) -> [Int] {
        guard pageCount > 0 else { return [] }
        let last = pageCount - 1
        let low = min(max(buildWindow.lowerBound, 0), last)
        let high = min(max(buildWindow.upperBound, low), last)
        return Array(low...high)
    }

    /// Put the list back on the page the magnified view was showing.
    ///
    /// Retried because `commander.scrollView` is found by the observer's
    /// `didMoveToWindow`, which is not ordered against `onAppear`. A restore that
    /// never lands drops the reader at the top of the document, which is exactly
    /// what this exists to prevent — so it is worth waiting for, and a wait that
    /// never ends is worse, so it is bounded and says so.
    private func restore(to page: Int, viewportWidth: CGFloat, viewportHeight: CGFloat,
                         attempt: Int = 0) {
        let width = viewportWidth - pageGap * 2
        guard let top = contentTop(of: page, width: width) else { return }
        guard commander.scrollView != nil else {
            guard attempt < 12 else {
                SessionRecorder.shared.record("NAVIGATION",
                    String(format: "restore lost page=%d reason=no-scroll-view", page))
                return
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                restore(to: page, viewportWidth: viewportWidth,
                        viewportHeight: viewportHeight, attempt: attempt + 1)
            }
            return
        }
        let extent = usableExtent(fallback: viewportHeight)
        let target = top - max(0, (extent - rowHeight(page, width: width)) / 2)
        SessionRecorder.shared.record("NAVIGATION",
            String(format: "restore page=%d target=%.0f attempt=%d", page, target, attempt))
        commander.scroll(toContentY: target, animated: false)
    }

    private func pages(width viewportWidth: CGFloat, viewportHeight: CGFloat) -> some View {
        // The list is never scaled. A pinch here accumulates until it is
        // unambiguous and then hands the reader over to the pinned page view —
        // scaling the list instead re-measured every row on every gesture event,
        // which is why the reader jumped back towards page one.
        let pageWidth = viewportWidth

        return ScrollViewReader { proxy in
            ScrollView([.vertical, .horizontal]) {
                pageBoard(width: pageWidth - pageGap * 2, viewportWidth: viewportWidth)
                .onAppear { lastListPageWidth = pageWidth - pageGap * 2 }
                .onChange(of: pageWidth) { _, width in
                    lastListPageWidth = width - pageGap * 2
                }
                .frame(width: viewportWidth, alignment: .top)
                // Inside the content on purpose. A `.background` on the scroll
                // view itself is laid out beside it, not within it, so walking up
                // from there never reaches the scroll view and the offset was
                // never heard.
                .background {
                    ScrollObserver(onScroll: { offset in
                        readerScrolled(toContentY: offset,
                                       viewportWidth: viewportWidth,
                                       viewportHeight: viewportHeight)
                    }, onFound: { commander.scrollView = $0 })
                }
            }
            // SwiftUI's own pinch, attached to the **scroll view** rather than to
            // its content.
            //
            // Two reasons for both halves of that. A UIKit recogniser hosted in an
            // overlay was tried and could not be made to receive touches reliably;
            // this one is known to fire. And the location must be in the same space
            // as the page frames, which are recorded against the scroll view — a
            // gesture on the content reports in content space, which differs by the
            // scroll offset, so the page lookup only ever succeeded at the top of
            // the document.
            //
            // The anchor is where the pinch *began* rather than the live midpoint
            // between the fingers, which is what Android uses. That is a real
            // difference and it is here because SwiftUI does not report the live
            // centroid at all.
            .simultaneousGesture(
                MagnifyGesture()
                    .onChanged { value in
                        guard model.settings.selectedTextId == nil else {
                            // A caption in hand is being resized from the first
                            // event, so the pinch is real straight away.
                            model.beginPinch()
                            model.scaleSelectedText(value.magnification / max(lastPinch, 0.0001))
                            lastPinch = value.magnification
                            return
                        }
                        let factor = value.magnification / max(lastPinch, 0.0001)
                        lastPinch = value.magnification
                        pinchProgress = pinchProgressAfter(pinchProgress, factor)
                        SessionRecorder.shared.record("ZOOM_TOUCH",
                            String(format: "list factor=%.3f progress=%.3f at=%.0f,%.0f",
                                   factor, pinchProgress,
                                   value.startLocation.x, value.startLocation.y))

                        // Not raised on the way here at all — only at the
                        // hand-over, below.
                        //
                        // `isPinching` does two jobs: it defers caption writes
                        // during a resize, which is the branch above and is
                        // untouched, and it stands the two-finger pan down. For a
                        // page pinch only the second applies, and standing the pan
                        // down before a zoom is going to happen buys nothing and
                        // costs the scroll.
                        //
                        // Two thresholds were tried and both were wrong, because
                        // the premise was: a recording of an ordinary two-finger
                        // scroll has `progress` wandering to 0.836 and back, a
                        // quarter of its samples past six per cent. Fingers
                        // dragging together really do change their separation that
                        // much. There is no spread small enough to mean "not a
                        // pinch" and large enough to catch one — but there is a
                        // moment that is unambiguous, and it is this one.
                        guard pinchProgress >= Zoom.pinchHandover else { return }
                        model.beginPinch()
                        // The gesture ends here, not at `.onEnded`: handing over
                        // replaces this whole view, so the end callback never
                        // arrives and `isPinching` stayed true for the rest of the
                        // session. Every caption drag was cancelled the moment it
                        // began, and every caption restyle was deferred to a pinch
                        // that had already finished — which is why moving and
                        // resizing text stopped working after one zoom.
                        model.endPinch()
                        // Clamped only here: pinching *out* at fit-width has
                        // nowhere to go and must not hand over below the size
                        // already on screen.
                        handOver(to: value.startLocation, target: max(pinchProgress, 1),
                                 viewport: CGSize(width: viewportWidth, height: viewportHeight))
                        pinchProgress = 1
                        lastPinch = 1
                    }
                    .onEnded { _ in
                        model.endPinch()
                        pinchProgress = 1
                        lastPinch = 1
                    }
            )
            // The other way in. Double tap is the gesture every reader has, and
            // the list had none.
            .simultaneousGesture(
                SpatialTapGesture(count: 2, coordinateSpace: .named("reader"))
                    .onEnded { tap in
                        guard model.settings.selectedTextId == nil else { return }
                        SessionRecorder.shared.record("ZOOM_DTAP_LIST",
                            String(format: "at=%.0f,%.0f", tap.location.x, tap.location.y))
                        // Opened showing the whole page, not magnified.
                        //
                        // Android enters at `DOUBLE_TAP_ZOOM` (2.5x) from the
                        // list; this does not. A double tap here is how you get
                        // *into* the magnified view at all, and arriving already
                        // deep inside a page hides what you were pointing at.
                        // Once in, double tap still toggles fit and 2.5x, which
                        // is Android's behaviour and is left alone.
                        handOver(to: tap.location, target: Zoom.fitWidth,
                                 viewport: CGSize(width: viewportWidth, height: viewportHeight))
                    }
            )
            // Words selected, or a mark in hand, take the scroller too.
            //
            // Refusing to run *alongside* the scroll view is not the same as
            // stopping it: our own recogniser stands down, and the scroller
            // carries on with the finger. So a mark being dragged moved the page
            // instead of the mark. Compose can consume the event inside the page
            // before the list sees it; SwiftUI has no such interception point, so
            // the scroller is switched off for as long as something is in hand.
            //
            // Compose can consume a drag inside the page before the enclosing
            // list sees it, which is how Android stops the pages moving while a
            // selection is being adjusted (`change.consume()` in its drag
            // handler). SwiftUI has no such interception point, so the same end
            // has to be reached by switching the scroller off — otherwise the
            // list wins the drag and the selection never grows.
            //
            // The way out is a tap, armed on every page for exactly this reason,
            // and Copy and Highlight both put the selection down themselves.
            .scrollDisabled(model.settings.tool != .none
                            || model.selection != nil
                            || model.settings.selectedMark != nil)
            // Picking a tool up arms the scroll hint again — see `toolChanged`.
            .onChange(of: model.settings.tool) { _, _ in model.toolChanged() }
            // An import or a delete changes where every page after it sits
            // without moving `contentOffset` by a point — so no scroll is
            // reported, and the window would be left pointing at pages that have
            // moved out from under it. Asked again from wherever the list is.
            .onChange(of: model.document?.pageCount ?? 0) { _, _ in
                guard let scroll = commander.scrollView else { return }
                readerScrolled(toContentY: scroll.contentOffset.y,
                               viewportWidth: viewportWidth,
                               viewportHeight: viewportHeight)
            }
            // Fitted whether or not a tool is live. With one armed the list's own
            // scrolling is off, because a one-finger drag has to mean "draw"; and
            // with none armed the pinch claims every two-finger event, so the
            // scroll view never sees one either. Both cases need this, which is
            // why Android fits it unconditionally — fitting it only when a tool
            // was active left two fingers doing nothing the rest of the time.
            .overlay {
                TwoFingerPanLayer(onPan: { delta in
                    // Nothing while a caption is in hand: the two fingers
                    // resizing it must not also scroll the document out from
                    // under it. And nothing while a pinch is running: this layer
                    // only started receiving touches at all once the gesture host
                    // was fixed, and left ungated it scrolls the page away under
                    // a zoom.
                    guard model.settings.selectedTextId == nil,
                          !model.isPinching else { return }
                    twoFingerScroll(by: delta.height, viewportHeight: viewportHeight)
                }, onTwoFingers: { down in
                    // Told to the model so the drawing layer inside each page can
                    // abandon the stroke the first finger began.
                    if model.twoFingersDown != down { model.twoFingersDown = down }
                })
            }
            // Above the whole list, not inside a page: a capture routinely spans
            // the bottom of one page, the gap between them, and the top of the
            // next, and inside one page's layer the drag would be clipped to it.
            .captureOverlay(active: model.settings.tool == .snapshot,
                            lasso: model.settings.captureLasso) { drag, ring in
                takeCapture(drag, ring: ring, viewportWidth: viewportWidth)
            }
            // Heard by every caption layer below, however deep.
            .environment(\.isPinching, model.isPinching)
            .coordinateSpace(name: "reader")
            // The reader's own height, measured in its own coordinate space.
            // The height handed down by the enclosing geometry is not the same
            // number, and testing "the last page's bottom is at or above the
            // viewport's" against the container's meant that test could never be
            // true, and the page at the end of a document stayed one short.
            //
            // This is the scroll view's full height, and the tool ribbon floats
            // in front of its last inch — see `usableExtent`, which is what
            // anything centring against the reader must use instead.
            .background {
                GeometryReader { scroll in
                    Color.clear.preference(key: ReaderViewportKey.self,
                                           value: scroll.size.height)
                }
            }
            .onPreferenceChange(ReaderViewportKey.self) { height in
                // Compared before writing: an unchanged height written back into
                // state is another turn of the same loop.
                if height > 0, height != readerHeight { readerHeight = height }
            }
            // Nothing is measured here any more. The scroll observer below reports
            // the offset, and where every page sits follows from that and the page
            // sizes — which the engine can answer before a single row is built.
            .onChange(of: model.currentPage) { _, page in
                // Only when something asked to go there: following every page
                // that scrolls past would fight the reader's own scrolling. The
                // settle window is the model's — clearing it here meant a jump
                // onto the page already showing never cleared it at all.
                guard model.jumpRequested else { return }
                // Scrolled by arithmetic, not by asking for a row.
                //
                // `proxy.scrollTo` has to estimate where an unbuilt row of a lazy
                // stack sits, and a hundred pages in that estimate is nowhere
                // near — choosing a thumbnail deep in a long document landed on a
                // different page entirely. Every page's position is already known,
                // so the offset is simply set.
                //
                // Brought to the **middle**: a page chosen from the rail is the one
                // being looked for, and landing it against an edge shows half of it
                // with the neighbour it was chosen over filling the rest. The ends
                // of the document are the exception the scroll view makes for
                // itself — there is nothing past them to centre against.
                let width = viewportWidth - pageGap * 2
                if let top = contentTop(of: page, width: width) {
                    let height = rowHeight(page, width: width)
                    let extent = usableExtent(fallback: viewportHeight)
                    let target = top - max(0, (extent - height) / 2)
                    SessionRecorder.shared.record("TOOL_GESTURE",
                        String(format: "jump page=%d top=%.0f h=%.0f extent=%.0f ribbon=%.0f target=%.0f",
                               page, top, height, extent, ribbonHeight, target))
                    // One write, and it lands. There is no settle behind this any
                    // more: `contentTop` is now where the page *is* rather than
                    // where a lazy stack was predicted to have put it, and the
                    // content had its true height before the first row was built,
                    // so the scroll view's clamp has nothing left to cut down.
                    //
                    // Animated only for a page already on screen; animating a
                    // twenty-thousand-point move shows nothing but a blur.
                    commander.scroll(toContentY: target,
                                     animated: pageFramesBox.frames[page] != nil)
                }
            }
            // Leaving the magnified page builds this list again from nothing, and
            // a new ScrollView opens at the top of the document.
            //
            // The restore cannot hang off `currentPage` changing, because it does
            // not change: it was already set to the page that was pinned, on the
            // way in. So the reader zoomed into page 40, pinched out, and landed
            // on page 1. Restoring on appearance is the only signal that fires.
            //
            // Consumed once. Left unconditional, this re-ran every time anything
            // rebuilt the list and hauled the reader back up the document — you
            // would scroll up, and be moved down again a moment later.
            // Unanimated: the list has only just appeared, and sweeping it from
            // page 1 shows a journey nobody took.
            .onAppear {
                // Not through the proxy any more: only the pages near the
                // viewport exist in the board, so there is no row carrying page
                // 40's `.id` to ask for — and `contentTop` is the truth now, so
                // the offset is simply set.
                if let page = model.takeRestore() {
                    restore(to: page, viewportWidth: viewportWidth,
                            viewportHeight: viewportHeight)
                }
            }
        }

    }

    @ViewBuilder
    private var notice: some View {
        if let notice = model.notice {
            Text(notice)
                .font(.footnote.weight(.medium))
                // Orange when something did not work, so it reads as an answer
                // rather than as an acknowledgement.
                .foregroundStyle(model.noticeIsWarning ? .orange : Color.primary)
                .padding(.horizontal, 14).padding(.vertical, 8)
                .background(.regularMaterial, in: Capsule())
                .overlay(
                    Capsule().strokeBorder(model.noticeIsWarning
                                           ? Color.orange.opacity(0.5) : .clear,
                                           lineWidth: 1))
                .padding(.top, 6)
                .transition(.move(edge: .top).combined(with: .opacity))
                .task {
                    // Long enough to read a sentence. Two seconds is ample for
                    // "Copied." and not for an instruction — the scroll hint is
                    // two clauses long, shown at the top while the reader is
                    // looking at the ribbon at the bottom, and it was going
                    // before it had been noticed.
                    try? await Task.sleep(for: .seconds(4))
                    withAnimation {
                        model.notice = nil
                        model.noticeIsWarning = false
                    }
                }
        }
    }

    /// Android's `TopAppBar`: the document on the left, and the actions that
    /// belong to an edit on the right.
    ///
    /// Undo, redo and both saves stay put at any width — they apply to edits
    /// already made, so they have to be reachable when no tool is selected.
    /// Everything past them folds into the overflow.
    private var topBar: some View {
        HStack(spacing: 0) {
            Button {
                // Out of the magnified page first, if that is where you are.
                //
                // A departure from Android, whose `BackHandler` closes the
                // document whatever is on screen. Back means "undo the last
                // thing that took me somewhere", and entering the zoomed view is
                // one of those things — being thrown out to the library from a
                // page you were reading closely loses your place for a gesture
                // that meant something much smaller.
                if model.zoomedPage != nil {
                    model.exitZoom()
                    return
                }
                // Asked only when there is something to lose. With nothing
                // unsaved this leaves at once, and the question is never seen.
                if model.hasUnsavedWork { askingToLeave = true } else { dismiss() }
            } label: {
                Image(systemName: "chevron.backward")
                    .font(.system(size: 17, weight: .semibold))
                    .frame(width: 40, height: 44)
            }

            VStack(alignment: .leading, spacing: 0) {
                Text(model.document?.name ?? "Pagify")
                    .font(.system(size: 19, weight: .semibold))
                    .lineLimit(1)
                if let document = model.document {
                    Text("Page \(model.currentPage + 1) of \(document.pageCount)")
                        .font(.system(size: 12))
                        .foregroundStyle(PagifyColor.onSurface(scheme).opacity(0.6))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            barButton("arrow.uturn.backward", "Undo the last edit",
                      enabled: model.editState.canUndo, action: model.undo)
            barButton("arrow.uturn.forward", "Redo the last undone edit",
                      enabled: model.editState.canRedo, action: model.redo)
            barButton("square.and.arrow.down", "Save",
                      enabled: model.canSave, action: model.save)
            barButton("square.and.arrow.down.on.square", "Save a copy",
                      enabled: model.document != nil, action: model.saveCopy)

            // The eight reader actions. Zoom is not among them: it is gesture
            // only, and the three menu items that were here were invented.
            Menu {
                // Shown as a stop, tinted with the error colour, while it runs —
                // a recorder left on by accident should be visible.
                Button { model.toggleRecording() } label: {
                    Label(model.isRecording
                            ? "Stop recording and save the render timeline"
                            : "Record a render timeline",
                          systemImage: model.isRecording ? "stop.circle" : "record.circle")
                }
                .tint(model.isRecording ? .red : nil)

                Button { model.showThumbnails.toggle() } label: {
                    Label(model.showThumbnails ? "Hide page thumbnails" : "Show page thumbnails",
                          systemImage: "sidebar.left")
                }
                Button { model.rotatePage(model.currentPage) } label: {
                    Label("Rotate", systemImage: "arrow.clockwise")
                }
                Button { showingOrganiser = true } label: {
                    Label(model.editState.dirty
                            ? "Organise pages \u{2014} unsaved changes" : "Organise pages",
                          systemImage: "square.grid.2x2")
                }
                Button { model.insertBlankPage(after: model.currentPage) } label: {
                    Label("Add a blank page", systemImage: "photo.badge.plus")
                }
                Button(role: .destructive) { model.deletePage(model.currentPage) } label: {
                    Label("Delete this page", systemImage: "trash")
                }
                .disabled((model.document?.pageCount ?? 0) <= 1)
                Divider()
                Button { showingMetadata = true } label: {
                    Label("Document details", systemImage: "info.circle")
                }
                Button { dismiss() } label: {
                    Label("Open a PDF", systemImage: "folder")
                }
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 17, weight: .semibold))
                    .frame(width: 40, height: 44)
            }
        }
        .padding(.horizontal, 6)
        .foregroundStyle(PagifyColor.onSurface(scheme))
        .background(PagifyColor.background(scheme))
    }

    private func barButton(_ symbol: String, _ label: String,
                           enabled: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 17, weight: .medium))
                .frame(width: 40, height: 44)
                .opacity(enabled ? 1 : 0.3)
        }
        .disabled(!enabled)
        .accessibilityLabel(label)
    }
}

/// One page, rendered off the main thread at the width it is actually shown at,
/// with the drawing layer over it.
struct PageView: View {
    let document: PagifyDocument
    let index: Int
    let width: CGFloat
    /// The settled zoom. Only the *resolution* follows it — the layout width
    /// above is fixed, so a zoom never re-lays the page out.
    let zoom: CGFloat
    let onAspect: (CGFloat) -> Void
    let revision: Int
    /// Separate from `revision`: a new mark redraws the overlay, it does not make
    /// PDFium rasterise the page again.
    let annotationRevision: Int
    let settings: AnnotationSettings
    let committed: [WireAnnotation]
    let onCommit: (WireAnnotation) -> Void
    let onErase: (CGPoint) -> Void
    let onEraseStart: () -> Void
    let onEraseEnd: () -> Void
    let onPlaceText: (CGPoint, CGPoint?) -> Void
    let selectedText: Int32?
    let onSelectText: (Int32?) -> Void
    let onMoveText: (Int32, CGSize) -> Void
    let onEditText: (Int32) -> Void
    let segments: [TextSegment]
    let onHighlightMissed: () -> Void
    var onScrollBlocked: () -> Void = {}
    var twoFingersDown: Bool = false
    let onRequestNote: (CGPoint) -> Void
    let onOpenNote: (Int) -> Void
    let onAppearPage: () -> Void
    /// The text selected anywhere in the reader; the canvas draws it only if it
    /// belongs to this page.
    var selection: PageTextSelection?
    var onSelectWord: (CGPoint) -> Void = { _ in }
    var onMoveSelectionHandle: (Bool, CGPoint) -> Void = { _, _ in }
    var onClearSelection: () -> Void = {}
    var selectedMark: Int?
    var onSelectMark: (Int?) -> Void = { _ in }
    var onMoveMark: (Int, CGSize, Bool) -> Void = { _, _, _ in }

    @Environment(\.displayScale) private var displayScale
    @State private var image: CGImage?
    /// Passed in, not measured. See `ReaderModel.pageSize(_:)` for why a row that
    /// corrects its own height keeps the whole list moving.
    let pageSize: CGSize

    /// What the page will actually be rasterised at, quantised so that small
    /// zoom changes reuse the raster the engine already has.
    private var renderScale: CGFloat {
        RenderScale.forPage(pageSize, targetPixelWidth: width * displayScale * zoom)
    }

    private var displayedSize: CGSize {
        guard pageSize.width > 0 else { return CGSize(width: width, height: width) }
        return CGSize(width: width, height: width * pageSize.height / pageSize.width)
    }

    var body: some View {
        ZStack {
            if let image {
                Image(decorative: image, scale: displayScale)
                    .resizable()
                    .frame(width: displayedSize.width, height: displayedSize.height)
            } else {
                Rectangle()
                    .fill(Color(.secondarySystemGroupedBackground))
                    .frame(width: displayedSize.width, height: displayedSize.height)
                    // No spinner.
                    //
                    // An indeterminate `ProgressView` animates for as long as it
                    // exists, and animation invalidates its subtree every frame —
                    // which re-evaluates the enclosing `GeometryReader` and calls
                    // the page-frame preference again, thirty times a second, for
                    // ever. In a long document there is always a page in view
                    // waiting on its raster, so the reader never stopped relaying
                    // itself. A page arrives in tens of milliseconds; a bare
                    // rectangle is the honest placeholder for that, and a still
                    // one costs nothing.
            }

            AnnotationCanvas(pageIndex: index,
                             // In the list the page sits at the top-left of its
                             // own row, so the origin is zero.
                             mapping: PageMapping(
                                scale: pageSize.width > 0 ? displayedSize.width / pageSize.width : 0,
                                origin: .zero,
                                pageWidthPoints: pageSize.width,
                                pageHeightPoints: pageSize.height),
                             settings: settings,
                             onCommit: onCommit,
                             onErase: onErase,
                             onEraseStart: onEraseStart,
                             onEraseEnd: onEraseEnd,
                             onPlaceText: onPlaceText,
                             committed: committed,
                             selectedText: selectedText,
                             onSelectText: onSelectText,
                             onMoveText: onMoveText,
                             onEditText: onEditText,
                             segments: segments,
                             selection: selection,
                             onSelectWord: onSelectWord,
                             onMoveSelectionHandle: onMoveSelectionHandle,
                             onClearSelection: onClearSelection,
                             selectedMark: selectedMark,
                             onSelectMark: onSelectMark,
                             onMoveMark: onMoveMark,
                             onHighlightMissed: onHighlightMissed,
                             onScrollBlocked: onScrollBlocked,
                             twoFingersDown: twoFingersDown,
                             annotationRevision: annotationRevision,
                             onRequestNote: onRequestNote,
                             onOpenNote: onOpenNote)
                .frame(width: displayedSize.width, height: displayedSize.height)
        }
        // No rounded clip: it crops marks drawn at the very edge of the sheet,
        // which is exactly where a margin note goes.
        // Re-render on any edit as well as on resize: committed marks are drawn
        // by PDFium as part of the page, so the re-render is what proves the mark
        // reached the document.
        // Keyed on the *quantised* scale, not on the zoom. A pinch produces a
        // continuum of values, and re-rendering on each of them is what made the
        // page flash on every gesture; crossing a quantum is the only change that
        // actually produces different pixels.
        .task(id: "\(width)-\(revision)-\(renderScale)") { await render() }
        .onAppear { onAppearPage() }
    }

    private func render() async {
        guard width > 0 else { return }

        // Held back a beat, so a page the reader is scrolling **past** is never
        // drawn at all.
        //
        // `.task(id:)` cancels when the row leaves, but a render already started
        // runs to completion regardless — a recording of one flick through a
        // 149-page document caught four full-page renders finishing on the same
        // millisecond, 79ms each, again and again. Four PDFium rasters competing for the
        // same core is what the scroll felt like. A page that is still on screen
        // a tenth of a second later is one somebody is actually looking at.
        try? await Task.sleep(for: .milliseconds(90))
        guard !Task.isCancelled else { return }

        let index = index
        let document = document
        // The layout width times the zoom: the page is displayed at `width` and
        // then transformed, so at 3x it needs three times the pixels to stay
        // sharp — without ever being laid out any wider.
        let target = width * displayScale * zoom
        let began = DispatchTime.now().uptimeNanoseconds
        SessionRecorder.shared.record("PAGE_ENTER", String(format: "page=%d pts=%.0f zoom=%.2f", index, width, zoom))

        let rendered: (CGImage, CGSize)? = await RenderGate.shared.run {
            await Task.detached(priority: .userInitiated) {
            guard let size = try? document.pageSize(index), size.width > 0 else { return nil }
            // Quantised and area-capped, so the engine's cache can hit and a deep
            // zoom cannot ask for an allocation it will refuse.
            let scale = RenderScale.forPage(size, targetPixelWidth: target)
            guard let image = try? document.render(page: index, scale: scale) else { return nil }
            return (image, size)
            }.value
        }

        // Superseded on the way back: the row may have left while this was in
        // flight, and drawing into a cell that has been recycled for another page
        // puts the wrong picture on screen.
        guard !Task.isCancelled else { return }

        if let rendered {
            SessionRecorder.shared.record("PAGE_READABLE",
               String(format: "page=%d px=%dx%d scale=%.2f",
                      index, rendered.0.width, rendered.0.height, renderScale),
               durationMillis: Int((DispatchTime.now().uptimeNanoseconds &- began) / 1_000_000))
            // The reader needs every page's shape to know how far it scrolls,
            // long before it has drawn them all.
            onAspect(rendered.1.width / rendered.1.height)
            image = rendered.0
        }
    }
}

/// What the document says about itself.
struct MetadataSheet: View {
    let document: PagifyDocument
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        List {
            ForEach(rows, id: \.0) { row in
                LabeledContent(row.0, value: row.1)
            }
        }
        .navigationTitle("Document details")
        .navigationBarTitleDisplayMode(.inline)
    }

    /// Title, Author, Subject, Keywords, Creator, Producer, then Pages.
    ///
    /// A row whose value is missing is left out rather than shown blank — an
    /// empty "Author" line says the document has no author, which is not the
    /// same as the file not recording one.
    private var rows: [(String, String)] {
        var out: [(String, String)] = []
        if let json = try? document.metadataJSON(),
           let data = json.data(using: .utf8),
           let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] {
            for (key, label) in [("title", "Title"), ("author", "Author"),
                                 ("subject", "Subject"), ("keywords", "Keywords"),
                                 ("creator", "Creator"), ("producer", "Producer")] {
                if let value = o[key] as? String,
                   !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    out.append((label, value))
                }
            }
        }
        out.append(("Pages", "\(document.pageCount)"))
        return out
    }
}


/// A written capture, waiting to be handed on.
///
/// A wrapper only because `URL` is not `Identifiable`, and the file name is
/// already unique — it carries the moment it was taken.
struct CapturedFile: Identifiable {
    let url: URL
    var id: String { url.lastPathComponent }
}
