import SwiftData
import SwiftUI

/// §A.6's Progress sheet: stage chips, a met-in-person switch, meetings (each
/// with its own cancel button, "arrange another" rather than a single slot),
/// and a follow-up date with offset shortcuts plus a custom fallback.
///
/// Deliberately small, on purpose, not by omission — the doc comment behind
/// this on Android states the reasoning directly: a card scanned standing up
/// at a stand is worth 30 seconds, and a longer form gets skipped, "at which
/// point the CRM holds nothing and is worse than no CRM, because the empty
/// stages look like facts."
struct ProgressSheet: View {
    @Bindable var contact: Contact
    @Environment(\.modelContext) private var modelContext
    @Environment(\.dismiss) private var dismiss

    @State private var addingMeeting = false
    @State private var newMeetingDate = Date.now
    @State private var customFollowUpShowing = false
    @State private var customFollowUpDate = Date.now

    var body: some View {
        NavigationStack {
            List {
                Section("Stage") {
                    stageChips
                    Toggle("Met in person", isOn: $contact.met)
                }

                Section("Meetings") {
                    ForEach(contact.meetings.sorted(by: { $0.at < $1.at })) { meeting in
                        meetingRow(meeting)
                    }
                    Button {
                        newMeetingDate = .now
                        addingMeeting = true
                    } label: {
                        Label("Arrange a Meeting", systemImage: "calendar.badge.plus")
                    }
                }

                Section("Follow Up") {
                    if let followUpAt = contact.followUpAt {
                        HStack {
                            Text(followUpAt, format: .dateTime.day().month().hour().minute())
                            Spacer()
                            if contact.followUpIsDue() {
                                Text("Due").font(.caption).foregroundStyle(.orange)
                            }
                        }
                        Button("Clear", role: .destructive) { contact.setFollowUp(nil) }
                    }
                    HStack {
                        followUpShortcut("3 days", Calendar.current.date(byAdding: .day, value: 3, to: .now))
                        followUpShortcut("Next week", Calendar.current.date(byAdding: .day, value: 7, to: .now))
                        followUpShortcut("A month", Calendar.current.date(byAdding: .month, value: 1, to: .now))
                    }
                    Button("Choose a Date…") {
                        customFollowUpDate = contact.followUpAt ?? .now
                        customFollowUpShowing = true
                    }
                }
            }
            .navigationTitle(contact.name.isEmpty ? "Contact" : contact.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .sheet(isPresented: $addingMeeting) {
                NavigationStack {
                    DatePicker("Meeting Time", selection: $newMeetingDate)
                        .datePickerStyle(.graphical)
                        .padding()
                        .navigationTitle("Arrange a Meeting")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar {
                            ToolbarItem(placement: .cancellationAction) {
                                Button("Cancel") { addingMeeting = false }
                            }
                            ToolbarItem(placement: .confirmationAction) {
                                Button("Add") {
                                    let meeting = Meeting(at: newMeetingDate)
                                    meeting.contact = contact
                                    modelContext.insert(meeting)
                                    addingMeeting = false
                                }
                            }
                        }
                }
                .presentationDetents([.medium])
            }
            .sheet(isPresented: $customFollowUpShowing) {
                NavigationStack {
                    DatePicker("Follow Up", selection: $customFollowUpDate)
                        .datePickerStyle(.graphical)
                        .padding()
                        .navigationTitle("Follow Up")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar {
                            ToolbarItem(placement: .cancellationAction) {
                                Button("Cancel") { customFollowUpShowing = false }
                            }
                            ToolbarItem(placement: .confirmationAction) {
                                Button("Set") {
                                    contact.setFollowUp(customFollowUpDate)
                                    customFollowUpShowing = false
                                }
                            }
                        }
                }
                .presentationDetents([.medium])
            }
        }
    }

    private var stageChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack {
                ForEach(DealStage.allCases) { stage in
                    Button {
                        contact.stage = stage
                    } label: {
                        Text(stage.label)
                            .font(.subheadline.weight(contact.stage == stage ? .semibold : .regular))
                            .padding(.horizontal, 12)
                            .padding(.vertical, 6)
                            .background(
                                contact.stage == stage ? stage.colour.opacity(0.25) : Color(.secondarySystemBackground),
                                in: Capsule()
                            )
                            .foregroundStyle(contact.stage == stage ? stage.colour : Color.primary)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    private func followUpShortcut(_ label: String, _ date: Date?) -> some View {
        Button(label) {
            if let date { contact.setFollowUp(date) }
        }
        .buttonStyle(.bordered)
        .disabled(date == nil)
    }

    @ViewBuilder
    private func meetingRow(_ meeting: Meeting) -> some View {
        HStack {
            VStack(alignment: .leading) {
                Text(meeting.at, format: .dateTime.day().month().hour().minute())
                if meeting.doneAt != nil {
                    Text("Done").font(.caption).foregroundStyle(.secondary)
                } else if meeting.at <= .now {
                    Text("Overdue").font(.caption).foregroundStyle(.orange)
                }
            }
            Spacer()
            Button(role: .destructive) {
                modelContext.delete(meeting)
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
        }
    }
}
