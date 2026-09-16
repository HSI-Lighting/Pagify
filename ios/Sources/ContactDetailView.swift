import SwiftData
import SwiftUI

/// §A.10's "one contact form for both create and edit," merged with §A.6's
/// Progress sheet into the single screen tapping a contact opens — Android
/// keeps these as two separate pieces of UI, reached differently; on iOS
/// "tapping a contact shows the details of the contact" reads more naturally
/// as one comprehensive screen than a form that hands off to a second sheet
/// for stage, meetings and follow-up, so those sections live here too.
///
/// The doc comment behind the Android original states why there is exactly
/// one of these, never a separate add-contact form: "a second form for
/// typing in a person would drift from the first the moment either gained a
/// field." The same reasoning is why this file has no sibling.
struct ContactDetailView: View {
    @Bindable var contact: Contact
    /// `true` only for a contact created by "Add Manually" and not yet
    /// inserted into the store — Cancel simply discards the free-standing
    /// object, and Save is the one moment it's written down at all. An
    /// existing contact has no such moment: every field here edits it live,
    /// the same way `ProgressSheet` always did, and "Done" is only a
    /// dismissal.
    let isNew: Bool

    @Environment(\.modelContext) private var modelContext
    @Environment(\.dismiss) private var dismiss

    @State private var addingMeeting = false
    @State private var newMeetingDate = Date.now
    @State private var customFollowUpShowing = false
    @State private var customFollowUpDate = Date.now
    @State private var addingToGroup = false
    @State private var confirmingDelete = false
    /// The already-stamped export, computed once when the toolbar button is
    /// tapped — never lazily inside the sheet's own `Binding`, which would
    /// re-run `VCardExport.stamping` on every re-render the sheet is up for
    /// and bump `exportCount` on its own.
    @State private var exporting: VCardExport?

    init(contact: Contact, isNew: Bool = false) {
        self.contact = contact
        self.isNew = isNew
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Details") {
                    TextField("Name", text: $contact.name)
                    TextField("Title", text: $contact.title)
                    TextField("Company", text: $contact.company)
                    TextField("Address", text: $contact.address, axis: .vertical)
                }

                Section("Phones") {
                    ForEach($contact.phones) { $phone in
                        HStack {
                            Picker("", selection: $phone.kind) {
                                ForEach(PhoneKind.allCases) { kind in
                                    Text(kind.label).tag(kind)
                                }
                            }
                            .labelsHidden()
                            .fixedSize()
                            TextField("Number", text: $phone.raw)
                                .keyboardType(.phonePad)
                                // A hand-edited number no longer matches what
                                // was originally parsed — exporting it
                                // alongside a stale machine reading of the
                                // ORIGINAL text would be worse than exporting
                                // neither, so editing drops it and
                                // `to_vcard` falls back to the raw text
                                // whenever `normalised` is empty.
                                .onChange(of: phone.raw) { _, _ in phone.normalised = "" }
                        }
                    }
                    .onDelete { contact.phones.remove(atOffsets: $0) }
                    Button {
                        contact.phones.append(PhoneField(raw: "", normalised: "", kind: .work, confidence: 1))
                    } label: {
                        Label("Add Phone", systemImage: "plus")
                    }
                }

                Section("Emails") {
                    ForEach($contact.emails.indices, id: \.self) { index in
                        TextField("Email", text: $contact.emails[index])
                            .keyboardType(.emailAddress)
                            .textInputAutocapitalization(.never)
                    }
                    .onDelete { contact.emails.remove(atOffsets: $0) }
                    Button {
                        contact.emails.append("")
                    } label: {
                        Label("Add Email", systemImage: "plus")
                    }
                }

                Section("Websites") {
                    ForEach($contact.urls.indices, id: \.self) { index in
                        TextField("Website", text: $contact.urls[index])
                            .keyboardType(.URL)
                            .textInputAutocapitalization(.never)
                    }
                    .onDelete { contact.urls.remove(atOffsets: $0) }
                    Button {
                        contact.urls.append("")
                    } label: {
                        Label("Add Website", systemImage: "plus")
                    }
                }

