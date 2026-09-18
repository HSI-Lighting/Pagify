import CoreGraphics
import Foundation

/// One open DXF/DWG drawing. The handle is closed exactly once, in `deinit`
/// — the same shape as `PagifyDocument`/`ModelDocument`.
///
/// DWG and DXF are not two engines behind this: `drawing::dwg` converts into
/// the exact same `Drawing` the DXF reader produces directly, chosen by
/// `open`'s own extension check — so everything past opening (render, pan,
/// zoom, measure, layers) is one code path regardless of which file came
/// in, and nothing here needs to know which one it opened.
final class DrawingDocument: @unchecked Sendable {
    let handle: Int64
    let name: String

    private let scopedURL: URL?

    init(path: String, name: String? = nil, scopedURL: URL? = nil) throws {
        // Drawings carry no usable font of their own — SHX/Windows fonts
        // never travel with CAD files — so text is drawn with the app's own
        // substituted font, the same one already registered for markup
        // captions. Best-effort: `pagify_open_drawing` tolerates a missing
        // or unreadable font rather than failing the whole open over it.
        PagifyFonts.register(.notoSans)
        let fontName = PagifyFont.notoSans.asset

        let handle = pagify_open_drawing(path, fontName)
        guard handle != PAGIFY_INVALID_HANDLE else {
            scopedURL?.stopAccessingSecurityScopedResource()
            throw PagifyEngine.failure("could not open \(path)")
        }
        self.handle = handle
        self.name = name ?? (path as NSString).lastPathComponent
        self.scopedURL = scopedURL
    }

    deinit {
        _ = pagify_close_drawing(handle)
        scopedURL?.stopAccessingSecurityScopedResource()
    }

    func summary() -> DrawingSummary? {
        guard let json = PagifyEngine.string(pagify_drawing_summary_json(handle)),
              let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(DrawingSummary.self, from: data)
    }

    func layers() -> [DrawingLayer] {
        guard let json = PagifyEngine.string(pagify_drawing_layers_json(handle)),
              let data = json.data(using: .utf8) else { return [] }
        return (try? JSONDecoder().decode([DrawingLayer].self, from: data)) ?? []
    }

    func showLayer(at index: Int, visible: Bool) {
        _ = pagify_show_drawing_layer(handle, Int64(index), visible)
    }

    /// `across`/`down` are fractions of the view (a one-finger drag);
    /// `width`/`height` are the current viewport size in pixels, since the
    /// pan distance in drawing-space depends on the current zoom scale.
    func pan(across: Double, down: Double, width: Int, height: Int) {
        _ = pagify_pan_drawing(handle, Float(across), Float(down), UInt32(width), UInt32(height))
    }

    /// `atX`/`atY` (pixels, e.g. a pinch's midpoint) stay fixed on screen;
    /// `by` is a multiplier, greater than 1 moves closer.
    func zoom(by: Double, atX: Double, atY: Double, width: Int, height: Int) {
        _ = pagify_zoom_drawing(handle, Float(by), Float(atX), Float(atY), UInt32(width), UInt32(height))
    }

    func fit(width: Int, height: Int) {
        _ = pagify_fit_drawing(handle, UInt32(width), UInt32(height))
    }

    /// Places a measure point, snapped to nearby geometry, and returns the
    /// running measurement. A third tap starts over rather than chaining —
    /// the engine's own behaviour, not something this wrapper decides.
    func measure(atX: Double, atY: Double, width: Int, height: Int) -> DrawingMeasurement? {
        guard let json = PagifyEngine.string(
            pagify_measure_drawing_at(handle, Float(atX), Float(atY), UInt32(width), UInt32(height))
        ), let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(DrawingMeasurement.self, from: data)
    }

    func clearMeasure() {
        _ = pagify_clear_drawing_measure(handle)
    }

    /// Rasterise the current view. Same buffer contract as
    /// `PagifyDocument.render(page:scale:)`/`ModelDocument.render(width:height:)`
    /// — RGBA8, premultiplied, big-endian, since `tiny-skia`'s `Pixmap`
    /// (already used for markup on the PDF side) produces exactly that
    /// shape. Hatching draws automatically here — `drawing::raster::draw`
    /// expands a hatch's boundary and pattern into strokes at draw time,
    /// scaled to the current zoom, so there's no separate call to make for
    /// it.
    ///
    /// `by` is how much larger `width`/`height` are than the view `fit`,
    /// `pan` and `zoom` were last called with — 1.0 for the live, on-screen
    /// frame; a capture asks for a bigger bitmap through this same call, at
    /// a `by` matching how much bigger it is. Not optional the way it looks
    /// — see `pagify_render_drawing_into`'s own doc.
    func render(width: Int, height: Int, by: Double = 1) throws -> CGImage {
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

        let outcome = pagify_render_drawing_into(
            handle,
            UInt32(width),
            UInt32(height),
            Float(by),
            pixels.assumingMemoryBound(to: UInt8.self),
            context.bytesPerRow
        )
        guard outcome == PAGIFY_OK else {
            throw PagifyEngine.failure("could not render the drawing")
        }
        guard let image = context.makeImage() else {
            throw PagifyError.engine("the renderer produced no image")
        }
        return image
    }
}

/// Mirrors `DrawingSession::summary_json()`'s own field names exactly (see
/// drawing/session.rs).
struct DrawingSummary: Decodable {
    struct Size: Decodable {
        let x, y: Double
    }
    struct Skipped: Decodable {
        let what: String
        let count: Int
    }

    let name: String
    let shapes: Int
    let layers: Int
    let notShown: Int
    let metresPerUnit: Double
    let unitsDeclared: Bool
    let skipped: [Skipped]
    /// Absent when the drawing has no bounds at all (nothing kept).
    let size: Size?

    /// "1,234 shapes · 3 layers" — matches Android's own subtitle shape.
    var subtitle: String {
        let shapesText = "\(shapes.formatted()) shape" + (shapes == 1 ? "" : "s")
        let layersText = "\(layers) layer" + (layers == 1 ? "" : "s")
        return "\(shapesText) · \(layersText)"
    }

    /// "Not shown: 12 unsupported entities" — nil when nothing was skipped.
    var skippedWarning: String? {
        guard !skipped.isEmpty else { return nil }
        let parts = skipped.map { "\($0.count) \($0.what)" }
        return "Not shown: \(parts.joined(separator: ", "))."
    }
}

/// Mirrors `DrawingSession::layers_json()` — a plain array, one entry per
/// layer, in file order.
struct DrawingLayer: Decodable, Identifiable {
    let name: String
    let visible: Bool
    /// `"#rrggbb"`.
    let colour: String

    var id: String { name }
}

/// Mirrors `DrawingSession::measure_json()`.
struct DrawingMeasurement: Decodable {
    struct Point: Decodable {
        let x, y: Double
        /// `"endpoint" | "intersection" | "midpoint" | "centre" | "free"`.
        let snap: String
    }

    let points: [Point]
    let distance: Double
    let metresPerUnit: Double
    let unitsDeclared: Bool

    /// What to show while measuring — matches Android's own three-state
    /// readout ("Tap a point" → "Tap the second point — first is <snap>" →
    /// the number). Never converted to metres, on purpose: half of these
    /// files never declare real-world units, and a wrong conversion reads as
    /// more confident than the drawing itself is.
    var readout: String {
        switch points.count {
        case 0: return "Tap a point"
        case 1: return "Tap the second point — first is \(points[0].snap)"
        default: return String(format: "%.3f", distance)
        }
    }
}
