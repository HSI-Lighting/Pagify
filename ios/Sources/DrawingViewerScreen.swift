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
    @State private var lastRenderSize: CGSize = .zero
    /// As in `ModelViewerScreen`: 1x while a finger is down, the display's
    /// real scale once it lifts.
    @State private var gesturing = false

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
                        Image(decorative: image, scale: image.width == Int(geo.size.width) ? 1 : UIScreen.main.scale)
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
                            gesturing = true
                            let width = max(geo.size.width, 1)
                            let height = max(geo.size.height, 1)
                            document.pan(across: dx / width, down: dy / height,
                                        width: Int(width), height: Int(height))
                            render(document: document, size: geo.size, fullRes: false)
                        },
                        onPinch: { ratio, midpoint in
                            guard let document else { return }
                            gesturing = true
                            document.zoom(by: ratio, atX: midpoint.x, atY: midpoint.y,
                                         width: Int(geo.size.width), height: Int(geo.size.height))
                            render(document: document, size: geo.size, fullRes: false)
                        },
                        onTap: { point in
                            guard measuring, let document else { return }
                            measurement = document.measure(
                                atX: point.x, atY: point.y,
                                width: Int(geo.size.width), height: Int(geo.size.height)
                            )
                            render(document: document, size: geo.size, fullRes: true)
                        },
                        onGestureEnded: {
                            gesturing = false
                            if let document { render(document: document, size: geo.size, fullRes: true) }
                        }
                    )
                }
                .onAppear {
                    guard !hasFitted else { return }
                    hasFitted = true
                    document?.fit(width: Int(geo.size.width), height: Int(geo.size.height))
                    render(document: document, size: geo.size, fullRes: true)
                }
                .onChange(of: geo.size) { _, newSize in
                    // Re-fitting here, like `ModelViewerScreen` never does,
                    // would undo the user's own pan/zoom every time the
                    // canvas resizes for a reason that has nothing to do with
                    // them — the layers button appearing once `layers` loads
                    // shifts the nav bar, same as a rotation would. Only the
                    // very first layout (`onAppear`, above) gets to fit.
                    if let document { render(document: document, size: newSize, fullRes: true) }
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

                    Button {
                        document?.fit(width: Int(lastRenderSize.width), height: Int(lastRenderSize.height))
                        if let document { render(document: document, size: lastRenderSize, fullRes: true) }
                    } label: {
                        Label("Fit", systemImage: "arrow.up.left.and.arrow.down.right")
                            .labelStyle(.iconOnly)
                            .padding(14)
                    }
                    .buttonStyle(.borderedProminent)
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
                            if let document { render(document: document, size: lastRenderSize, fullRes: true) }
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

    private func render(document: DrawingDocument?, size: CGSize, fullRes: Bool) {
        guard let document else { return }
        lastRenderSize = size
        guard size.width > 0, size.height > 0 else { return }
        let scale = fullRes ? UIScreen.main.scale : 1
        let width = max(1, Int(size.width * scale))
        let height = max(1, Int(size.height * scale))
        image = try? document.render(width: width, height: height)
    }
}
