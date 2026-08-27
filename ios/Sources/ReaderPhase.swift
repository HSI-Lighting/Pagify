import Foundation

/// What the reader is doing. Android's `PdfReaderState.Phase`.
///
/// A state machine rather than a scattering of booleans, because the interesting
/// cases are the ones that are not "a document is open": a protected file that
/// needs a password is not a failure and not an empty reader, and treating it as
/// either is how an encrypted PDF became simply un-openable.
enum ReaderPhase: Equatable {
    case empty
    case loading
    case ready
    /// `retry` is true once a password has been offered and refused.
    case passwordRequired(retry: Bool)
    case failed(String)

    var isReady: Bool { self == .ready }
}
