import SwiftUI

/// A DXF/DWG file, full screen — the iOS twin of Android's
/// `DrawingViewer.kt`. DWG and DXF are not two code paths here: both
/// convert into the same `Drawing` on the Rust side, so pan, zoom, measure
/// and layers all work identically regardless of which file was opened —
/// see `DrawingDocument`'s own doc comment.
struct DrawingViewerScreen: View {
    let file: OpenedFile
    let recents: RecentDocumentsStore
    var onClose: () -> Void

    private var url: URL { file.url }

    @State private var document: DrawingDocument?
    @State private var summary: DrawingSummary?
    @State private var layers: [DrawingLayer] = []
    @State private var image: CGImage?
    @State private var errorMessage: String?
    @State private var loading = true
    @State private var showingLayers = false
    @State private var measuring = false
    @State private var measurement: DrawingMeasurement?
    @State private var hasFitted = false
    @State private var fitSettleTask: Task<Void, Never>?
    @State private var lastRenderSize: CGSize = .zero

    // ---- capture ------------------------------------------------------------
    @State private var framing = false
    @State private var lasso = false
    @State private var captureBox: PageRect?
    @State private var captureRing: [CGPoint] = []
    @State private var capture: CapturePreview?
    @State private var capturedFile: CapturedFile?
    @State private var isCapturing = false

