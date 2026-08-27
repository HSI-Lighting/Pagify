import CoreGraphics
import Foundation

/// Byte order of the buffer handed to the engine. Core Graphics and PDFium do
/// not agree by default, so the order is stated rather than assumed — and
/// `PixelOrderProbe` checks the statement against a known colour.
enum PagifyPixelOrder: Int32 {
    case rgba = 0
    case bgra = 1
}

/// One open document. The handle is closed exactly once, in `deinit`.
///
/// `@unchecked Sendable` because the engine guards every session with its own
/// lock: the registry hands out a `&mut` only under that lock, which is what
/// makes rendering on a background queue while the UI reads page sizes safe.
final class PagifyDocument: @unchecked Sendable {
    let handle: Int64
    let name: String

    /// Asked of the engine, never cached. It changes under us on every insert and
    /// delete, and a stale copy builds a reorder map of the wrong length.
    var pageCount: Int { max(0, Int(pagify_get_page_count(handle))) }

    /// A security-scoped URL this document must keep alive for as long as it is
    /// open. §7.4: the grant dies with the process, and access has to be held
    /// across every read, not just the open.
    private let scopedURL: URL?

    init(path: String, password: String? = nil, name: String? = nil, scopedURL: URL? = nil) throws {
        let handle = pagify_open_document(path, password)
        guard handle != PAGIFY_INVALID_HANDLE else {
            if let scopedURL { scopedURL.stopAccessingSecurityScopedResource() }
            throw PagifyError.from(PagifyEngine.lastError() ?? "could not open \(path)",
                                   passwordTried: password != nil)
        }

        let count = pagify_get_page_count(handle)
        guard count >= 0 else {
            _ = pagify_close_document(handle)
            if let scopedURL { scopedURL.stopAccessingSecurityScopedResource() }
            throw PagifyEngine.failure("could not count the pages of \(path)")
        }

        self.handle = handle
        self.name = name ?? (path as NSString).lastPathComponent
        self.scopedURL = scopedURL
    }

    deinit {
        _ = pagify_close_document(handle)
        scopedURL?.stopAccessingSecurityScopedResource()
    }

    /// Page size in points. The no-load path — called for every page that
    /// scrolls into view.
    func pageSize(_ index: Int) throws -> CGSize {
        var size = [Float](repeating: 0, count: 2)
        guard pagify_get_page_size(handle, Int32(index), &size) == PAGIFY_OK else {
            throw PagifyEngine.failure("could not measure page \(index)")
        }
        return CGSize(width: CGFloat(size[0]), height: CGFloat(size[1]))
    }

    func metadataJSON() throws -> String {
        guard let json = PagifyEngine.string(pagify_get_metadata_json(handle)) else {
            throw PagifyEngine.failure("could not read the metadata")
        }
        return json
    }

    func text(page index: Int) throws -> String {
        guard let text = PagifyEngine.string(pagify_get_page_text(handle, Int32(index))) else {
            throw PagifyEngine.failure("could not read the text of page \(index)")
        }
        return text
    }

