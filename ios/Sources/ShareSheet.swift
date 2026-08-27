import SwiftUI
import UIKit

/// A file worth handing to `UIActivityViewController`.
struct ShareableFile: Identifiable {
    let url: URL
    var id: String { url.path }
}

/// The system share sheet.
///
/// A recording that cannot be retrieved records nothing. Android's files come off
/// by cable, which is a far safer assumption there than here — so on iOS the file
/// is offered directly the moment it is written, as well as sitting in Documents
/// for the Files app.
struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}
