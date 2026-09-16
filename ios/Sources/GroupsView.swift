import SwiftData
import SwiftUI

/// The groups half of §A.9. **Merging two groups is deliberately not built
/// here** — Android's own `merge` exists at the store/DAO level, tested, but
/// has no wired-up UI entry point either; carrying that gap forward is a
/// decision, not an oversight, and it can be built properly whenever
/// something actually calls for it rather than guessed at now.
struct GroupsListView: View {
    @Environment(\.modelContext) private var modelContext
    @Query(sort: \ContactGroup.name) private var groups: [ContactGroup]
    @Query private var allContacts: [Contact]

    /// Called after a group is created, so the parent screen can force its
    /// Groups/All toggle to Groups — unconditionally, on every creation, not
    /// only relied on as a side effect of the empty→non-empty default. That
    /// unconditional push is the actual fix behind two real, identically
    /// reported Android bugs: creating a group while already on the flat
    /// list left the list on the flat list, where the new group is invisible,
    /// and it only ever reproduced from the SECOND group onward — the first
    /// one happened to flip the (then merely computed) default on its own,
    /// which is exactly what made it look intermittent rather than always
    /// broken.
    var onGroupCreated: () -> Void

    @State private var addingGroup = false
    @State private var newGroupName = ""
    /// The already-stamped export, computed once when the swipe action
    /// fires — never lazily inside the sheet's own `Binding`, which would
    /// re-run `VCardExport.stamping` on every re-render the sheet is up for
    /// and bump every member's `exportCount` on its own.
    @State private var exportingGroup: VCardExport?

    private var ungroupedCount: Int { allContacts.filter { $0.groups.isEmpty }.count }

    var body: some View {
        // No navigationTitle/toolbar of its own — rendered as the content of
        // ContactsScreen's existing "Contacts" navigation entry rather than
        // pushed as a separate one, so the Scan Card action stays reachable
        // regardless of which half of the Groups/All toggle is showing.
        List {
            Section {
                Button {
                    newGroupName = ""
                    addingGroup = true
                } label: {
                    Label("New Group", systemImage: "folder.badge.plus")
                }

                ForEach(groups) { group in
                    HStack {
                        NavigationLink {
                            GroupDetailView(group: group)
                        } label: {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(group.name)
                                    .font(.headline)
                                Text(group.rowSubtitle)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        Spacer()
                        // A menu, not a swipe: this list already has a
                        // whole-screen left/right drag gesture for the
                        // calendar and camera (see ContactsScreen), and it
                        // claims a horizontal drag anywhere on the list
                        // before a row's own swipe action ever sees it —
                        // confirmed by reproducing it, not assumed. A tap
                        // target has no direction to collide with. Matches
                        // Android's own per-row "⋮" IconButton + dropdown
                        // more closely than a swipe would have anyway.
                        Menu {
                            Button {
                                exportingGroup = .stamping(group.contacts, group: group)
                            } label: {
                                Label("Export", systemImage: "square.and.arrow.up")
                            }
                            Button(role: .destructive) {
                                modelContext.delete(group)
                            } label: {
                                Label("Delete", systemImage: "trash")
                            }
                        } label: {
                            Image(systemName: "ellipsis.circle")
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                }

                if ungroupedCount > 0 {
                    NavigationLink {
                        UngroupedContactsView()
                    } label: {
                        HStack {
                            Text("Ungrouped")
                            Spacer()
                            Text("\(ungroupedCount)").foregroundStyle(.secondary)
                        }
                    }
                }
            } footer: {
                // Stated to the user explicitly, not left implicit — matches
                // the confirmation dialog's own wording on Android: deleting
                // a group only ever removes the grouping, never a contact.
                Text("Deleting a group never deletes the contacts in it — they move to Ungrouped.")
            }
        }
        .alert("New Group", isPresented: $addingGroup) {
            TextField("Name", text: $newGroupName)
            Button("Cancel", role: .cancel) {}
            Button("Create") {
                let trimmed = newGroupName.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !trimmed.isEmpty else { return }
                modelContext.insert(ContactGroup(name: trimmed))
                onGroupCreated()
            }
        }
        .sheet(item: $exportingGroup) { export in
            if let url = export.fileURL {
                ShareSheet(items: [url])
            }
        }
    }
}

private extension ContactGroup {
    /// Matches Android's own `GroupRow` subtitle exactly: the member count
    /// on its own until this group has ever been exported as a whole, then
    /// " · sent <date>" appended once `lastExportedAt` is set.
    var rowSubtitle: String {
        let count = contacts.count
        let countPhrase = count == 1 ? "1 contact" : "\(count) contacts"
        guard let lastExportedAt else { return countPhrase }
        return "\(countPhrase) · sent \(lastExportedAt.formatted(.dateTime.day().month().year()))"
    }
}

struct GroupDetailView: View {
    @Bindable var group: ContactGroup
    @Environment(\.modelContext) private var modelContext

    @State private var searchText = ""
    /// Scoped to this group's own members, not the whole store — matches
    /// Android's own `pool = openGroup?.let(inGroup) ?: contacts`.
    /// `Contact.searchable` is already lowercased, so only the query needs
    /// folding here.
    private var shown: [Contact] {
        let trimmed = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return group.contacts }
        return group.contacts.filter { $0.searchable.contains(trimmed.lowercased()) }
    }

    /// Separate from the outer list row's own "…" menu — Android gives the
    /// open group's header its own export entry point too, for exactly this
    /// group's current members.
    @State private var exportingGroup: VCardExport?

    var body: some View {
        List {
            ForEach(shown) { contact in
                ContactRow(contact: contact)
            }
            .onDelete { indices in
                // Removes the grouping only — the contact itself is
                // untouched, same guarantee as deleting the group entirely.
                // Indexed into `shown`, not `group.contacts` — the two can
                // disagree in order and length once a search is narrowing
                // what's on screen.
                for index in indices {
                    shown[index].groups.removeAll { $0.persistentModelID == group.persistentModelID }
                }
            }
        }
        .searchable(text: $searchText, prompt: "Search \(group.name)…")
        .navigationTitle(group.name)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    exportingGroup = .stamping(group.contacts, group: group)
                } label: {
                    Label("Export", systemImage: "square.and.arrow.up")
                }
            }
        }
        .overlay {
            if group.contacts.isEmpty {
                ContentUnavailableView("No Contacts", systemImage: "person.2",
                                       description: Text("File a contact into this group from its Progress screen."))
            }
        }
        .sheet(item: $exportingGroup) { export in
            if let url = export.fileURL {
                ShareSheet(items: [url])
            }
        }
    }
}

