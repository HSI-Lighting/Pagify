import Foundation

enum PagifyError: LocalizedError {
    case engine(String)
    /// The document is encrypted. `retry` is true when a password was supplied
    /// and refused, which is a different sentence to show than the first ask.
    case needsPassword(retry: Bool)

    var errorDescription: String? {
        switch self {
        case .engine(let message): return message
        case .needsPassword(let retry):
            return retry ? "That password was not accepted." : "This document is protected."
        }
    }

    /// Classify an engine message.
    ///
    /// PDFium reports "needs a password" and "wrong password" through one
    /// channel, so the only thing that separates them is whether we sent one.
    static func from(_ message: String, passwordTried: Bool) -> PagifyError {
        let lower = message.lowercased()
        if lower.contains("password") {
            return .needsPassword(retry: passwordTried)
        }
        return .engine(message)
    }
}

/// Process-wide engine set-up.
///
/// The only iOS-specific part of the whole bridge: iOS has no system PDFium to
/// find by soname and, at the pinned chromium/7881 tag, no static archive
/// published either — so the app embeds `libpdfium.dylib` and hands the engine
/// the bundle path, which it only learns at runtime.
enum PagifyEngine {
    private static var started = false

    static func start() throws {
        guard !started else { return }
        pagify_init()

        let url = Bundle.main.bundleURL.appendingPathComponent("Frameworks/libpdfium.dylib")
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw PagifyError.engine("libpdfium.dylib is missing from the bundle (\(url.path))")
        }
        guard pagify_set_pdfium_library_path(url.path) == PAGIFY_OK else {
            throw failure("could not point the engine at PDFium")
        }
        started = true
    }

    static var version: String { string(pagify_version()) ?? "unknown" }

    /// The message for the last failure *on this thread*, or nil if it succeeded.
    static func lastError() -> String? { string(pagify_last_error_message()) }

    /// Adopt a `char *` the engine allocated, and release it.
    static func string(_ pointer: UnsafeMutablePointer<CChar>?) -> String? {
        guard let pointer else { return nil }
        defer { pagify_string_free(pointer) }
        return String(cString: pointer)
    }

    /// Wrap whatever the engine last complained about, falling back to a
    /// description of the call when it declined to say.
    static func failure(_ fallback: String) -> PagifyError {
        .engine(lastError() ?? fallback)
    }

    /// The `onTrimMemory` twin. `didReceiveMemoryWarning` is the low case; a
    /// scene leaving the foreground under pressure is the high one.
    static func trimMemory(level: Int32) { pagify_on_trim_memory(level) }
}
