import SwiftUI

/// Rearranging, rotating, deleting, adding, importing and exporting pages.
///
/// Pages move by holding one and dragging it. The nudge arrows this replaced
/// were a *document edit* per tap, and every edit re-renders every thumbnail in
/// the grid — so moving a page five places meant five taps and five redraws of
/// the whole sheet. A drag reorders on screen only and commits once, on
/// release: one edit, one re-render, one entry in the undo history.
///
/// Tapping a page points at it. That is where things go — a blank page and an
/// import both land after the page in hand — and without it the only page that
/// could be pointed at was whichever one the reader happened to be on behind
/// the sheet, so choosing where to put something meant closing the organiser,
/// scrolling the document, and opening it again.
///
/// Undo and redo here are the *document's*, not the annotation history's — the
/// two are kept apart deliberately, and this sheet is where document history is
/// shown.
struct PageOrganiser: View {
    let document: PagifyDocument
    let revision: Int
    @ObservedObject var model: ReaderModel

    /// Marks made this session that are not yet in the file.
    ///
    /// Kept apart from `editState` because a highlight does not make the
    /// *document* dirty — marks live in the app's own history until a save writes
    /// them out. Without this the Save button stayed disabled on a document whose
    /// only change was every annotation the reader had drawn on it.
    var unsavedMarks: Int = 0
    var isSaving: Bool = false

    /// The reader's one-shot message, shown here rather than in the snackbar.
    ///
    /// A sheet draws over the reader and over anything the reader is floating on
    /// top of it, so a save that fails while this is open produced a message
    /// nobody could see. That is measured, not guessed: saving a document opened
    /// from another app fails on a read-only grant, and the only sign of it was
    /// in the console.
    var message: String?
    var onMessageShown: (() -> Void)?

    /// What to do with the paper chosen in the blank-page sheet.
    var onAddBlank: ((BlankSheet) -> Void)?

    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var scheme
    @StateObject private var reorder = GridReorderState()
    @State private var open: OrganiserSheet?

    /// The file pages are being taken from.
    ///
    /// Held open only while its picker is on screen — the thumbnails render from
    /// it — and closed whichever way that picker goes. The import itself opens
    /// the file again through the model, which is what carries the pages across
    /// as a document of their own.
    @State private var importSource: PagifyDocument?
    @State private var importURL: URL?

    private var editState: EditState { model.editState }
    private var pageCount: Int { document.pageCount }
    private var enabled: Bool { editState.editable && !isSaving }
    private var hasChanges: Bool { editState.dirty || unsavedMarks > 0 }

    /// A failure outranks a notice: the sheet covers the reader, so this is the
    /// only place an export that could not be written gets to say so.
    private var shownMessage: String? { message ?? model.failure ?? model.notice }

