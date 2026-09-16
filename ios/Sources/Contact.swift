import Foundation
import SwiftData

/// Fixed, not user-definable — a user-editable stage set would need
/// identity-tracked migrations for renames. Matches
/// `com.hsilighting.pagify.core.DealStage` on Android exactly.
enum DealStage: String, Codable, CaseIterable, Identifiable {
    case new, contacted, meeting, quoted, won, lost

    var id: String { rawValue }

    var isClosed: Bool { self == .won || self == .lost }
}

/// A card, and everything that has happened with the person on it since.
///
/// Stored fields are plain values, never the `Field`/`PhoneField` confidence
/// wrappers `BusinessCard` uses — confidence belongs to the scan/review step,
/// and by the time a card is saved its fields are facts, not guesses. `phones`
/// carries `PhoneField` only because its `kind` and `normalised` value are
/// genuinely part of the stored record, not a confidence score.
@Model
final class Contact {
    var name: String = ""
    var title: String = ""
    var company: String = ""
    var address: String = ""
    var notes: String = ""

    /// Everything the recogniser produced, never discarded — this is what
    /// makes "search finds a field the parser missed" work (`searchable`,
    /// below).
    var rawText: String = ""

    var phones: [PhoneField] = []
    var emails: [String] = []
    var urls: [String] = []

    var capturedAt: Date = Date.now
    /// `nil` = never exported, deliberately distinct from "exported at the
    /// epoch."
    var exportedAt: Date?
    var exportCount: Int = 0

    /// The actual persisted value. Stored as the stage's own name, not a
    /// position, so inserting a stage in the middle can never silently
    /// renumber existing rows — see `stage` below for the read side of this.
    private var stageRaw: String = DealStage.new.rawValue

    /// An unrecognised stored value (a stage this build has never heard of —
    /// SwiftData has no equivalent of Room's destructive-migration escape
    /// hatch, but a future case added and later removed would land here the
    /// same way) reads as `.new` rather than crashing the fetch.
    var stage: DealStage {
        get { DealStage(rawValue: stageRaw) ?? .new }
        set { stageRaw = newValue.rawValue }
    }

    /// Met face to face vs. card handed on — orthogonal to `stage`.
    var met: Bool = false

    var followUpAt: Date?
    var followUpDoneAt: Date?

    @Relationship(deleteRule: .cascade, inverse: \Meeting.contact)
    var meetings: [Meeting] = []

    /// Native SwiftData many-to-many, not an explicit join entity.
    ///
    /// **Deliberate departure from Android's `group_membership` table.** Room
    /// needs an explicit join entity for many-to-many at all; SwiftData does
    /// not, and models it as a true many-to-many relationship directly. The
    /// persisted semantics are identical: deleting a group removes it from
    /// every contact's `groups` here exactly as deleting a `group_membership`
    /// row does there, and neither ever touches `Contact` or `ContactGroup`
    /// itself. No `deleteRule` is set here on purpose — the default
    /// (`.nullify`, i.e. "stop pointing at it") is correct; `.cascade` would
    /// wrongly delete every contact in a group the moment the group itself
    /// was deleted.
    @Relationship(inverse: \ContactGroup.contacts)
    var groups: [ContactGroup] = []

    init(name: String = "", company: String = "", rawText: String = "") {
        self.name = name
        self.company = company
        self.rawText = rawText
    }

    /// Computed on every call, never a stored flag — there is no moment to
    /// recompute it in that would not eventually go stale relative to the
    /// wall clock.
    func followUpIsDue(asOf now: Date = .now) -> Bool {
        guard let followUpAt, followUpDoneAt == nil else { return false }
        return followUpAt <= now
    }

    /// As `followUpIsDue`, across every meeting rather than one column — a
    /// second meeting can be arranged without disturbing the first, which is
    /// the entire reason meetings are their own relationship (see
    /// `Meeting.swift`) and not a single column here.
    func meetingIsDue(asOf now: Date = .now) -> Bool {
        meetings.contains { $0.doneAt == nil && $0.at <= now }
    }

    /// Moving or clearing the follow-up date must reset `followUpDoneAt` —
    /// otherwise an old "done" stamp silently stops the new date from ever
    /// coming due, correct in storage and never firing. Go through this
    /// rather than assigning `followUpAt` directly so no call site can forget.
    func setFollowUp(_ date: Date?) {
        followUpAt = date
        followUpDoneAt = nil
    }

    /// Concatenates every text field, including `rawText`, specifically so a
    /// phone number the parser failed to classify correctly is still findable
    /// by search even though no structured field holds it. Lowercased here,
    /// matching Android's own `searchable` exactly, so every call site can
    /// compare it against a lowercased query with a plain `contains` rather
    /// than each one remembering to fold case itself.
    var searchable: String {
        ([name, title, company, address, notes, rawText]
            + phones.map(\.raw) + emails + urls)
            .joined(separator: " ")
            .lowercased()
    }

    /// Builds the `BusinessCard` the vCard writer expects. Every field is a
    /// stored fact here, so confidence is always 1.0. Whitespace-only fields
    /// are dropped rather than exported blank, so an importer does not create
    /// a contact with an empty company — the same reason a hand-corrected
    /// phone number drops its stale normalised value in the review sheet.
    var asVCard: BusinessCard {
        func nonBlank(_ text: String) -> String? {
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? nil : trimmed
        }
        func field(_ text: String) -> Field? {
            nonBlank(text).map { Field(value: $0, confidence: 1) }
        }
        return BusinessCard(
            name: field(name),
            title: field(title),
            company: field(company),
            phones: phones,
            emails: emails.compactMap(nonBlank).map { Field(value: $0, confidence: 1) },
            urls: urls.compactMap(nonBlank).map { Field(value: $0, confidence: 1) },
            address: field(address),
            notes: nonBlank(notes),
            rawText: rawText
        )
    }
}
