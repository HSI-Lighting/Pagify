import Foundation
import SwiftData
import UserNotifications

/// Getting a reminder in front of somebody who has closed the app.
///
/// Matches Android's own `Reminders.kt` in intent — a meeting gets the
/// urgent treatment, a follow-up stays a quiet line — but not in mechanism,
/// and the difference is a real platform ceiling, not a shortcut taken here.
///
/// **A meeting cannot ring through silent mode or take the lock screen on
/// this side.** Android's version does that with `AlarmManager
/// .setAlarmClock` plus a foreground service holding the alarm audio
/// stream — neither of which iOS lets an ordinary app touch. The one thing
/// that comes close, Apple's Critical Alerts entitlement (the one flag that
/// actually bypasses the mute switch), is granted by Apple case-by-case to
/// a narrow set of categories — health, safety, public alarms — and a
/// business-card CRM does not qualify. This uses the strongest ordinary
/// treatment iOS actually ships instead: `.timeSensitive` interruption
/// level (breaks through Focus modes, though not the mute switch) and a
/// Done/"Ten more minutes" action pair on the notification itself, the
/// same two actions Android's alarm screen offers.
///
/// **One request per reminder, not Android's single-alarm-plus-reschedule
/// dance.** That dance exists on Android because `AlarmManager` grants a
/// process very few wakeup slots; `UNUserNotificationCenter` allows up to
/// 64 pending requests at once, which comfortably covers a personal CRM's
/// near-term meetings and follow-ups, so every reminder here just gets its
/// own request, added and removed as it changes.
enum Reminders {
    static let meetingCategory = "MEETING_REMINDER"
    static let followUpCategory = "FOLLOW_UP_REMINDER"
    static let doneAction = "REMINDER_DONE"
    static let snoozeAction = "REMINDER_SNOOZE"
    /// Matches Android's `SNOOZE_MILLIS`.
    static let snoozeInterval: TimeInterval = 600

    static func registerCategories() {
        let done = UNNotificationAction(identifier: doneAction, title: "Done", options: [])
        let snooze = UNNotificationAction(identifier: snoozeAction, title: "Ten more minutes", options: [])
        UNUserNotificationCenter.current().setNotificationCategories([
            UNNotificationCategory(identifier: meetingCategory, actions: [done, snooze],
                                    intentIdentifiers: [], options: []),
            UNNotificationCategory(identifier: followUpCategory, actions: [done],
                                    intentIdentifiers: [], options: []),
        ])
    }

