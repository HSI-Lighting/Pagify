import Foundation
import SwiftData

/// Its own model, deliberately, from the start — the reason is a real,
/// shipped Android bug worth not repeating: an earlier schema held one
/// `meetingAt` column directly on `Contact`, so arranging a second meeting
/// silently overwrote the first. A row here is kept, not deleted, once dealt
/// with — that is both history and what stops a done meeting from re-ringing
/// (see `Contact.meetingIsDue`).
@Model
final class Meeting {
    var at: Date
    var doneAt: Date?
    var contact: Contact?

    init(at: Date, doneAt: Date? = nil) {
        self.at = at
        self.doneAt = doneAt
    }
}