    /// The order the grid draws. Identity except while a page is being dragged,
    /// when it is what the drop would produce — so the pages shuffle under the
    /// finger and the result is visible before letting go.
    private var order: [Int] {
        reorder.order.count == pageCount ? reorder.order : Array(0..<pageCount)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header

            Text("Tap a page to put things after it · hold to drag it")
                .font(.caption)
                .foregroundStyle(.secondary)

            // The grid stays rendered and its buttons go dead, rather than
            // disappearing: a read-only document still has pages worth looking
            // at, and a sheet that opens empty reads as broken.
            if !editState.editable {
                Text("This document cannot be edited.")
                    .font(.body)
                    .padding(.vertical, 12)
            }

            grid

            if let shownMessage {
                banner(shownMessage)
            }

            actions
        }
        .padding(.horizontal, 16)
        .background(PagifyColor.background(scheme))
        // The sheet holds still while a page is held.
        //
        // This is what was actually moving. A sheet is dismissed by dragging it
        // down, and the grid's scroll view is taken out of the gesture the moment
        // a page is picked up — so a downward drag had nothing else to claim it
        // and UIKit handed it to the dismissal. The whole window followed the page
        // down and then closed, which reads exactly like "the window moves with
        // it", and is why disabling the scroller and refusing its pan changed
        // nothing: the scroller was never the thing moving.
        .interactiveDismissDisabled(reorder.slot != nil)
        .sheet(item: $open) { sheet in
            content(of: sheet)
        }
        .onChange(of: open) { _, now in
            // The file is held open only for as long as the picker drawing it
            // is — including when that picker is swiped away rather than
            // cancelled, which reaches no button of ours.
            if now == nil { closeImportSource() }
        }
    }

    private var header: some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Organise pages").font(.headline)
                Text(subtitle).font(.caption)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Button { model.undo() } label: {
                Image(systemName: "arrow.uturn.backward")
            }
            .disabled(!editState.canUndo || isSaving)
            // Naming the specific change is what makes undo safe to press:
            // "Undo" alone gives no way to tell what is about to be reversed.
            .accessibilityLabel(editState.undoLabel.map { "Undo: \($0)" }
                ?? "Undo the last page change")

            Button { model.redo() } label: {
                Image(systemName: "arrow.uturn.forward")
            }
            .disabled(!editState.canRedo || isSaving)
            .accessibilityLabel(editState.redoLabel.map { "Redo: \($0)" }
                ?? "Redo the last undone page change")
            .padding(.leading, 8)
        }
        .padding(.top, 16)
    }

    private var subtitle: String {
        var text = pageCount == 1 ? "1 page" : "\(pageCount) pages"
        if hasChanges { text += " · unsaved changes" }
        return text
    }

    private var grid: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 132), spacing: 12)], spacing: 12) {
                // Identified by the **slot**, not the page, and that is
                // load-bearing twice over. A drag changes which slot holds a
                // page, so identifying cells by page tears down the gesture the
                // instant it succeeds; and a scroll view anchored to a cell that
                // is itself moving chases it down the document. Keyed by slot,
                // the gesture survives and the anchor holds still.
                ForEach(Array(order.indices), id: \.self) { slot in
                    let index = order[slot]
                    PageOrganiserCell(
                        document: document,
                        index: index,
                        // What it would be numbered if the drag ended here.
                        // Outside a drag this is the page number; during one it
                        // is the answer to "where am I putting this".
                        label: slot + 1,
                        revision: revision,
                        isCurrent: index == model.currentPage,
                        dragging: reorder.isDragging(slot: slot),
                        enabled: enabled,
                        canDelete: pageCount > 1,
                        onSelect: { model.jumpTo(index) },
                        onRotate: { model.rotatePage(index) },
                        onDelete: { model.deletePage(index) },
                        // The drag lives on the thumbnail alone. Attached to the
                        // whole cell it also covers rotate and delete, so holding
                        // either of those lifts the page instead of doing nothing.
                        reorder: reorder,
                        slot: slot,
                        pageCount: pageCount
                    )
                    .reorderable(reorder, slot: slot, count: pageCount, enabled: enabled)
                }
            }
            .padding(.vertical, 12)
            .reorderableContent(reorder)
        }
        .reorderableGrid(reorder)
        .onAppear {
            reorder.onMove = { from, to in model.movePage(from: from, to: to) }
        }
    }

    private func banner(_ text: String) -> some View {
        Text(text)
            .font(.body)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.bottom, 8)
            // Cleared on a timer so the next message is seen as a new one rather
            // than blending into the last.
            .task(id: text) {
                try? await Task.sleep(for: .milliseconds(Self.messageDwellMillis))
                guard !Task.isCancelled else { return }
                if let onMessageShown {
                    onMessageShown()
                } else {
                    model.failure = nil
                    model.notice = nil
                }
            }
    }

    /// Two rows: what can be done to the pages, then what can be done with the
    /// document.
    ///
    /// Six buttons across one row fits a tablet and not a phone, and what a
    /// crowded `HStack` does is truncate labels — leaving "Sav…" beside "Clos…"
    /// with no say in which of them lost.
    private var actions: some View {
        VStack(spacing: 4) {
            HStack(spacing: 8) {
                Button { open = .blankPage } label: {
                    Label("Blank", systemImage: "plus")
                }
                .disabled(!enabled)

                Button("Import") { open = .fileToImportFrom }
                    .disabled(!enabled)

                // Offered on a read-only document too: writing chosen pages
                // somewhere else is exactly what you do when you cannot write
                // where they are.
                Button("Export") { open = .pagesToExport }
                    .disabled(isSaving || pageCount == 0)

                Spacer()
            }

            HStack(spacing: 8) {
                Spacer()

                if isSaving {
                    ProgressView().frame(width: 20, height: 20)
                }
                Button("Save a copy") { model.saveCopy() }
                    .disabled(!hasChanges || isSaving)
                // Never gated on there being changes: leaving is always
                // available, and a Close that greys itself out on an unsaved
                // document traps you.
                Button("Close") { dismiss() }
                    .disabled(isSaving)
                Button("Save") { model.save() }
                    .buttonStyle(.borderedProminent)
                    .disabled(!hasChanges || isSaving)
            }
        }
        .padding(.bottom, 16)
    }

    @ViewBuilder
    private func content(of sheet: OrganiserSheet) -> some View {
        switch sheet {
        case .blankPage:
            BlankPageSheet(
                // The page the reader is pointing at, not a cell in this grid: a
                // new sheet that does not match its neighbours reads as a
                // mistake, and the page in hand is the one whose neighbours
                // these are.
                template: try? document.pageSize(model.currentPage),
                onAdd: { paper in
                    open = nil
                    addBlank(paper)
                },
                onDismiss: { open = nil }
            )

        case .pagesToExport:
            PagePicker(
                document: document,
                revision: revision,
                confirmLabel: { $0 == 1 ? "Export 1 page" : "Export \($0) pages" },
                onConfirm: { export($0) },
                onCancel: { open = nil }
            )

        case .exportDestination(let file):
            ExportDestinationPicker(file: file) { discard(file) }

        case .fileToImportFrom:
            DocumentPicker { url in openImportSource(url) }

        case .pagesToImport:
            if let importSource {
                PagePicker(
                    document: importSource,
                    confirmLabel: { $0 == 1 ? "Import 1 page" : "Import \($0) pages" },
                    onConfirm: { importChosen($0) },
                    onCancel: { open = nil }
                )
            }
        }
    }

    /// Insert the paper the reader chose.
    ///
    /// The fallback puts blank pages after the page in hand and drops the chosen
    /// size, colour and ruling, because the only insert the model offers takes a
    /// position and nothing else. Pass `onAddBlank` to route the whole payload to
    /// a model that can carry it.
    private func addBlank(_ sheet: BlankSheet) {
        if let onAddBlank {
            onAddBlank(sheet)
            return
        }
        for offset in 0..<max(1, sheet.count) {
            model.insertBlankPage(after: model.currentPage + offset)
        }
    }

    // ------------------------------------------------------ pages out and in --

    /// Write the chosen pages to a scratch file, then ask where they should go.
    ///
    /// That order, rather than asking first and writing into what comes back:
    /// `UIDocumentPickerViewController` places a file that already exists, so a
    /// picker put up before the bytes were down would export an empty document.
    /// It is also what makes a failed export show a message instead of a file.
    private func export(_ chosen: [Int]) {
        guard !chosen.isEmpty else {
            open = nil
            return
        }

        let scratch = FileManager.default.temporaryDirectory
            .appendingPathComponent(exportName(chosen))
        model.exportPages(chosen, to: scratch)

        // Existence is not the test. The engine creates the file before it can
        // fail, so an export that threw leaves an empty one behind — and handing
        // that to Files is worse than saying nothing, because it looks like it
        // worked.
        let written = (try? scratch.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? 0
        guard written > 0 else {
            try? FileManager.default.removeItem(at: scratch)
            open = nil
            return
        }
        open = .exportDestination(scratch)
    }

    /// What the exported file is called before the reader renames it.
    private func exportName(_ chosen: [Int]) -> String {
        let base = document.name.hasSuffix(".pdf")
            ? String(document.name.dropLast(4))
            : document.name
        let stem = base.isEmpty ? "Document" : base
        return chosen.count == 1
            ? "\(stem) page \(chosen[0] + 1).pdf"
            : "\(stem) \(chosen.count) pages.pdf"
    }

    private func discard(_ scratch: URL) {
        try? FileManager.default.removeItem(at: scratch)
        open = nil
    }

    private func openImportSource(_ url: URL) {
        // Held for as long as the document is: §7.4, the grant covers every read
        // and not merely the open, and PDFium reads pages lazily for the
        // document's whole life.
        let scoped = url.startAccessingSecurityScopedResource()
        do {
            importSource = try PagifyDocument(path: url.path,
                                              name: url.lastPathComponent,
                                              scopedURL: scoped ? url : nil)
            importURL = url
            open = .pagesToImport
        } catch {
            if scoped { url.stopAccessingSecurityScopedResource() }
            model.failure = error.localizedDescription
            open = nil
        }
    }

    private func closeImportSource() {
        // Dropped rather than merely forgotten: releasing the last reference is
        // what closes the document and gives the security-scoped grant back, and
        // the model's own import starts a fresh one on the same URL.
        importSource = nil
        importURL = nil
    }

    private func importChosen(_ chosen: [Int]) {
        guard let url = importURL, !chosen.isEmpty else {
            open = nil
            return
        }
        // After the page being pointed at, which is what "put this here" means
        // when you are looking at one. Appending at the end would be a different
        // request, and one nobody made.
        let at = min(max(model.currentPage + 1, 0), pageCount)

        // Closed before the import, so the two never hold the file at once.
        open = nil
        closeImportSource()
        model.importPages(from: url, indices: chosen, at: at)
    }

    /// How long a message stays on screen in the sheet before it is cleared.
    private static let messageDwellMillis = 4_000
}

