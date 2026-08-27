import Foundation

/// Milestone 2 of `docs/IOS_PORT.md`, run in the app rather than reasoned about.
///
/// The house rule for this project is *measure, do not infer*, and pixel order is
/// the case it was written for: PDFium asked for `FPDFBitmap_BGRA` actually emits
/// red in byte 0 on Android/arm64, which is the opposite of what the name says.
/// Nothing about that is guaranteed to hold on a different platform, and getting
/// it wrong tints every document in the app rather than failing.
///
/// So: build a document painted a colour whose channels are all different —
/// orange, `A=FF R=FF G=80 B=00`, the same asymmetric colour the Android value
/// was established with — render it, and look at the bytes. A symmetric colour
/// would pass whichever way round the channels came out, which is why it is not
/// grey.
enum PixelOrderProbe {
    struct Outcome {
        let requested: PagifyPixelOrder
        let bytes: [UInt8]

        /// What those bytes mean, given we know the page is `FF 80 00` orange.
        var reading: String {
            switch (bytes[0], bytes[1], bytes[2]) {
            case (0xFF, 0x80, 0x00): return "R,G,B — as asked"
            case (0x00, 0x80, 0xFF): return "B,G,R — reversed"
            default: return "unrecognised"
            }
        }

        var isAsRequested: Bool {
            switch requested {
            case .rgba: return bytes[0] == 0xFF && bytes[1] == 0x80 && bytes[2] == 0x00
            case .bgra: return bytes[0] == 0x00 && bytes[1] == 0x80 && bytes[2] == 0xFF
            }
        }

        var hex: String {
            bytes.map { String(format: "%02X", $0) }.joined(separator: " ")
        }
    }

    /// Both orders probed, so the answer is a comparison rather than a single
    /// reading that could be right by accident.
    static func run() throws -> [Outcome] {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("pixel-order-probe.pdf")
        try? FileManager.default.removeItem(at: url)

        // O_CREAT|O_WRONLY rather than a FileHandle: the engine adopts the
        // descriptor and closes it on every path, and a FileHandle would close it
        // a second time on deinit.
        let fd = open(url.path, O_CREAT | O_WRONLY | O_TRUNC, 0o644)
        guard fd >= 0 else {
            throw PagifyError.engine("could not create the probe file (errno \(errno))")
        }

        let orange = Int32(bitPattern: 0xFF_FF_80_00 as UInt32)  // A=FF R=FF G=80 B=00
        guard pagify_create_blank_document(fd, 1, 64, 64, orange, 0) == PAGIFY_OK else {
            throw PagifyEngine.failure("could not build the probe document")
        }

        let document = try PagifyDocument(path: url.path, name: "pixel-order-probe.pdf")

        return try [PagifyPixelOrder.rgba, .bgra].map { order in
            Outcome(requested: order, bytes: try sample(document, order: order))
        }
    }

    /// One pixel from the middle of the sheet, away from any edge the renderer
    /// might antialias.
    private static func sample(_ document: PagifyDocument, order: PagifyPixelOrder) throws -> [UInt8] {
        // The sheet is 64pt, so a 64px buffer at zoom 1 means the cache entry and
        // the target agree about the size rather than relying on the engine to
        // reconcile them mid-probe.
        let side = 64
        let stride = side * 4
        var buffer = [UInt8](repeating: 0, count: stride * side)

        // Cleared between orders. The cache is keyed on the zoom, not the byte
        // order, so the second read would otherwise be served the first order's
        // pixels — and a probe that returns a cached answer proves nothing about
        // the path it was meant to measure.
        _ = pagify_clear_cache(document.handle)

        let outcome: Int32 = buffer.withUnsafeMutableBufferPointer { pixels in
            pagify_render_page_into(
                document.handle,
                0,
                1.0,
                0,
                pixels.baseAddress,
                UInt32(side),
                UInt32(side),
                stride,
                order.rawValue
            )
        }
        guard outcome >= 0 else {
            throw PagifyEngine.failure("the probe render failed")
        }

        let middle = (side / 2) * stride + (side / 2) * 4
        return Array(buffer[middle..<(middle + 4)])
    }
}
