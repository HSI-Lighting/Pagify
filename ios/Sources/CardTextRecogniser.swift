import Vision
import UIKit

/// The Android side asks ML Kit for *lines*, not blocks or words, specifically
/// because a block can span a whole column — "handing the parser one segment
/// holding the name, the title and the company at once." Vision's own line
/// grouping (`recognizedText(for:).topCandidates`) gives the same shape, so
/// this stays a straight line-for-line port rather than a redesign.
enum CardTextRecogniser {
    enum Failure: Error {
        case notAnImage
        case recognitionFailed(Error)
    }

    /// Segments in the photograph's own pixel space, unscaled — matching
    /// exactly what `pagify_parse_photographed_card` expects, so no platform
    /// side ever needs its own scaling step (the parser's rules are all
    /// relative position and relative size; the units cancel).
    static func recognise(_ image: UIImage) async throws -> [RecognisedTextSegment] {
        guard let cgImage = image.cgImage else { throw Failure.notAnImage }
        let width = CGFloat(cgImage.width)
        let height = CGFloat(cgImage.height)

        return try await withCheckedThrowingContinuation { continuation in
            // Vision genuinely double-reports some failures: a model-load
            // error ("Could not create inference context") has been measured
            // to reach BOTH the request's own completion handler and the
            // `catch` around `perform(_:)` for the same call — proven by
            // crash log, not inferred — and a continuation resumed twice is
            // a fatal `SWIFT TASK CONTINUATION MISUSE`, not a recoverable
            // error. Both call sites run on this same thread (Vision's
            // completion handler fires synchronously inside `perform`), so
            // a plain flag is enough; no lock is needed for a race that
            // cannot happen.
            var resumed = false
            func resumeOnce(_ body: () -> Void) {
                guard !resumed else { return }
                resumed = true
                body()
            }

            let request = VNRecognizeTextRequest { request, error in
                if let error {
                    resumeOnce { continuation.resume(throwing: Failure.recognitionFailed(error)) }
                    return
                }
                let observations = (request.results as? [VNRecognizedTextObservation]) ?? []
                let segments = observations.compactMap { observation -> RecognisedTextSegment? in
                    guard let candidate = observation.topCandidates(1).first else { return nil }
                    return Self.segment(from: observation.boundingBox, text: candidate.string,
                                        imageWidth: width, imageHeight: height)
                }
                resumeOnce { continuation.resume(returning: segments) }
            }
            // Field extraction wants what the card actually says, not Vision's
            // idea of a plausible dictionary word — a title like "CTO" or a
            // name in a script Vision only partly reads must survive exactly
            // as printed.
            request.usesLanguageCorrection = false

            let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
            do {
                try handler.perform([request])
            } catch {
                resumeOnce { continuation.resume(throwing: Failure.recognitionFailed(error)) }
            }
        }
    }

    /// Vision's `boundingBox` is normalised 0–1 in a **bottom-left** origin
    /// (Core Graphics/PDF convention); the parser wants top-left pixel space
    /// to match Android's coordinate system exactly. Extracted as a pure
    /// function so the conversion itself — the one arithmetic step a whole
    /// card's field positions depend on — can be tested directly against
    /// known boxes rather than only observed indirectly through a live scan.
    static func segment(from box: CGRect, text: String, imageWidth: CGFloat, imageHeight: CGFloat) -> RecognisedTextSegment {
        let left = box.minX * imageWidth
        let top = (1.0 - box.maxY) * imageHeight
        return RecognisedTextSegment(
            left: Float(left),
            top: Float(top),
            right: Float(left + box.width * imageWidth),
            bottom: Float(top + box.height * imageHeight),
            text: text
        )
    }
}