                Section("Notes") {
                    TextField("Notes", text: $contact.notes, axis: .vertical)
                }

                Section("Stage") {
                    stageChips
                    Toggle("Met in person", isOn: $contact.met)
                }

                Section("Groups") {
                    ForEach(contact.groups) { group in
                        Text(group.name)
                    }
                    .onDelete { indices in
                        for index in indices {
                            let group = contact.groups[index]
                            contact.groups.removeAll { $0.persistentModelID == group.persistentModelID }
                        }
                    }
                    Button {
                        addingToGroup = true
                    } label: {
                        Label("Add to Group…", systemImage: "folder.badge.plus")
                    }
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

                // Android's ContactSheet keeps these two rows read-only next
                // to its own Export button; there's no separate view screen
                // here to hold them, so — like the rest of ContactSheet —
                // they land in this merged one instead.
                if !isNew {
                    Section {
                        HStack {
                            Text("Added")
                            Spacer()
                            Text(contact.capturedAt, format: .dateTime.day().month())
                                .foregroundStyle(.secondary)
                        }
                        HStack {
                            Text("Exported")
                            Spacer()
                            Text(contact.exportedAt.map { date in
                                let times = contact.exportCount == 1 ? "once" : "\(contact.exportCount) times"
                                return "\(date.formatted(.dateTime.day().month())) · \(times)"
                            } ?? "never")
                            .foregroundStyle(.secondary)
                        }
                    }
                }

                if !isNew {
                    Section {
                        NavigationLink("Recognised Text") {
                            ScrollView {
                                Text(contact.rawText.isEmpty ? "Nothing recorded." : contact.rawText)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .padding()
                            }
                            .navigationTitle("Recognised Text")
                        }
                    }
                }
            }
            .navigationTitle(isNew ? "New Contact" : (contact.name.isEmpty ? "Contact" : contact.name))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if isNew {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { dismiss() }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Save") {
                            modelContext.insert(contact)
                            dismiss()
                        }
                    }
                } else {
                    // Matches Android's ContactSheet trash icon exactly —
                    // there is no separate "Edit" button next to it because
                    // every field on this screen is already live-editable;
                    // Delete is the one action typing into a field cannot
                    // reach.
                    ToolbarItem(placement: .destructiveAction) {
                        Button(role: .destructive) {
                            confirmingDelete = true
                        } label: {
                            Image(systemName: "trash")
                        }
                    }
                    // Matches Android's Share-icon Export button on ContactSheet.
                    ToolbarItem(placement: .primaryAction) {
                        Button {
                            exporting = .stamping([contact])
                        } label: {
                            Image(systemName: "square.and.arrow.up")
                        }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done") { dismiss() }
                    }
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
                // `.large`, not `.medium` — a graphical `DatePicker` showing
                // both the calendar and a time row needs more height than
                // `.medium` gives it on most phones, and the overflow was
                // simply clipped rather than scrollable: the time wheel was
                // in the view hierarchy the whole time, just off the bottom
                // of the sheet, which is exactly what "no option to select a
                // time" looks like from the outside.
                .presentationDetents([.large])
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
                // Same fix as the meeting picker above, same reason: the
                // time row was being clipped by `.medium`, not missing.
                .presentationDetents([.large])
            }
            .sheet(isPresented: $addingToGroup) {
                AddToGroupSheet(contact: contact)
            }
            .sheet(item: $exporting) { export in
                if let url = export.fileURL {
                    ShareSheet(items: [url])
                }
            }
            .alert("Delete \(contact.name.isEmpty ? "This Contact" : contact.name)?", isPresented: $confirmingDelete) {
                Button("Cancel", role: .cancel) {}
                Button("Delete", role: .destructive) {
                    modelContext.delete(contact)
                    dismiss()
                }
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