    /// Render a page straight into a Core Graphics bitmap context.
    ///
    /// The context's own dimensions decide the render size; `scale` only
    /// identifies the cache entry, so the app's rounding — not Rust's — settles
    /// how big the target is and the two can never disagree.
    ///
    /// `premultipliedLast | byteOrder32Big` is R, G, B, A in memory, which is
    /// what PDFium already produces, so the engine hands the pixels over rather
    /// than converting them. `PixelOrderProbe` is what establishes that.
    /// Re-render a dragged region as one picture, off screen.
    ///
    /// A capture is **not** a screenshot: no screen pixels are read. The tiles say
    /// which part of which page belongs where, and the engine draws those crops
    /// again at an export scale of their own. Nothing that is not in the document
    /// can appear in the result — not by filtering it out, but because it was
    /// never a source.
    ///
    /// - Parameter markup: marks drawn in the editor, in the capture's own
    ///   coordinates, or nil for the first pass.
    /// - Parameter mask: the lasso ring. Everything outside it is painted over
    ///   with the background, so a detail can be lifted off a busy drawing.
    func captureRegion(_ request: CaptureRequest,
                       markup: [Markup] = [],
                       format: CaptureFormat,
                       scale: CaptureScale) throws -> Data {
        let tiles = request.tilesWireJSON
        let marks = markup.isEmpty ? nil : markupWireJSON(markup)
        let mask = request.mask.isEmpty ? nil : maskWireJSON(request.mask)

        var buffer = PagifyBuffer()
        tiles.withCString { tilesText in
            format.wireName.withCString { formatText in
                func call(_ marksText: UnsafePointer<CChar>?,
                          _ maskText: UnsafePointer<CChar>?) {
                    buffer = pagify_capture_viewport(
                        handle,
                        tilesText,
                        Float(request.width),
                        Float(request.height),
                        Float(scale.factor),
                        Int32(bitPattern: request.background.argb),
                        formatText,
                        Int32(request.quality),
                        marksText,
                        maskText)
                }
                // Nested `withCString` rather than optional pointers built by
                // hand: the buffers only live for the duration of the closure, and
                // a pointer taken out of one is a pointer to freed memory.
                switch (marks, mask) {
                case let (marks?, mask?):
                    marks.withCString { m in mask.withCString { k in call(m, k) } }
                case let (marks?, nil):
                    marks.withCString { m in call(m, nil) }
                case let (nil, mask?):
                    mask.withCString { k in call(nil, k) }
                case (nil, nil):
                    call(nil, nil)
                }
            }
        }

        guard let data = buffer.data else {
            throw PagifyError.engine(PagifyEngine.lastError() ?? "the capture could not be drawn")
        }
        // Copied out before the engine's buffer is released: `Data` here owns its
        // bytes, and the caller keeps it long after this call returns.
        let bytes = Data(bytes: data, count: buffer.len)
        pagify_buffer_free(buffer)
        return bytes
    }

    func render(page index: Int, scale: CGFloat) throws -> CGImage {
        let size = try pageSize(index)
        let width = max(1, Int((size.width * scale).rounded()))
        let height = max(1, Int((size.height * scale).rounded()))

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

        let outcome = pagify_render_page_into(
            handle,
            Int32(index),
            Float(scale),
            0,
            pixels.assumingMemoryBound(to: UInt8.self),
            UInt32(width),
            UInt32(height),
            context.bytesPerRow,
            PagifyPixelOrder.rgba.rawValue
        )
        guard outcome >= 0 else {
            throw PagifyEngine.failure("could not render page \(index)")
        }

        guard let image = context.makeImage() else {
            throw PagifyError.engine("the renderer produced no image for page \(index)")
        }
        return image
    }

    // ------------------------------------------------------------- editing --

    /// Apply one edit and return the resulting state.
    ///
    /// Every mutation goes through here. Adding a tool means adding a `Command`
    /// case, not an FFI entry point.
    @discardableResult
    func execute(_ command: PagifyCommand) throws -> EditState {
        let json = try command.encoded()
        guard let result = PagifyEngine.string(pagify_execute_command_json(handle, json)) else {
            throw PagifyEngine.failure("the edit was refused")
        }
        return EditState(json: result)
    }

    /// An empty history is not an error — the returned state simply still reports
    /// `canUndo == false`, and the UI drives its buttons from that.
    func undo() throws -> EditState {
        guard let json = PagifyEngine.string(pagify_undo_edit(handle)) else {
            throw PagifyEngine.failure("could not undo")
        }
        return EditState(json: json)
    }

    func redo() throws -> EditState {
        guard let json = PagifyEngine.string(pagify_redo_edit(handle)) else {
            throw PagifyEngine.failure("could not redo")
        }
        return EditState(json: json)
    }

    func editState() -> EditState {
        guard let json = PagifyEngine.string(pagify_get_edit_state_json(handle)) else {
            return EditState()
        }
        return EditState(json: json)
    }

    /// The marks already on a page, each with the engine's own index for it.
    func annotations(page index: Int) -> [PlacedAnnotation] {
        guard let json = PagifyEngine.string(pagify_get_annotations_json(handle, Int32(index))) else {
            return []
        }
        return placedAnnotations(fromJSON: json)
    }

    /// The runs of text on a page, with the box each occupies.
    ///
    /// What the highlighter snaps to. Coordinates are in points from the page's
    /// top-left, matching `pageSize`, so the app scales them by the same factor
    /// it renders at.
    func textSegments(page index: Int) -> [TextSegment] {
        guard let json = PagifyEngine.string(
            pagify_get_text_segments_json(handle, Int32(index))) else { return [] }
        return decodeTextSegments(json)
    }

