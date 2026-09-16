import SwiftUI
import UIKit

/// The camera route into card scanning — deliberately separate from
/// `PhotoPicker`, which needs no permission at all. This one does
/// (`NSCameraUsageDescription`, Info.plist), and the system shows that
/// prompt the first time this is presented, on its own, the normal way.
///
/// `UIImagePickerController` rather than `AVCaptureSession`: a card scan is
/// one still photo, framed and confirmed once, not a live viewfinder feature
/// with its own UI to build and maintain — the system camera screen already
/// does exactly this.
struct CameraCapture: UIViewControllerRepresentable {
    var onCapture: (Data) -> Void
    var onCancel: () -> Void

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let controller = UIImagePickerController()
        controller.sourceType = .camera
        controller.delegate = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: UIImagePickerController, context: Context) {}

    func makeCoordinator() -> Coordinator {
        Coordinator(onCapture: onCapture, onCancel: onCancel)
    }

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let onCapture: (Data) -> Void
        let onCancel: () -> Void

        init(onCapture: @escaping (Data) -> Void, onCancel: @escaping () -> Void) {
            self.onCapture = onCapture
            self.onCancel = onCancel
        }

        func imagePickerController(
            _ picker: UIImagePickerController,
            didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]
        ) {
            picker.dismiss(animated: true)
            // The original, not the edited crop — `.originalImage` only,
            // since this controller is never configured with
            // `allowsEditing`, and `CardImage`/Vision want the full frame.
            guard let image = info[.originalImage] as? UIImage,
                  let data = image.jpegData(compressionQuality: 0.9) else {
                onCancel()
                return
            }
            onCapture(data)
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
            picker.dismiss(animated: true)
            onCancel()
        }
    }
}
