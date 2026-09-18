import SwiftUI
import UniformTypeIdentifiers

/// `UIDocumentPickerViewController`, the Storage Access Framework's opposite
/// number.
///
/// `asCopy: false` on purpose: the reader edits the file the user chose, in
/// place, rather than a copy of it that their own file browser will never show
/// the changes to. The cost is that the URL is security-scoped, and §7.4 of the
/// port brief is about what that costs — the grant dies with the process, so
/// access is held for as long as the document is open and a *persisted bookmark*
/// is what makes the file reachable next launch.
struct DocumentPicker: UIViewControllerRepresentable {
    let onPick: (URL) -> Void

    func makeUIViewController(context: Context) -> UIDocumentPickerViewController {
        // Unrestricted, matching Android's own picker (launched with mime
        // filter "*/*") — a file this app doesn't recognise is routed to the
        // PDF reader by `openAny`'s own extension switch and fails there with
        // an ordinary error, which is the same "somebody's mistake, don't
        // hide it" philosophy `RecentKind.of` itself already documents,
        // rather than the picker silently refusing to offer the file at all.
        let picker = UIDocumentPickerViewController(
            forOpeningContentTypes: [.item],
            asCopy: false
        )
        picker.delegate = context.coordinator
        picker.allowsMultipleSelection = false
        return picker
    }

    func updateUIViewController(_ controller: UIDocumentPickerViewController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(onPick: onPick) }

    final class Coordinator: NSObject, UIDocumentPickerDelegate {
        private let onPick: (URL) -> Void
        init(onPick: @escaping (URL) -> Void) { self.onPick = onPick }

        func documentPicker(_ controller: UIDocumentPickerViewController,
                            didPickDocumentsAt urls: [URL]) {
            guard let url = urls.first else { return }
            onPick(url)
        }
    }
}