/// What the organiser has opened over itself.
///
/// One value rather than a flag per sheet: SwiftUI honours a single `sheet`
/// modifier per view, and five of them stacked on the same stack leaves
/// whichever it prefers unopenable.
private enum OrganiserSheet: Identifiable, Equatable {
    case blankPage
    case pagesToExport
    /// Carries the scratch file so that dismissing the picker — by any route —
    /// knows which one to delete.
    case exportDestination(URL)
    case fileToImportFrom
    case pagesToImport

    var id: String {
        switch self {
        case .blankPage: return "blank"
        case .pagesToExport: return "export"
        case .exportDestination(let file): return "export to \(file.path)"
        case .fileToImportFrom: return "import file"
        case .pagesToImport: return "import pages"
        }
    }
}

/// `UIDocumentPickerViewController(forExporting:)`, `DocumentPicker`'s opposite
/// number: it puts a file we have already written wherever the reader chooses.
///
/// `asCopy: true`, so the scratch file can be deleted afterwards. Exporting it
/// in place *moves* it, and the deletion would then take the reader's exported
/// pages with it.
struct ExportDestinationPicker: UIViewControllerRepresentable {
    let file: URL
    /// Called however the picker goes away — chosen or cancelled — because the
    /// scratch file has to be cleaned up either way.
    let onFinished: () -> Void

