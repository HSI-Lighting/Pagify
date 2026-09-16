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
}
