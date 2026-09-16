import Foundation

/// The other half of the OCR pipeline's boundary into `rust/pdf_core` —
/// `CardTextRecogniser` produces the segments, this turns them into cards.
/// Card splitting and field extraction are pure text geometry with no
/// platform dependency, which is why they live in Rust once rather than
/// twice: see `contacts::parse` for the algorithm itself.
enum CardParser {
    enum Error: Swift.Error {
        case engine(String)
    }

    private static func lastError() -> String? {
        guard let raw = pagify_last_error_message() else { return nil }
        defer { pagify_string_free(raw) }
        return String(cString: raw)
    }

    /// One photograph can hold several cards — six emptied from a pocket
    /// after an event is the case this exists for — so the result is always
    /// an array, possibly empty, never one bare card.
    static func parse(_ segments: [RecognisedTextSegment]) throws -> [BusinessCard] {
        let json = String(data: try JSONEncoder().encode(segments), encoding: .utf8)!
        return try json.withCString { ptr -> [BusinessCard] in
            guard let raw = pagify_parse_photographed_card(ptr) else {
                throw Error.engine(lastError() ?? "could not parse the photographed card")
            }
            defer { pagify_string_free(raw) }
            let data = Data(String(cString: raw).utf8)
            return try JSONDecoder().decode([BusinessCard].self, from: data)
        }
    }
}