    var body: some View {
        NavigationStack {
            ZStack {
                // Matches `drawing::raster::Style::default().background`.
                Color(red: 24 / 255, green: 26 / 255, blue: 30 / 255).ignoresSafeArea()
                body(for: document)
            }
            .navigationTitle(document?.name ?? url.lastPathComponent)
            .navigationBarTitleDisplayMode(.inline)
            .toolbarColorScheme(.dark, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { onClose() }
                }
                if !layers.isEmpty {
                    ToolbarItem(placement: .primaryAction) {
                        Button {
                            showingLayers.toggle()
                        } label: {
                            Image(systemName: "square.3.layers.3d")
                        }
                    }
                }
            }
        }
        .preferredColorScheme(.dark)
        .task { open() }
        .fullScreenCover(item: $capture) { taken in
            if let document {
                CaptureEditor(
                    capture: taken,
                    readerBackground: MarkColor(argb: drawingBackdropARGB),
                    render: { request in await renderCapture(request, document: document) },
                    export: { action, request, marks in
                        await exportCapture(action, request: request, marks: marks, document: document)
                    },
                    onDismiss: { capture = nil }
                )
            }
        }
        .sheet(item: $capturedFile) { file in
            ShareSheet(items: [file.url])
        }
    }

    @ViewBuilder
    private func body(for document: DrawingDocument?) -> some View {
        if let errorMessage {
            ContentUnavailableView("Could Not Open Drawing", systemImage: "square.on.square.dashed",
                                   description: Text(errorMessage))
                .foregroundStyle(.white)
        } else if loading {
            ProgressView().tint(.white)
        } else {
            GeometryReader { geo in
                ZStack(alignment: .topLeading) {
                    if let image {
                        Image(decorative: image, scale: UIScreen.main.scale)
                            .resizable()
                            .frame(width: geo.size.width, height: geo.size.height)
                    }
                    TrackballGestureSurface(
                        onDrag: { dx, dy, _ in
                            // No orbit here — a drawing is flat, so one
                            // finger and two both pan, matching Android's
                            // own `DrawingViewer.kt` exactly (it never
                            // distinguishes finger count for a plain drag,
                            // only for the pinch that rides along with it).
                            guard let document else { return }
                            let (width, height) = pixelSize(of: geo.size)
                            document.pan(across: dx / max(geo.size.width, 1), down: dy / max(geo.size.height, 1),
                                        width: width, height: height)
                            render(document: document, size: geo.size)
                        },
                        onPinch: { ratio, midpoint in
                            guard let document else { return }
                            let (width, height) = pixelSize(of: geo.size)
                            document.zoom(by: ratio, atX: midpoint.x * pixelScale, atY: midpoint.y * pixelScale,
                                         width: width, height: height)
                            render(document: document, size: geo.size)
                        },
                        onTap: { point in
                            guard measuring, let document else { return }
                            let (width, height) = pixelSize(of: geo.size)
                            measurement = document.measure(
                                atX: point.x * pixelScale, atY: point.y * pixelScale,
                                width: width, height: height
                            )
                            render(document: document, size: geo.size)
                        }
                    )
                    .captureOverlay(active: framing, lasso: lasso) { box, ring in
                        takeRegion(box, ring: ring)
                    }
                }
                .onAppear {
                    scheduleInitialFit(size: geo.size)
                }
                .onChange(of: geo.size) { _, newSize in
                    // Re-fitting here, like `ModelViewerScreen` never does,
                    // would undo the user's own pan/zoom every time the
                    // canvas resizes for a reason that has nothing to do with
                    // them — the layers button appearing once `layers` loads
                    // shifts the nav bar, same as a rotation would. Only the
                    // very first, settled layout gets to fit — see
                    // `scheduleInitialFit`.
                    if hasFitted {
                        if let document { render(document: document, size: newSize) }
                    } else {
                        scheduleInitialFit(size: newSize)
                    }
                }
            }

            VStack {
                if let readout = measuring ? (measurement?.readout ?? "Tap a point") : nil {
                    Text(readout)
                        .font(.caption)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(.black.opacity(0.65), in: Capsule())
                        .padding(.top, 8)
                }
                Spacer()
                if let warning = summary?.skippedWarning {
                    Text(warning)
                        .font(.caption)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(.black.opacity(0.65), in: Capsule())
                        .padding(.bottom, 8)
                }
                HStack(spacing: 16) {
                    // Matches Android's own `ModelRibbon` order exactly:
                    // Snapshot (its box/ring choice split into its own slot
                    // apiece, the reader's own convention, rather than
                    // Android's single button plus dropdown) then Fit, then
                    // the drawing's own extra tool — Measure — last, in the
                    // slot Android's `extra` parameter fills only for this
                    // viewer.
                    Button {
                        let holding = framing && !lasso
                        lasso = false
                        framing = !holding
                    } label: {
                        Label("Snapshot", systemImage: "viewfinder")
                            .labelStyle(.iconOnly)
                            .padding(14)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(framing && !lasso ? .orange : .accentColor)
                    .clipShape(Circle())

                    Button {
                        let holding = framing && lasso
                        lasso = true
                        framing = !holding
                    } label: {
                        Label("Draw around", systemImage: "scribble.variable")
                            .labelStyle(.iconOnly)
                            .padding(14)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(framing && lasso ? .orange : .accentColor)
                    .clipShape(Circle())

                    Button {
                        let (width, height) = pixelSize(of: lastRenderSize)
                        document?.fit(width: width, height: height)
                        if let document { render(document: document, size: lastRenderSize) }
                    } label: {
                        Label("Fit", systemImage: "arrow.up.left.and.arrow.down.right")
                            .labelStyle(.iconOnly)
                            .padding(14)
                    }
                    .buttonStyle(.borderedProminent)
                    .clipShape(Circle())

                    Button {
                        measuring.toggle()
                        if !measuring {
                            document?.clearMeasure()
                            measurement = nil
                        }
                    } label: {
                        Label("Measure", systemImage: "ruler")
                            .labelStyle(.iconOnly)
                            .padding(14)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(measuring ? .orange : .accentColor)
                    .clipShape(Circle())
                }
                .padding(.bottom, 24)
            }

            if showingLayers {
                layersPanel
            }
        }
    }

    private var layersPanel: some View {
        VStack(alignment: .leading) {
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(layers.enumerated()), id: \.element.id) { index, layer in
                    Toggle(isOn: Binding(
                        get: { layer.visible },
                        set: { visible in
                            layers[index] = DrawingLayer(name: layer.name, visible: visible, colour: layer.colour)
                            document?.showLayer(at: index, visible: visible)
                            if let document { render(document: document, size: lastRenderSize) }
                        }
                    )) {
                        Text(layer.name)
                    }
                    .tint(.accentColor)
                }
            }
            .padding(12)
            .background(.black.opacity(0.7), in: RoundedRectangle(cornerRadius: 12))
            .foregroundStyle(.white)
            .frame(maxWidth: 260)
            Spacer()
        }
        .padding()
    }

    private func open() {
        let scoped = file.alreadyScoped || url.startAccessingSecurityScopedResource()
        do {
            let opened = try DrawingDocument(path: url.path, name: url.lastPathComponent,
                                            scopedURL: scoped ? url : nil)
            document = opened
            summary = opened.summary()
            layers = opened.layers()
            recents.remember(url: url, name: opened.name, pageCount: 0, kind: .drawing)
            loading = false
        } catch {
            if scoped { url.stopAccessingSecurityScopedResource() }
            errorMessage = error.localizedDescription
            loading = false
        }
    }

    private func render(document: DrawingDocument?, size: CGSize) {
        guard let document else { return }
        lastRenderSize = size
        guard size.width > 0, size.height > 0 else { return }
        let (width, height) = pixelSize(of: size)
        image = try? document.render(width: width, height: height)
    }

    /// The very first size `GeometryReader` reports here is sometimes a
    /// mid-transition sliver of the real canvas — a `.fullScreenCover`
    /// settles into place over a couple of layout passes, not instantly —
    /// and fitting against that sliver locks in a scale for a viewport that
    /// never actually existed on screen. Debouncing past a short quiet
    /// window means whichever size arrives last (the real, settled one) is
    /// the only one `fit()` ever sees, and — same as the guard already on
    /// every call site — it still only ever runs once.
    private func scheduleInitialFit(size: CGSize) {
        guard !hasFitted else { return }
        fitSettleTask?.cancel()
        fitSettleTask = Task {
            try? await Task.sleep(nanoseconds: 200_000_000)
            guard !Task.isCancelled, !hasFitted else { return }
            hasFitted = true
            let (width, height) = pixelSize(of: size)
            document?.fit(width: width, height: height)
            render(document: document, size: size)
        }
    }

    /// One physical pixel per unit of `view.scale` — not one point. Every
    /// Rust entry point here (`fit`/`pan`/`zoom_about`/`measure_at`/`draw`)
    /// documents its `width`/`height`/`at_x`/`at_y` as **pixels**, and
    /// `draw`'s own doc spells out why that word is load-bearing: "a sheet
    /// does not reframe itself when the canvas grows" — its scale is a fixed
    /// pixels-per-drawing-unit ratio, not derived from whatever buffer it is
    /// asked to fill. Calibrating that ratio with `fit()` in SwiftUI's own
    /// points, then rendering a `points × UIScreen.scale` bitmap through the
    /// very same ratio, was exactly this bug: the picture that came back
    /// showed `UIScreen.scale` times more of the drawing than `fit()` had
    /// framed — a crop stretched to fill the screen, not a scale of it, and
    /// on release from a gesture (full-res render, real device pixels) it
    /// visibly jumped relative to mid-gesture (proxy render, plain points).
    /// Every call site below converts through this one point instead, so
    /// `fit`, `pan`, `zoom`, `measure` and `render` all calibrate and read
    /// the identical pixels-per-unit ratio.
    private var pixelScale: CGFloat { UIScreen.main.scale }

    private func pixelSize(of size: CGSize) -> (width: Int, height: Int) {
        (max(1, Int(size.width * pixelScale)), max(1, Int(size.height * pixelScale)))
    }

    // ---- capture --------------------------------------------------------------

    /// The dark ground a drawing's own capture fills where the lasso left
    /// nothing — matches `body`'s own backdrop, `#181A1E`.
    private var drawingBackdropARGB: UInt32 { 0xFF18_1A_1E }

    /// Turn a dragged box or lasso into a picture, and open the editor on it.
    ///
    /// The box/ring `.captureOverlay` reports are in this view's own points;
    /// everything downstream (`viewerRegionInCapture`, `viewerCutOut`) works
    /// in the same pixels `fit`/`pan`/`zoom`/`measure` already do, so they are
    /// converted here, once, rather than carrying two units through the rest
    /// of the capture path.
    private func takeRegion(_ box: PageRect, ring: [CGPoint]) {
        guard !isCapturing, let document else { return }
        framing = false

        let scale = pixelScale
        captureBox = PageRect(left: box.left * scale, top: box.top * scale,
                              right: box.right * scale, bottom: box.bottom * scale)
        captureRing = ring.map { CGPoint(x: $0.x * scale, y: $0.y * scale) }

        isCapturing = true
        Task {
            let request = CaptureRequest(tiles: [], width: 0, height: 0,
                                         background: MarkColor(argb: drawingBackdropARGB),
                                         originPage: 0, scale: .high, format: .png)
            capture = await renderCapture(request, document: document)
            isCapturing = false
        }
    }

    /// Draw the region framed by `captureBox`/`captureRing`, off the main
    /// thread, at `request.scale`/`format` — called both for the first
    /// picture and every time the editor's own sheet asks for a different
    /// sharpness or format, from the same remembered region.
    private func renderCapture(_ request: CaptureRequest, document: DrawingDocument,
                               markup: [Markup] = []) async -> CapturePreview? {
        guard let box = captureBox else { return nil }
        let (viewWidth, viewHeight) = pixelSize(of: lastRenderSize)
        let (wholeWidth, wholeHeight) = viewerCaptureSize(
            viewWidth: viewWidth, viewHeight: viewHeight, scale: Int(request.scale.factor))
        guard let cut = viewerRegionInCapture(box: box, viewWidth: viewWidth, viewHeight: viewHeight,
                                             wholeWidth: wholeWidth, wholeHeight: wholeHeight) else { return nil }
        let by = Double(wholeWidth) / Double(viewWidth)
        let ring = captureRing
        let format = request.format
        let name = document.name

        return await Task.detached(priority: .userInitiated) { () -> CapturePreview? in
            guard let whole = try? document.render(width: wholeWidth, height: wholeHeight, by: by) else {
                return nil
            }
            var picture = viewerCutOut(whole, cut: cut, ring: ring, factor: CGFloat(by))
            if !markup.isEmpty {
                picture = compositeViewerMarkup(into: picture, marks: markup, scale: CGFloat(by))
            }
            guard let bytes = encodeViewerCapture(picture, format: format) else { return nil }

            var updated = request
            updated.width = Double(picture.width)
            updated.height = Double(picture.height)
            return CapturePreview(
                request: updated,
                bytes: bytes,
                fileName: viewerCaptureFileName(source: name, stamp: viewerCaptureTimestamp(), format: format),
                picture: picture)
        }.value
    }

    /// Hand the finished picture — marks drawn in by the engine — to Files, a
    /// share sheet or the pasteboard. Same shape as `ReaderView`'s own
    /// `exportCapture`.
    private func exportCapture(_ action: CaptureExportAction, request: CaptureRequest,
                               marks: [Markup], document: DrawingDocument) async {
        isCapturing = true
        defer { isCapturing = false }
        guard let preview = await renderCapture(request, document: document, markup: marks) else { return }

        switch action {
        case .copy:
            UIPasteboard.general.setData(preview.bytes, forPasteboardType: request.format.pasteboardType)
        case .save, .share:
            let url = FileManager.default.temporaryDirectory.appendingPathComponent(preview.fileName)
            try? preview.bytes.write(to: url, options: .atomic)
            capturedFile = CapturedFile(url: url)
        }
    }
}