struct UngroupedContactsView: View {
    @Query private var allContacts: [Contact]
    private var ungrouped: [Contact] { allContacts.filter { $0.groups.isEmpty } }

    var body: some View {
        List(ungrouped) { contact in
            ContactRow(contact: contact)
        }
        .navigationTitle("Ungrouped")
    }
}

/// Filing an existing contact into a group. **Rebuilt from a real Android
/// bug worth not repeating**: the first version of this sheet listed groups
/// the contact was ALREADY in, and tapping one removed it — so a card
/// scanned before any group existed could never be filed into one
/// afterward. Filtered here to the complement on purpose: only groups the
/// contact is not already in are offered at all.
struct AddToGroupSheet: View {
    let contact: Contact
    @Environment(\.dismiss) private var dismiss
    @Environment(\.modelContext) private var modelContext
    @Query(sort: \ContactGroup.name) private var allGroups: [ContactGroup]

    @State private var addingGroup = false
    @State private var newGroupName = ""

    private var available: [ContactGroup] {
        let current = Set(contact.groups.map(\.persistentModelID))
        return allGroups.filter { !current.contains($0.persistentModelID) }
    }

    var body: some View {
        NavigationStack {
            List {
                if available.isEmpty {
                    ContentUnavailableView("No Other Groups", systemImage: "folder",
                                           description: Text("Every existing group already has this contact."))
                } else {
                    ForEach(available) { group in
                        Button(group.name) {
                            contact.groups.append(group)
                            dismiss()
                        }
                    }
                }
            }
            .navigationTitle("Add to Group")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        newGroupName = ""
                        addingGroup = true
                    } label: {
                        Label("New Group", systemImage: "folder.badge.plus")
                    }
                }
            }
            .alert("New Group", isPresented: $addingGroup) {
                TextField("Name", text: $newGroupName)
                Button("Cancel", role: .cancel) {}
                Button("Create") {
                    let trimmed = newGroupName.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !trimmed.isEmpty else { return }
                    let group = ContactGroup(name: trimmed)
                    modelContext.insert(group)
                    contact.groups.append(group)
                    dismiss()
                }
            }
        }
    }
}

/// The same row rendering the flat All list uses — pulled out once both
/// GroupDetailView and UngroupedContactsView needed it, rather than
/// re-diverging two copies of it under separate edits later.
struct ContactRow: View {
    let contact: Contact
    @State private var opening = false

    var body: some View {
        Button {
            opening = true
        } label: {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(contact.name.isEmpty ? "(no name)" : contact.name)
                        .font(.headline)
                        .foregroundStyle(.primary)
                    if !contact.company.isEmpty {
                        Text(contact.company).font(.subheadline).foregroundStyle(.secondary)
                    }
                }
                Spacer()
                StageBadge(stage: contact.stage)
            }
        }
        .buttonStyle(.plain)
        .sheet(isPresented: $opening) {
            ContactDetailView(contact: contact)
        }
    }
}