    func makeUIViewController(context: Context) -> UIDocumentPickerViewController {
        let picker = UIDocumentPickerViewController(forExporting: [file], asCopy: true)
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ controller: UIDocumentPickerViewController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(onFinished: onFinished) }

    final class Coordinator: NSObject, UIDocumentPickerDelegate {
        private let onFinished: () -> Void
        init(onFinished: @escaping () -> Void) { self.onFinished = onFinished }

        func documentPicker(_ controller: UIDocumentPickerViewController,
                            didPickDocumentsAt urls: [URL]) {
            onFinished()
        }

        func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
            onFinished()
        }
    }
}

/// One page of the grid: a picture, the number it would carry, and the two
/// things that can be done to it in place.
///
/// Rotate and delete only. Moving a page used to be two more buttons here and is
/// now the cell itself — held and dragged — which is both quicker and one edit
/// instead of one per step.
private struct PageOrganiserCell: View {
    let document: PagifyDocument
    let index: Int
    /// The number to show: where this page would sit if a drag ended now.
    let label: Int
    let revision: Int
    let isCurrent: Bool
    let dragging: Bool
    let enabled: Bool
    let canDelete: Bool
    let onSelect: () -> Void
    let onRotate: () -> Void
    let onDelete: () -> Void
    @ObservedObject var reorder: GridReorderState
    let slot: Int
    let pageCount: Int

    @Environment(\.colorScheme) private var scheme
    @Environment(\.displayScale) private var displayScale
    @State private var image: CGImage?

    private var picked: Bool { isCurrent || dragging }

    var body: some View {
        VStack(spacing: 0) {
            ZStack {
                Rectangle().fill(PagifyColor.surfaceVariant(scheme))
                if let image {
                    Image(decorative: image, scale: displayScale)
                        .resizable()
                        .scaledToFit()
                }
            }
            // A fixed shape, not a fixed height: every cell in the grid is the
            // same slot whatever shape the page inside it turns out to be, so the
            // rows line up and a landscape page does not shove its neighbours.
            .aspectRatio(0.72, contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: 4))
            .overlay(
                RoundedRectangle(cornerRadius: 4)
                    .strokeBorder(picked ? PagifyColor.primary(scheme) : Color(.separator),
                                  lineWidth: picked ? 2 : 1)
            )
            .contentShape(RoundedRectangle(cornerRadius: 4))
            .onTapGesture(perform: onSelect)
            .accessibilityElement()
            .accessibilityLabel("Page \(index + 1)")
            .accessibilityHint("Puts a blank page or an import after this one")
            // The drag is fitted here and nowhere else, so a hold on rotate or
            // delete below cannot lift the page.
            .reorderHandle(reorder, slot: slot, count: pageCount, enabled: enabled,
                           onTap: onSelect)

            Text("\(label)").font(.caption2)

            // Two equal-width targets sharing the cell.
            HStack(spacing: 0) {
                button("arrow.clockwise", "Rotate page \(index + 1)", enabled, onRotate)
                button("trash", "Delete page \(index + 1)", enabled && canDelete, onDelete)
            }
        }
        .task(id: PageRenderKey(index: index, revision: revision)) {
            image = nil
            guard let size = await ThumbnailRenderer.shared.size(of: document, page: index),
                  size.width > 0, size.height > 0 else { return }
            image = await ThumbnailRenderer.shared.image(of: document, page: index,
                                                         scale: RenderScale.thumbnailFor(size))
        }
    }

    private func button(_ symbol: String,
                        _ label: String,
                        _ isEnabled: Bool,
                        _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 15))
                .frame(maxWidth: .infinity, minHeight: 34)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(isEnabled ? PagifyColor.onSurface(scheme) : Color.secondary.opacity(0.5))
        .disabled(!isEnabled)
        .accessibilityLabel(label)
    }
}