    static func requestAuthorizationIfNeeded() {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in }
    }

    // MARK: - Scheduling

    static func scheduleMeeting(_ meeting: Meeting) {
        guard let contact = meeting.contact, meeting.doneAt == nil, meeting.at > .now,
              let identifier = identifier(for: meeting.persistentModelID) else { return }

        let content = UNMutableNotificationContent()
        let who = displayName(of: contact)
        content.title = "Meeting with \(who)"
        content.body = contact.company.isEmpty ? "Starting now" : contact.company
        content.sound = .default
        content.categoryIdentifier = meetingCategory
        content.interruptionLevel = .timeSensitive
        content.userInfo = [identifierKey: identifier]

        add(content, identifier: identifier, at: meeting.at)
    }

    static func cancelMeeting(_ meeting: Meeting) {
        guard let identifier = identifier(for: meeting.persistentModelID) else { return }
        remove(identifier)
    }

    static func scheduleFollowUp(for contact: Contact) {
        guard let identifier = identifier(for: contact.persistentModelID) else { return }
        guard let followUpAt = contact.followUpAt, contact.followUpDoneAt == nil,
              followUpAt > .now else {
            remove(identifier)
            return
        }

        let content = UNMutableNotificationContent()
        let who = displayName(of: contact)
        content.title = "Follow up with \(who)"
        content.body = contact.company.isEmpty ? "Time to get back to them" : contact.company
        content.sound = .default
        content.categoryIdentifier = followUpCategory
        content.userInfo = [identifierKey: identifier]

        add(content, identifier: identifier, at: followUpAt)
    }

    static func cancelFollowUp(for contact: Contact) {
        guard let identifier = identifier(for: contact.persistentModelID) else { return }
        remove(identifier)
    }

    /// Safe from anywhere and at any time, matching the promise Android's own
    /// `reschedule` makes — run at launch so a reminder that failed to
    /// schedule (denied permission since granted, a save that raced app
    /// termination) is not silently gone forever.
    static func resyncAll(context: ModelContext) {
        let now = Date.now
        let duePeople = (try? context.fetch(FetchDescriptor<Contact>(
            predicate: #Predicate { $0.followUpAt != nil && $0.followUpDoneAt == nil }
        ))) ?? []
        for contact in duePeople where (contact.followUpAt ?? .distantPast) > now {
            scheduleFollowUp(for: contact)
        }

        let pendingMeetings = (try? context.fetch(FetchDescriptor<Meeting>(
            predicate: #Predicate { $0.doneAt == nil }
        ))) ?? []
        for meeting in pendingMeetings where meeting.at > now {
            scheduleMeeting(meeting)
        }
    }

    // MARK: - Handling a response

    /// Marking something done from the notification's own button, or moving
    /// a meeting back by [snoozeInterval] from "Ten more minutes" — the two
    /// actions Android's `ReminderReceiver` answers, ported to whichever of
    /// `Meeting`/`Contact` the identifier resolves back to.
    static func handle(_ response: UNNotificationResponse, context: ModelContext) {
        guard let identifier = response.notification.request.content.userInfo[identifierKey] as? String,
              let modelID = decode(identifier) else { return }

        if let meeting = context.model(for: modelID) as? Meeting {
            switch response.actionIdentifier {
            case doneAction:
                meeting.doneAt = .now
                remove(identifier)
            case snoozeAction:
                meeting.at = meeting.at.addingTimeInterval(snoozeInterval)
                scheduleMeeting(meeting)
            default:
                break
            }
        } else if let contact = context.model(for: modelID) as? Contact {
            switch response.actionIdentifier {
            case doneAction:
                contact.followUpDoneAt = .now
                remove(identifier)
            default:
                break
            }
        }
    }

    // MARK: - Plumbing

    private static let identifierKey = "pagifyReminderIdentifier"

    private static func displayName(of contact: Contact) -> String {
        if !contact.name.isEmpty { return contact.name }
        if !contact.company.isEmpty { return contact.company }
        return "a contact"
    }

    /// A notification identifier has to be a plain string, and the one
    /// stable handle a SwiftData object offers is its own
    /// `PersistentIdentifier` — `Codable`, so this round-trips it through
    /// JSON rather than through an extra `UUID` field that would exist for
    /// no reason but to be a notification identifier.
    private static func identifier(for modelID: PersistentIdentifier) -> String? {
        guard let data = try? JSONEncoder().encode(modelID) else { return nil }
        return data.base64EncodedString()
    }

    private static func decode(_ identifier: String) -> PersistentIdentifier? {
        guard let data = Data(base64Encoded: identifier) else { return nil }
        return try? JSONDecoder().decode(PersistentIdentifier.self, from: data)
    }

    private static func add(_ content: UNMutableNotificationContent, identifier: String, at date: Date) {
        let interval = max(date.timeIntervalSinceNow, 1)
        let trigger = UNTimeIntervalNotificationTrigger(timeInterval: interval, repeats: false)
        let request = UNNotificationRequest(identifier: identifier, content: content, trigger: trigger)
        UNUserNotificationCenter.current().add(request)
    }

    private static func remove(_ identifier: String) {
        let center = UNUserNotificationCenter.current()
        center.removePendingNotificationRequests(withIdentifiers: [identifier])
        center.removeDeliveredNotifications(withIdentifiers: [identifier])
    }
}

/// Routes a tapped notification action back into `Reminders`. Held as a
/// single long-lived instance — `UNUserNotificationCenter.current().delegate`
/// is a weak reference, so a temporary would vanish before ever being asked
/// anything.
final class ReminderCenterDelegate: NSObject, UNUserNotificationCenterDelegate {
    /// Set once, from `RootView`, which is the first place a `ModelContext`
    /// is actually available — the delegate itself is created before
    /// SwiftData's environment exists.
    var context: ModelContext?

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        // Without this, a notification for a meeting starting while the app
        // is already open would be swallowed silently — the one moment it
        // matters most that it not be.
        completionHandler([.banner, .sound, .list])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        if let context {
            Reminders.handle(response, context: context)
        }
        completionHandler()
    }
}

nonisolated(unsafe) let reminderCenterDelegate = ReminderCenterDelegate()
