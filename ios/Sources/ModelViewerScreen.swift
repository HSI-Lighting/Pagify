import SwiftUI

/// A STEP file, full screen — the iOS twin of Android's `ModelViewer.kt`.
///
/// One flattened mesh and a trackball camera, never a part tree: STEP
/// assemblies are placed in world space during tessellation on the Rust
/// side (`step::assembly`) and never exposed as a hierarchy, so there is no
/// outliner here to build — Android has none either.
///
/// Presented as a `.fullScreenCover` from `RootView`, the same way the PDF
/// reader takes the screen — a viewer with a tab bar under it is a viewer
/// with a strip of its own screen missing.
struct ModelViewerScreen: View {
    let file: OpenedFile
    let recents: RecentDocumentsStore
    var onClose: () -> Void

    private var url: URL { file.url }

    @State private var document: ModelDocument?
    @State private var summary: ModelSummary?
    @State private var image: CGImage?
    @State private var errorMessage: String?
    @State private var loading = true
    @State private var showingDetails = false
    /// While a finger is down, frames render at 1x for responsiveness — the
    /// same trade Android's own `PROXY_FRACTION` makes, just a two-step
    /// switch instead of a continuous fraction. The final frame after the
    /// finger lifts always renders at the display's real scale.
    @State private var gesturing = false

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
                // Matches Android's own fixed backdrop (Style::default's
                // background / ModelViewer.kt's BACKDROP), not a system
                // background colour — the model's own shading assumes it.
                Color(red: 30 / 255, green: 32 / 255, blue: 36 / 255).ignoresSafeArea()
                body(for: document)
            }
            .navigationTitle(document?.name ?? url.lastPathComponent)
            .navigationBarTitleDisplayMode(.inline)
            .toolbarColorScheme(.dark, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { onClose() }
                }
                if summary != nil {
                    ToolbarItem(placement: .primaryAction) {
                        Button {
                            showingDetails.toggle()
                        } label: {
                            Image(systemName: "info.circle")
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
                    readerBackground: MarkColor(argb: modelBackdropARGB),
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
    private func body(for document: ModelDocument?) -> some View {
        if let errorMessage {
            ContentUnavailableView("Could Not Open Model", systemImage: "cube.transparent",
                                   description: Text(errorMessage))
                .foregroundStyle(.white)
        } else if loading {
            ProgressView().tint(.white)
        } else {
            GeometryReader { geo in
                ZStack(alignment: .topLeading) {
                    if let image {
                        Image(decorative: image, scale: image.width == Int(geo.size.width) ? 1 : UIScreen.main.scale)
                            .resizable()
                            .frame(width: geo.size.width, height: geo.size.height)
                    }
                    TrackballGestureSurface(
                        onDrag: { dx, dy, fingers in
                            guard let document else { return }
                            gesturing = true
                            let width = max(geo.size.width, 1)
                            let height = max(geo.size.height, 1)
                            if fingers == 1 {
                                // Matches Android's own `orbit`: the view's
                                // *width* on both axes, so a non-square
                                // viewport doesn't skew one axis against
                                // the other.
                                document.orbit(across: dx / width, down: dy / width)
                            } else {
                                document.pan(across: dx / width, down: dy / height)
                            }
                            render(document: document, size: geo.size, fullRes: false)
                        },
                        onPinch: { ratio, _ in
                            guard let document else { return }
                            gesturing = true
                            document.zoom(by: ratio)
                            render(document: document, size: geo.size, fullRes: false)
                        },
                        onGestureEnded: {
                            gesturing = false
                            if let document { render(document: document, size: geo.size, fullRes: true) }
                        }
                    )
                    .captureOverlay(active: framing, lasso: lasso) { box, ring in
                        takeRegion(box, ring: ring)
                    }
                }
                .onAppear { render(document: document, size: geo.size, fullRes: true) }
                .onChange(of: geo.size) { _, newSize in
                    if let document { render(document: document, size: newSize, fullRes: true) }
                }
            }

            VStack {
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
                    Button {
                        document?.fit()
                        if let document { render(document: document, size: lastRenderSize, fullRes: true) }
                    } label: {
                        Label("Fit", systemImage: "arrow.up.left.and.arrow.down.right")
                            .labelStyle(.iconOnly)
                            .padding(14)
                    }
                    .buttonStyle(.borderedProminent)
                    .clipShape(Circle())

                    // Its own slot rather than a shape hidden behind a press
                    // on the one beside it, matching the reader's own ribbon:
                    // a box for most things, a ring for the detail a box
                    // can't take without its neighbours.
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
                }
                .padding(.bottom, 24)
            }

            if showingDetails, let summary {
                detailsPanel(summary)
            }
        }
    }

    @State private var lastRenderSize: CGSize = .zero

    private func detailsPanel(_ summary: ModelSummary) -> some View {
        VStack(alignment: .leading) {
            VStack(alignment: .leading, spacing: 6) {
                detailRow("Triangles", "\(summary.triangles.formatted())")
                detailRow("Faces", "\(summary.facesDrawn.formatted()) of \(summary.facesInFile.formatted())")
                detailRow("Size", String(format: "%.0f × %.0f × %.0f mm", summary.size.x, summary.size.y, summary.size.z))
                if summary.assembly > 0 {
                    detailRow("Assembly links", "\(summary.assembly)")
                }
                let surfaces = summary.surfaces
                let surfaceParts = [
                    ("Plane", surfaces.plane), ("Cylinder", surfaces.cylinder), ("Cone", surfaces.cone),
                    ("Torus", surfaces.torus), ("Sphere", surfaces.sphere), ("Freeform", surfaces.freeform),
                ].filter { $0.1 > 0 }
                ForEach(surfaceParts, id: \.0) { label, count in
                    detailRow(label, "\(count)")
                }
            }
            .padding(12)
            .background(.black.opacity(0.7), in: RoundedRectangle(cornerRadius: 12))
            .foregroundStyle(.white)
            Spacer()
        }
        .padding()
    }

    private func detailRow(_ label: String, _ value: String) -> some View {
        HStack {
            Text(label).foregroundStyle(.white.opacity(0.7))
            Spacer()
            Text(value)
        }
        .font(.caption)
    }

    private func open() {
        let scoped = file.alreadyScoped || url.startAccessingSecurityScopedResource()
        do {
            let opened = try ModelDocument(path: url.path, name: url.lastPathComponent,
                                          scopedURL: scoped ? url : nil)
            document = opened
            summary = opened.summary()
            recents.remember(url: url, name: opened.name, pageCount: 0, kind: .model)
            loading = false
        } catch {
            if scoped { url.stopAccessingSecurityScopedResource() }
            errorMessage = error.localizedDescription
            loading = false
        }
    }

    private func render(document: ModelDocument?, size: CGSize, fullRes: Bool) {
        guard let document else { return }
        lastRenderSize = size
        guard size.width > 0, size.height > 0 else { return }
        let scale = fullRes ? UIScreen.main.scale : 1
        let width = max(1, Int(size.width * scale))
        let height = max(1, Int(size.height * scale))
        image = try? document.render(width: width, height: height)
    }

    // ---- capture --------------------------------------------------------------

    /// The dark ground a model's own capture fills where the lasso left
    /// nothing — matches `body`'s own backdrop, `#1E2024`.
    private var modelBackdropARGB: UInt32 { 0xFF1E_2024 }

    private func pixelSize(of size: CGSize) -> (width: Int, height: Int) {
        let scale = UIScreen.main.scale
        return (max(1, Int(size.width * scale)), max(1, Int(size.height * scale)))
    }

    /// Turn a dragged box or lasso into a picture, and open the editor on it.
    ///
    /// The box/ring `.captureOverlay` reports are in this view's own points;
    /// `pixelSize(of:)` converts, once, into the real device pixels the
    /// camera's own render buffer is measured in — the same conversion the
    /// drawing viewer makes, though the model's camera has no equivalent bug
    /// to work around: a camera reprojects onto whatever canvas it is given,
    /// so only the *region math* needs pixels, never the render call itself.
    private func takeRegion(_ box: PageRect, ring: [CGPoint]) {
        guard !isCapturing, let document else { return }
        framing = false

        let scale = UIScreen.main.scale
        captureBox = PageRect(left: box.left * scale, top: box.top * scale,
                              right: box.right * scale, bottom: box.bottom * scale)
        captureRing = ring.map { CGPoint(x: $0.x * scale, y: $0.y * scale) }

        isCapturing = true
        Task {
            let request = CaptureRequest(tiles: [], width: 0, height: 0,
                                         background: MarkColor(argb: modelBackdropARGB),
                                         originPage: 0, scale: .high, format: .png)
            capture = await renderCapture(request, document: document)
            isCapturing = false
        }
    }

    /// Draw the region framed by `captureBox`/`captureRing`, off the main
    /// thread, at `request.scale`/`format` — called both for the first
    /// picture and every time the editor's own sheet asks for a different
    /// sharpness or format, from the same remembered region.
    private func renderCapture(_ request: CaptureRequest, document: ModelDocument,
                               markup: [Markup] = []) async -> CapturePreview? {
        guard let box = captureBox else { return nil }
        let (viewWidth, viewHeight) = pixelSize(of: lastRenderSize)
        let (wholeWidth, wholeHeight) = viewerCaptureSize(
            viewWidth: viewWidth, viewHeight: viewHeight, scale: Int(request.scale.factor))
        guard let cut = viewerRegionInCapture(box: box, viewWidth: viewWidth, viewHeight: viewHeight,
                                             wholeWidth: wholeWidth, wholeHeight: wholeHeight) else { return nil }
        // The camera reprojects onto whatever canvas it is given, so unlike
        // the drawing side there is no `by` to pass here — only the ring's
        // point-to-pixel factor, for `viewerCutOut` below.
        let factor = CGFloat(wholeWidth) / CGFloat(viewWidth)
        let ring = captureRing
        let format = request.format
        let name = document.name

        return await Task.detached(priority: .userInitiated) { () -> CapturePreview? in
            guard let whole = try? document.render(width: wholeWidth, height: wholeHeight) else { return nil }
            var picture = viewerCutOut(whole, cut: cut, ring: ring, factor: factor)
            if !markup.isEmpty {
                picture = compositeViewerMarkup(into: picture, marks: markup, scale: factor)
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
                               marks: [Markup], document: ModelDocument) async {
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