    /// A box for every character on a page.
    ///
    /// Costlier than the runs — a dense page is thousands of boxes — so it is a
    /// separate call, made only when someone actually selects.
    func pageCharacters(page index: Int) -> PageCharacters {
        guard let json = PagifyEngine.string(
            pagify_get_page_characters_json(handle, Int32(index))) else { return .empty }
        return decodePageCharacters(json)
    }

    /// The captions on a page, rebuilt from the blob stored beside each one.
    ///
    /// Text is page content, so it is not in `getAnnotationsJson` — this is the
    /// only way a saved caption is a mark again rather than part of the page, and
    /// the only way the eraser can ever touch one.
    func textMarks(page index: Int) -> [TextMark] {
        guard let json = PagifyEngine.string(pagify_get_text_marks_json(handle, Int32(index))),
              let data = json.data(using: .utf8),
              let blobs = (try? JSONSerialization.jsonObject(with: data)) as? [String] else {
            return []
        }
        return blobs.compactMap { TextMark(restoreJSON: $0) }
    }

    /// Write chosen pages out as their own PDF.
    ///
    /// The order of `indices` is the order they appear in the result — "page 3,
    /// then page 1" is a thing people ask for, and sorting it quietly hands them
    /// a different document.
    func exportPages(_ indices: [Int], to destination: URL) throws {
        let json = try encodeIndices(indices)
        let fd = Darwin.open(destination.path, O_CREAT | O_WRONLY | O_TRUNC, 0o644)
        guard fd >= 0 else {
            throw PagifyError.engine("could not create \(destination.lastPathComponent) (errno \(errno))")
        }
        // Ownership of `fd` transfers to the engine on every path.
        guard pagify_export_pages_to_fd(handle, json, fd) == PAGIFY_OK else {
            throw PagifyEngine.failure("could not export those pages")
        }
    }

    /// Bring another open document's pages into this one, after `at`.
    @discardableResult
    func importPages(from source: PagifyDocument, indices: [Int], at: Int) throws -> EditState {
        let json = try encodeIndices(indices)
        guard let result = PagifyEngine.string(
            pagify_import_pages(handle, source.handle, json, Int32(at))) else {
            throw PagifyEngine.failure("could not import those pages")
        }
        return EditState(json: result)
    }

    private func encodeIndices(_ indices: [Int]) throws -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: indices),
              let json = String(data: data, encoding: .utf8) else {
            throw PagifyError.engine("could not encode the page list")
        }
        return json
    }

    /// Write the document back to the file it came from.
    ///
    /// **Via a scratch file, never in place.** PDFium reads objects lazily for a
    /// document's whole life, so a save streams *from* the source while writing;
    /// pointing both ends at one file truncates the input halfway through and
    /// produces a PDF that is neither the old one nor the new one. Android does
    /// the same dance in `PdfRepository.writeTo`.
    ///
    /// `incremental` appends a delta and leaves the original bytes intact, which
    /// keeps any existing digital signature valid.
    func save(to destination: URL, incremental: Bool = true) throws {
        let scratch = FileManager.default.temporaryDirectory
            .appendingPathComponent("pagify-save-\(UUID().uuidString).pdf")

        let fd = open(scratch.path, O_CREAT | O_WRONLY | O_TRUNC, 0o644)
        guard fd >= 0 else {
            throw PagifyError.engine("could not open a scratch file (errno \(errno))")
        }
        // Ownership of `fd` transfers to the engine on every path, including the
        // failing ones — so there is nothing to close here.
        guard pagify_save_to_fd(handle, fd, incremental) == PAGIFY_OK else {
            try? FileManager.default.removeItem(at: scratch)
            throw PagifyEngine.failure("could not write the document")
        }

        do {
            // Replacing rather than writing over: the source is still open, and
            // the whole point of the scratch file is that the two never touch.
            _ = try FileManager.default.replaceItemAt(destination, withItemAt: scratch)
        } catch {
            try? FileManager.default.removeItem(at: scratch)
            throw PagifyError.engine("could not replace \(destination.lastPathComponent): \(error.localizedDescription)")
        }
    }

    func setCacheBudget(bytes: Int64) {
        _ = pagify_set_cache_budget_bytes(handle, bytes)
    }

    func cacheStatsJSON() -> String? {
        PagifyEngine.string(pagify_get_cache_stats_json(handle))
    }
}
