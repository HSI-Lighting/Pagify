import Foundation
import SwiftData

/// A category or an event — `eventDate` tells the two apart. One entity does
/// both jobs rather than a separate "event" concept existing alongside it.
@Model
final class ContactGroup {
    var name: String = ""
    /// `nil` = a plain category, non-`nil` = an event.
    var eventDate: Date?
    var notes: String = ""
    /// Stored as the raw ARGB/RGB value a `Color` is built from, not a
    /// platform colour type — this is a value that has to survive a
    /// migration, and a `Color` does not.
    var colourValue: Int64?
    var createdAt: Date = Date.now
    /// Set when the whole group is exported together, distinct from any one
    /// contact's own `exportedAt`.
    var lastExportedAt: Date?

    /// The other half of `Contact.groups`'s native many-to-many — see the
    /// departure note there. Plain property, no `@Relationship` of its own:
    /// SwiftData resolves the inverse from that side's annotation.
    var contacts: [Contact] = []

    init(name: String = "") {
        self.name = name
    }
}
