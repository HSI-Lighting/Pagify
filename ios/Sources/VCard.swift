import Foundation

/// The vCard boundary into `rust/pdf_core`'s `contacts` module
/// (`pagify_vcard` / `pagify_vcards` / `pagify_vcard_parse`). Card
/// storage, review, and the CRM live entirely on the Swift side — this is
/// only the three functions that have to cross into Rust: write one card,
/// write a group of them sharing one export moment, and read a scanned QR
/// payload back.
enum VCard {
    enum Error: Swift.Error {
        case engine(String)
    }

    private static func lastError() -> String? {
        guard let raw = pagify_last_error_message() else { return nil }
        defer { pagify_string_free(raw) }
        return String(cString: raw)
    }

    private static func exportedAtString(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.timeZone = TimeZone(identifier: "UTC")
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.string(from: date)
    }

    /// Render one card as a single-`VCARD` vCard 3.0 file.
    static func write(_ card: BusinessCard, exportedAt: Date) throws -> String {
        let cardJSON = String(data: try JSONEncoder().encode(card), encoding: .utf8)!
        return try cardJSON.withCString { cardPtr in
            try exportedAtString(exportedAt).withCString { datePtr in
                guard let raw = pagify_vcard(cardPtr, datePtr) else {
                    throw Error.engine(lastError() ?? "could not render the vCard")
                }
                defer { pagify_string_free(raw) }
                return String(cString: raw)
            }
        }
    }

    /// Render many cards as one file, concatenated `VCARD` blocks sharing a
    /// single `REV` — a group export leaves everyone in it at the same
    /// moment.
    static func write(_ cards: [BusinessCard], exportedAt: Date) throws -> String {
        let cardsJSON = String(data: try JSONEncoder().encode(cards), encoding: .utf8)!
        return try cardsJSON.withCString { cardsPtr in
            try exportedAtString(exportedAt).withCString { datePtr in
                guard let raw = pagify_vcards(cardsPtr, datePtr) else {
                    throw Error.engine(lastError() ?? "could not render the vCards")
                }
                defer { pagify_string_free(raw) }
                return String(cString: raw)
            }
        }
    }

    /// Read a scanned QR payload as a vCard. `nil` is an ordinary result —
    /// most QR codes on a business card hold a plain URL, not a contact — and
    /// is returned with no error message set. A thrown `Error` means the
    /// engine itself failed, which is a different thing entirely and should
    /// be surfaced, not silently treated as "not a vCard."
    static func read(_ text: String) throws -> BusinessCard? {
        try text.withCString { textPtr -> BusinessCard? in
            guard let raw = pagify_vcard_parse(textPtr) else {
                // Ordinary "not a vCard" leaves no message; a real engine
                // failure does. Only the second is worth throwing over.
                if let message = lastError() {
                    throw Error.engine(message)
                }
                return nil
            }
            defer { pagify_string_free(raw) }
            let json = Data(String(cString: raw).utf8)
            return try JSONDecoder().decode(BusinessCard.self, from: json)
        }
    }
}
