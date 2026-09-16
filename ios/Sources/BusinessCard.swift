import Foundation

/// The wire shape shared with the Rust card parser and vCard writer
/// (`rust/pdf_core/src/contacts/mod.rs`). Field names and casing must match
/// that module's `#[serde(rename_all = "camelCase")]` exactly — this is JSON
/// crossing an FFI boundary, not a Swift-only type, and a mismatch here fails
/// silently as a decode error rather than a compile error.
struct Region: Codable, Equatable {
    var left: Double
    var top: Double
    var right: Double
    var bottom: Double
}

/// One recognised line, in the photograph's own pixel space, unscaled — what
/// `pagify_parse_photographed_card` expects as input. The parser's rules are
/// all relative position and relative text size, so the units cancel and no
/// platform-side scaling step is needed before this crosses the boundary.
struct RecognisedTextSegment: Codable, Equatable {
    var left: Float
    var top: Float
    var right: Float
    var bottom: Float
    var text: String
}

/// A value read off a card, with how sure the recogniser was and, when it came
/// from a photograph, where on it — `region` is `nil` for anything from a QR
/// code or typed by hand, per the Rust doc comment: there is nowhere on the
/// card to point at, and pointing somewhere arbitrary is worse than not
/// pointing at all.
struct Field: Codable, Equatable {
    var value: String
    /// 0.0 to 1.0.
    var confidence: Float
    var region: Region?

    init(value: String, confidence: Float, region: Region? = nil) {
        self.value = value
        self.confidence = confidence
        self.region = region
    }
}

/// Wire values are lowercase (`cell`, `work`, `fax`, `home`) via the Rust
/// enum's own `rename_all = "camelCase"` — NOT the vCard `TYPE=CELL` token,
/// which only the writer produces on the way out. Sending the uppercase form
/// on this side fails the whole card, silently.
enum PhoneKind: String, Codable {
    case cell, work, fax, home
}

struct PhoneField: Codable, Equatable {
    var raw: String
    var normalised: String
    var kind: PhoneKind
    var confidence: Float
    var region: Region?

    init(raw: String, normalised: String, kind: PhoneKind, confidence: Float, region: Region? = nil) {
        self.raw = raw
        self.normalised = normalised
        self.kind = kind
        self.confidence = confidence
        self.region = region
    }
}

/// Everything read off one card. Every field is optional — a stand or a shop
/// often has no personal name — except `rawText`, which is never discarded:
/// the parser will always miss something, a second phone number or a line in
/// a script it does not handle, and this is what makes that recoverable
/// rather than lost.
struct BusinessCard: Codable, Equatable {
    var name: Field?
    var title: Field?
    var company: Field?
    var phones: [PhoneField] = []
    var emails: [Field] = []
    var urls: [Field] = []
    var address: Field?
    var notes: String?
    var rawText: String = ""
}
