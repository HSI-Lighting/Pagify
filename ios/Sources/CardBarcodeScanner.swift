import Vision
import UIKit

/// The other half of §B.2's two routes, tried in the order Android tries
/// them and for the same reason: a growing share of cards carry a QR
/// encoding a complete vCard, and when one does the data is exact — no
/// recognising letters, no guessing which line is the company. Nothing
/// downstream can improve on it, so it is tried first and accepted outright.
///
/// All symbologies, not `.qr` alone — Vision detects every format it
/// supports when `symbologies` is left unset, matching ML Kit's default
/// `BarcodeScanning.getClient()`, which does the same. A vCard can just as
/// well ride on a Data Matrix or a PDF417.
enum CardBarcodeScanner {
    /// Every payload decoded from the photo, not just the first — a card
    /// can carry two codes (one for the vCard, one for a website), and a
    /// desk of six cards can carry six vCards at once, which is the best
    /// version of this feature: six contacts, no recognition, exact data.
    static func scan(_ image: UIImage) async throws -> [String] {
        guard let cgImage = image.cgImage else { return [] }

        return try await withCheckedThrowingContinuation { continuation in
            // Reproduced by crash log, not inferred: a Vision model-load
            // failure ("Could not create inference context") reached both
            // this completion handler AND the `catch` around `perform(_:)`
            // for the same call, and a continuation resumed twice is a
            // fatal `SWIFT TASK CONTINUATION MISUSE` — see the identical
            // guard and its longer explanation in `CardTextRecogniser`.
            var resumed = false
            func resumeOnce(_ body: () -> Void) {
                guard !resumed else { return }
                resumed = true
                body()
            }

            let request = VNDetectBarcodesRequest { request, error in
                if let error {
                    resumeOnce { continuation.resume(throwing: error) }
                    return
                }
                let observations = (request.results as? [VNBarcodeObservation]) ?? []
                resumeOnce { continuation.resume(returning: observations.compactMap(\.payloadStringValue)) }
            }
            let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
            do {
                try handler.perform([request])
            } catch {
                resumeOnce { continuation.resume(throwing: error) }
            }
        }
    }
}
