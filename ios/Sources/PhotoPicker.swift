import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// `PHPickerViewController` runs out of process and needs no permission of
/// any kind — Pagify never sees the user's library, only the one image they
/// hand over. This is v1's only capture route on purpose: a live camera is a
/// later addition on an unchanged pipeline, and it will need
/// `NSCameraUsageDescription` and a real permission prompt when it arrives,
/// neither of which this picker requires.
struct PhotoPicker: UIViewControllerRepresentable {
    var onPick: (Data) -> Void

    func makeUIViewController(context: Context) -> PHPickerViewController {
        var configuration = PHPickerConfiguration()
        configuration.filter = .images
        configuration.selectionLimit = 1
        let controller = PHPickerViewController(configuration: configuration)
        controller.delegate = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: PHPickerViewController, context: Context) {}

    func makeCoordinator() -> Coordinator {
        Coordinator(onPick: onPick)
    }

    final class Coordinator: NSObject, PHPickerViewControllerDelegate {
        let onPick: (Data) -> Void
        init(onPick: @escaping (Data) -> Void) { self.onPick = onPick }

        func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
            picker.dismiss(animated: true)
            guard let provider = results.first?.itemProvider,
                  provider.canLoadObject(ofClass: UIImage.self) else { return }
            // `loadDataRepresentation` (rather than `loadObject(ofClass: UIImage.self)`)
            // hands back the original bytes, so HEIC stays HEIC and
            // `CardImage.load` sees the same EXIF orientation data the Photos
            // app itself would.
            provider.loadDataRepresentation(forTypeIdentifier: UTType.image.identifier) { data, _ in
                guard let data else { return }
                DispatchQueue.main.async { self.onPick(data) }
            }
        }
    }
}
