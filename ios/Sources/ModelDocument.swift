import CoreGraphics
import Foundation

/// One open STEP (3D) model. The handle is closed exactly once, in `deinit`
/// — the same shape as `PagifyDocument`, because the C ABI underneath is the
/// same shape: open-by-path, render-into-a-buffer, close.
///
/// A model is one flattened mesh plus a trackball camera, never a navigable
/// part tree — Android's own STEP assembly handling places every component
/// in world space during tessellation and never exposes it as one, so there
/// is nothing here to port that Android has either.
final class ModelDocument: @unchecked Sendable {
    let handle: Int64
    let name: String

    private let scopedURL: URL?

    init(path: String, name: String? = nil, scopedURL: URL? = nil) throws {
        let handle = pagify_open_model(path)
        guard handle != PAGIFY_INVALID_HANDLE else {
            scopedURL?.stopAccessingSecurityScopedResource()
            throw PagifyEngine.failure("could not open \(path)")
        }
        self.handle = handle
        self.name = name ?? (path as NSString).lastPathComponent
        self.scopedURL = scopedURL
    }

    deinit {
        _ = pagify_close_model(handle)
        scopedURL?.stopAccessingSecurityScopedResource()
    }

    /// Triangle/face counts, bounding-box size, surface-type breakdown and
    /// what was skipped — parsed by `ModelSummary` rather than kept as raw
    /// JSON, so the details panel and the persistent skipped-faces warning
    /// read it the same way.
    func summary() -> ModelSummary? {
        guard let json = PagifyEngine.string(pagify_model_summary_json(handle)),
              let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(ModelSummary.self, from: data)
    }

    /// `across`/`down` are fractions of the view — matches Android's own
    /// `orbit` exactly, including using the view's *width* for both axes so
    /// a non-square viewport doesn't skew one axis against the other.
    func orbit(across: Double, down: Double) {
        _ = pagify_orbit_model(handle, Float(across), Float(down))
    }

    /// `across`/`down` are fractions of the view.
    func pan(across: Double, down: Double) {
        _ = pagify_pan_model(handle, Float(across), Float(down))
    }

    /// `by` > 1 moves closer, matching a pinch's `apart / lastApart` ratio.
    func zoom(by: Double) {
        _ = pagify_zoom_model(handle, Float(by))
    }

    func fit() {
        _ = pagify_fit_model(handle)
    }

    /// Rasterise the current camera view. Same buffer contract as
    /// `PagifyDocument.render(page:scale:)` — RGBA8, premultiplied,
    /// big-endian — since the software rasteriser behind this and PDFium
    /// behind that both write into exactly that shape already.
    func render(width: Int, height: Int) throws -> CGImage {
        let bitmapInfo = CGImageAlphaInfo.premultipliedLast.rawValue
            | CGBitmapInfo.byteOrder32Big.rawValue

        guard let context = CGContext(
            data: nil,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: bitmapInfo
        ), let pixels = context.data else {
            throw PagifyError.engine("could not allocate a \(width)x\(height) bitmap")
        }

        let outcome = pagify_render_model_into(
            handle,
            UInt32(width),
            UInt32(height),
            pixels.assumingMemoryBound(to: UInt8.self),
            context.bytesPerRow
        )
        guard outcome == PAGIFY_OK else {
            throw PagifyEngine.failure("could not render the model")
        }
        guard let image = context.makeImage() else {
            throw PagifyError.engine("the renderer produced no image")
        }
        return image
    }
}

/// Mirrors `ModelSession::summary_json()`'s own field names exactly (see
/// step/session.rs) — one encoding, read by Android's details panel and
/// this one.
struct ModelSummary: Decodable {
    struct Size: Decodable {
        let x, y, z: Double
    }
    struct Surfaces: Decodable {
        let plane, cylinder, cone, torus, sphere, freeform: Int
    }
    struct Skipped: Decodable {
        let what: String
        let count: Int
    }

    let triangles: Int
    let facesInFile: Int
    let facesDrawn: Int
    let skipped: [Skipped]
    let surfaces: Surfaces
    let freeformCurves: Int
    let assembly: Int
    let size: Size

    /// "123 × 45 × 67 mm · 1,234 faces" — matches Android's own subtitle.
    var subtitle: String {
        let facesText = "\(facesDrawn.formatted()) face" + (facesDrawn == 1 ? "" : "s")
        let dims = [size.x, size.y, size.z].map { String(format: "%.0f", $0) }.joined(separator: " × ")
        return "\(dims) mm · \(facesText)"
    }

    /// "Not shown: 12 freeform surfaces, 3 degenerate faces" — nil when
    /// nothing was skipped, matching Android's own persistent warning banner
    /// that only appears when it has something to say.
    var skippedWarning: String? {
        guard !skipped.isEmpty else { return nil }
        let parts = skipped.map { "\($0.count) \($0.what)" }
        return "Not shown: \(parts.joined(separator: ", "))."
    }
}
