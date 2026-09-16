import SwiftData
import SwiftUI

/// §A.8. Reached by a swipe from the contacts list, not a separate tab —
/// closing it shares that same idea: **any sufficiently large horizontal
/// swipe closes it, not only the mirror of whichever gesture opened it.**
/// The first version of this on Android only recognised the same direction
/// that opens the screen, so the drag someone reaches for first — the one
/// that retraces how they got in — did nothing at all. Fixed by treating a
/// close-by-swipe as direction-agnostic, since inside this screen a
/// horizontal drag has no other meaning it could carry.
struct CalendarScreen: View {
    @Environment(\.dismiss) private var dismiss
    @Query private var contacts: [Contact]

    @State private var monthAnchor: Date = .now
    @State private var selectedDay: Date = Calendar.current.startOfDay(for: .now)

    private var calendar: Calendar { Calendar.current }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                monthHeader
                weekdayHeader
                dayGrid
                Divider()
                dueList
            }
            .navigationTitle("Calendar")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { dismiss() }
                }
            }
            // Direction-agnostic on purpose — see the type's own doc comment.
            .gesture(
                DragGesture(minimumDistance: 60)
                    .onEnded { value in
                        if abs(value.translation.width) > abs(value.translation.height) * 1.5 {
                            dismiss()
                        }
                    }
            )
        }
    }

    // MARK: - Month grid

    private var monthHeader: some View {
        HStack {
            Button { shiftMonth(by: -1) } label: { Image(systemName: "chevron.left") }
            Spacer()
            Text(monthAnchor, format: .dateTime.month(.wide).year())
                .font(.headline)
            Spacer()
            Button { shiftMonth(by: 1) } label: { Image(systemName: "chevron.right") }
        }
        .padding()
    }

    private var weekdayHeader: some View {
        let symbols = calendar.veryShortStandaloneWeekdaySymbols
        let ordered = Array(symbols[(calendar.firstWeekday - 1)...] + symbols[..<(calendar.firstWeekday - 1)])
        return HStack {
            ForEach(ordered, id: \.self) { symbol in
                Text(symbol).font(.caption).foregroundStyle(.secondary).frame(maxWidth: .infinity)
            }
        }
    }

    private var dayGrid: some View {
        let days = daysInGrid()
        let columns = Array(repeating: GridItem(.flexible()), count: 7)
        return LazyVGrid(columns: columns, spacing: 8) {
            ForEach(days, id: \.self) { day in
                dayCell(day)
            }
        }
        .padding(.horizontal)
    }

    @ViewBuilder
    private func dayCell(_ day: Date?) -> some View {
        if let day {
            let isSelected = calendar.isDate(day, inSameDayAs: selectedDay)
            let isToday = calendar.isDateInToday(day)
            let hasMeeting = dueMeetings(on: day).isEmpty == false
            let hasFollowUp = dueFollowUps(on: day).isEmpty == false

            Button {
                selectedDay = day
            } label: {
                VStack(spacing: 3) {
                    Text("\(calendar.component(.day, from: day))")
                        .font(.callout.weight(isToday ? .bold : .regular))
                        .frame(width: 30, height: 30)
                        .background(isSelected ? Color.accentColor : .clear, in: Circle())
                        .foregroundStyle(isSelected ? .white : .primary)
                    // Two small dots, not one two-colour fill — a day can be
                    // both a meeting day and a follow-up day at once, and
                    // fills cannot stack; two dots also survive
                    // colour-blindness where a single blended colour would
                    // not.
                    HStack(spacing: 3) {
                        Circle().fill(.orange).frame(width: 5, height: 5).opacity(hasMeeting ? 1 : 0)
                        Circle().fill(.blue).frame(width: 5, height: 5).opacity(hasFollowUp ? 1 : 0)
                    }
                    .frame(height: 5)
                }
            }
            .buttonStyle(.plain)
        } else {
            Color.clear.frame(height: 44)
        }
    }

    // MARK: - Due list

    private var dueList: some View {
        List {
            let meetings = dueMeetings(on: selectedDay)
            let followUps = dueFollowUps(on: selectedDay)

            if meetings.isEmpty && followUps.isEmpty {
                ContentUnavailableView("Nothing on \(selectedDay.formatted(date: .abbreviated, time: .omitted))",
                                       systemImage: "calendar")
            } else {
                if !meetings.isEmpty {
                    Section("Meetings") {
                        ForEach(meetings) { meeting in
                            HStack {
                                Circle().fill(.orange).frame(width: 8, height: 8)
                                Text(meeting.contact?.name ?? "(no name)")
                                Spacer()
                                Text(meeting.at, format: .dateTime.hour().minute())
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                if !followUps.isEmpty {
                    Section("Follow Ups") {
                        ForEach(followUps) { contact in
                            HStack {
                                Circle().fill(.blue).frame(width: 8, height: 8)
                                Text(contact.name.isEmpty ? "(no name)" : contact.name)
                                Spacer()
                                if let at = contact.followUpAt {
                                    Text(at, format: .dateTime.hour().minute()).foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
            }
        }
        .listStyle(.plain)
    }

    // MARK: - Data

    private func dueMeetings(on day: Date) -> [Meeting] {
        contacts.flatMap(\.meetings).filter { calendar.isDate($0.at, inSameDayAs: day) }
    }

    private func dueFollowUps(on day: Date) -> [Contact] {
        contacts.filter { contact in
            guard let at = contact.followUpAt else { return false }
            return calendar.isDate(at, inSameDayAs: day)
        }
    }

    // MARK: - Grid math

    private func shiftMonth(by delta: Int) {
        if let next = calendar.date(byAdding: .month, value: delta, to: monthAnchor) {
            monthAnchor = next
        }
    }

    /// One cell per day of the month, padded with `nil` leading cells so the
    /// first real day lands under its correct weekday column.
    private func daysInGrid() -> [Date?] {
        guard let range = calendar.range(of: .day, in: .month, for: monthAnchor),
              let firstOfMonth = calendar.date(
                from: calendar.dateComponents([.year, .month], from: monthAnchor)
              ) else { return [] }

        let weekday = calendar.component(.weekday, from: firstOfMonth)
        let leadingBlanks = (weekday - calendar.firstWeekday + 7) % 7

        var days: [Date?] = Array(repeating: nil, count: leadingBlanks)
        days += range.compactMap { calendar.date(byAdding: .day, value: $0 - 1, to: firstOfMonth) }
        return days
    }
}
