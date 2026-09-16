import SwiftData
import SwiftUI
import UIKit

struct ContactsScreen: View {
    @Environment(\.modelContext) private var modelContext
    @Query(sort: \Contact.capturedAt, order: .reverse) private var contacts: [Contact]
    @Query private var groups: [ContactGroup]

    /// Groups only once at least one exists — a first-time user with none
    /// sees the flat list, since an empty organisational layer would just be
    /// in the way. Set once, from the count at first appearance
    /// (`hasSetInitialView` guards against `onAppear` re-running this on
    /// every return to the tab and overriding a since-made manual choice);
    /// every subsequent group creation flips it unconditionally instead,
    /// which is the actual fix and not merely this default — see
    /// `GroupsListView.onGroupCreated`.
    @State private var showingGroupsView = false
    @State private var hasSetInitialView = false

    @State private var pickerShowing = false
    @State private var isProcessing = false
    @State private var lastResult: ScanResult?

    /// Cards still waiting for a look, one photo's worth at a time — a photo
    /// of several cards side by side reviews them one after another rather
    /// than all at once.
    @State private var reviewQueue: [(image: UIImage, card: BusinessCard)] = []
    private var currentReview: (image: UIImage, card: BusinessCard)? { reviewQueue.first }

    @State private var savedCount = 0
    @State private var openingContact: Contact?
    @State private var addingManually: Contact?
    @State private var cameraShowing = false
    @State private var calendarShowing = false
    @State private var cameraUnavailable = false

    /// Picked by long press, for deleting or exporting several at once —
    /// matches Android's `pickedContacts` exactly, including the reason it
    /// exists at all: Android has no per-row swipe gesture on this list,
    /// only this and the single delete icon inside a contact's own detail
    /// view, and both are what this file now matches instead of the
    /// SwiftUI-native `.onDelete` row swipe it started with.
    @State private var pickedContacts: Set<PersistentIdentifier> = []
    @State private var confirmingBulkDelete = false
    @State private var exportingSelected: [BusinessCard]?
    private var picking: Bool { !pickedContacts.isEmpty }


    enum ScanResult: Identifiable {
        case saved(count: Int)
        case nothingFound
        /// A QR was read and held something other than a vCard — usually a
        /// plain URL — and the printed side of the card came to nothing
        /// either. Matches Android's `Outcome.NotAContact`: the payload
        /// survives rather than folding into `.nothingFound`, since it may
        /// be the only thing the photo actually yielded.
        case qrNotAContact(String)
        case failed(String)

        var id: String {
            switch self {
            case .saved(let count): return "saved-\(count)"
            case .nothingFound: return "nothing"
            case .qrNotAContact(let payload): return "qr-\(payload)"
            case .failed(let message): return "failed-\(message)"
            }
        }
    }

    var body: some View {
        NavigationStack {
            Group {
                if showingGroupsView {
                    GroupsListView(onGroupCreated: { showingGroupsView = true })
                } else if contacts.isEmpty && !isProcessing {
                    ContentUnavailableView(
                        "No Contacts Yet",
                        systemImage: "person.crop.rectangle.badge.plus",
                        description: Text("Scan a business card to get started.")
                    )
                } else {
                    List {
                        if picking {
                            SelectionHeader(
                                count: pickedContacts.count,
                                onClose: { pickedContacts = [] },
                                onExport: {
                                    exportingSelected = contacts
                                        .filter { pickedContacts.contains($0.persistentModelID) }
                                        .map(\.asVCard)
                                },
                                onDelete: { confirmingBulkDelete = true }
                            )
                        }
                        ForEach(contacts) { contact in
                            let isPicked = pickedContacts.contains(contact.persistentModelID)
                            HStack(alignment: .top) {
                                if picking {
                                    Image(systemName: isPicked ? "checkmark.circle.fill" : "circle")
                                        .foregroundStyle(isPicked ? Color.accentColor : .secondary)
                                }
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(contact.name.isEmpty ? "(no name)" : contact.name)
                                        .font(.headline)
                                        .foregroundStyle(.primary)
                                    if !contact.company.isEmpty {
                                        Text(contact.company)
                                            .font(.subheadline)
                                            .foregroundStyle(.secondary)
                                    }
                                    if let phone = contact.phones.first {
                                        Text(phone.raw)
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                                Spacer()
                                StageBadge(stage: contact.stage)
                            }
                            // `.contentShape` before the gestures: without
                            // it, only the parts of the row that already
                            // draw something (the text, the badge) count as
                            // "in" the row for hit-testing, and the empty
                            // space between them silently swallows both taps
                            // and long presses.
                            .contentShape(Rectangle())
                            // Plain gesture modifiers on the row itself,
                            // deliberately not a `Button` — a `Button`'s own
                            // gesture recogniser was found to reliably beat
                            // an attached `.onLongPressGesture`/
                            // `simultaneousGesture(LongPressGesture(...))`
                            // for the touch, so the long press was silently
                            // never firing at all; reproduced against three
                            // separate simulated long-press mechanisms
                            // before concluding it was this, not the
                            // simulator.
                            .onTapGesture {
                                if picking {
                                    if isPicked { pickedContacts.remove(contact.persistentModelID) }
                                    else { pickedContacts.insert(contact.persistentModelID) }
                                } else {
                                    openingContact = contact
                                }
                            }
                            // Disabled while already picking, matching
                            // Android exactly — a long-press selection is
                            // already a modal state, and a second long press
                            // inside it has nothing new to start.
                            .onLongPressGesture(minimumDuration: 0.5) {
                                guard !picking else { return }
                                pickedContacts = [contact.persistentModelID]
                            }
                        }
                    }
                }
            }
            .overlay {
                if isProcessing {
                    ProgressView("Reading card…")
                        .padding()
                        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
                }
            }
            // Matches Android's own gesture exactly — anywhere on the list,
            // not edge-triggered. An earlier version here required the
            // touch to *start* within 24pt of the screen edge, invented
            // specifically to dodge a collision with a per-row swipe-to-
            // delete this file no longer has (removed when multi-select
            // replaced it — see "Checked Android before writing this one").
            // With that gesture gone, the edge restriction was pure
            // regression: on a real device, iOS's own edge zones (the
            // system back-swipe on the left, in particular) can claim a
            // touch that starts that close to the edge before this
            // recognizer ever sees it, which reads as "the gesture stopped
            // working" — it never had the chance to.
            //
            // `swipeThreshold = 72.dp` on Android; `minimumDistance` here is
            // the SwiftUI-idiomatic way to say "don't even start recognising
            // until a real drag is underway," not a stand-in for it — the
            // actual distance and direction gate is the `guard` below,
            // matching Android's own two-part test: decided as horizontal
            // once travelled is more than twice vertical, then measured
            // against the threshold only at release.
            .simultaneousGesture(
                DragGesture(minimumDistance: 20)
                    .onEnded { value in
                        // Disabled while picking, matching Android: a
                        // long-press selection is already a modal state,
                        // and a stray horizontal drag out of it would
                        // surprise.
                        guard !picking else { return }
                        guard abs(value.translation.width) > 72,
                              abs(value.translation.width) > abs(value.translation.height) * 2 else { return }
                        if value.translation.width > 0 {
                            calendarShowing = true
                        } else {
                            requestCamera()
                        }
                    }
            )
            .navigationTitle("Contacts")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Menu {
                        Button {
                            requestCamera()
                        } label: {
                            Label("Take Photo", systemImage: "camera")
                        }
                        Button {
                            pickerShowing = true
                        } label: {
                            Label("Choose from Library", systemImage: "photo.on.rectangle")
                        }
                        Divider()
                        Button {
                            addingManually = Contact()
                        } label: {
                            Label("Add Manually", systemImage: "square.and.pencil")
                        }
                    } label: {
                        Label("Add Contact", systemImage: "plus")
                    }
                    .disabled(isProcessing)
                }
                ToolbarItem(placement: .principal) {
                    Picker("View", selection: $showingGroupsView) {
                        Text("All").tag(false)
                        Text("Groups").tag(true)
                    }
                    .pickerStyle(.segmented)
                    .fixedSize()
                }
                // Android reaches the calendar BOTH ways at once — a visible
                // IconButton in its own top bar and the edge swipe — never
                // one instead of the other. `.primaryAction` rather than
                // `.secondaryAction` deliberately: the latter can collapse
                // into an overflow menu, and a hidden glyph is not a glyph
                // to tap.
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        calendarShowing = true
                    } label: {
                        Label("Calendar", systemImage: "calendar")
                    }
                }
            }
            .onAppear {
                guard !hasSetInitialView else { return }
                hasSetInitialView = true
                showingGroupsView = !groups.isEmpty
            }
            .sheet(isPresented: $pickerShowing) {
                PhotoPicker { data in
                    Task { await process(data) }
                }
                .ignoresSafeArea()
            }
            .fullScreenCover(isPresented: $cameraShowing) {
                CameraCapture(
                    onCapture: { data in
                        cameraShowing = false
                        Task { await process(data) }
                    },
                    onCancel: { cameraShowing = false }
                )
                .ignoresSafeArea()
            }
            .fullScreenCover(isPresented: $calendarShowing) {
                CalendarScreen()
            }
            .sheet(item: $openingContact) { contact in
                ContactDetailView(contact: contact)
            }
            .sheet(item: $addingManually) { contact in
                ContactDetailView(contact: contact, isNew: true)
            }
            .sheet(item: Binding(
                get: { currentReview.map(ReviewItem.init) },
                set: { _ in }
            )) { item in
                CardReviewSheet(
                    image: item.image,
                    card: item.card,
                    onSave: { edited in
                        modelContext.insert(Contact(from: edited))
                        savedCount += 1
                        advanceQueue()
                    },
                    onCancel: { advanceQueue() }
                )
                // A swipe-to-dismiss that silently discarded the card would
                // look, from the queue's point of view, identical to Cancel
                // but without the review sheet's own confirmation — Save or
                // Cancel are the only two ways off this screen.
                .interactiveDismissDisabled()
            }
            .alert(
                "Delete \(pickedContacts.count) \(pickedContacts.count == 1 ? "Contact" : "Contacts")?",
                isPresented: $confirmingBulkDelete
            ) {
                Button("Cancel", role: .cancel) {}
                Button("Delete", role: .destructive) {
                    for contact in contacts where pickedContacts.contains(contact.persistentModelID) {
                        modelContext.delete(contact)
                    }
                    pickedContacts = []
                }
            }
            .sheet(item: Binding(
                get: { exportingSelected.map { VCardExport(cards: $0) } },
                set: { _ in exportingSelected = nil; pickedContacts = [] }
            )) { export in
                if let url = export.fileURL {
                    ShareSheet(items: [url])
                }
            }
            .alert("No Camera", isPresented: $cameraUnavailable) {
                Button("OK", role: .cancel) {}
            } message: {
                Text("This device has no camera. Choose from Library instead.")
            }
            .alert(item: $lastResult) { result in
                switch result {
                case .saved(let count):
                    Alert(title: Text(count == 1 ? "Contact Added" : "\(count) Contacts Added"))
                case .nothingFound:
                    Alert(title: Text("No Card Found"),
                          message: Text("Nothing on that photo looked like a business card."))
                case .qrNotAContact(let payload):
                    Alert(title: Text("Not a Contact"),
                          message: Text("The QR code on that photo isn't a contact card:\n\(payload)"))
                case .failed(let message):
                    Alert(title: Text("Could Not Read Card"), message: Text(message))
                }
            }
        }
    }

    /// A group export shares one moment across every card in it — see
    /// `VCard.write(_:exportedAt:)` — written once, here, to a real `.vcf`
    /// file so the share sheet hands Mail, Contacts or AirDrop something
    /// they recognise rather than a bare block of text.
    private struct VCardExport: Identifiable {
        let id = UUID()
        let fileURL: URL?

        init(cards: [BusinessCard]) {
            guard let text = try? VCard.write(cards, exportedAt: .now) else {
                fileURL = nil
                return
            }
            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent("Contacts-\(UUID().uuidString)")
                .appendingPathExtension("vcf")
            fileURL = (try? text.write(to: url, atomically: true, encoding: .utf8)) != nil ? url : nil
        }
    }

    /// Wraps the head-of-queue tuple so `.sheet(item:)` has an `Identifiable`
    /// to key off — a plain tuple cannot be, and re-deriving one from
    /// `reviewQueue.first` on every access keeps a single source of truth
    /// instead of a second piece of state that could drift from it.
    private struct ReviewItem: Identifiable {
        let image: UIImage
        let card: BusinessCard
        var id: String { card.rawText }
    }

    /// `UIImagePickerController` does not fail softly when asked for a
    /// source type the device does not have — it throws
    /// `NSInvalidArgumentException: Source type 1 not available` and takes
    /// the app down, confirmed against the real crash log on a simulator (no
    /// camera hardware at all). Checked once, here, rather than left to
    /// `CameraCapture` to discover at the moment it is too late to recover
    /// from.
    private func requestCamera() {
        guard UIImagePickerController.isSourceTypeAvailable(.camera) else {
            cameraUnavailable = true
            return
        }
        cameraShowing = true
    }

    private func advanceQueue() {
        if !reviewQueue.isEmpty { reviewQueue.removeFirst() }
        if reviewQueue.isEmpty && savedCount > 0 {
            lastResult = .saved(count: savedCount)
            savedCount = 0
        }
    }

    /// A card with somewhere on the photo to point at goes to review; one
    /// read entirely off a QR code has no region on any field — nowhere to
    /// point at — and is saved straight through, matching §B.5's own rule
    /// exactly.
    private func needsReview(_ card: BusinessCard) -> Bool {
        card.name?.region != nil || card.title?.region != nil || card.company?.region != nil
            || card.address?.region != nil || card.phones.contains { $0.region != nil }
            || card.emails.contains { $0.region != nil } || card.urls.contains { $0.region != nil }
    }

    /// Two routes, tried in Android's own order and for its own stated
    /// reason: a growing share of cards carry a QR encoding a complete
    /// vCard, and when one does the data is exact — no recognising
    /// letters, no guessing which line is the company. Nothing downstream
    /// can improve on it, so it is tried first and accepted outright,
    /// before OCR ever runs.
    private func process(_ data: Data) async {
        isProcessing = true
        defer { isProcessing = false }

        guard let image = CardImage.load(data) else {
            lastResult = .failed("That did not look like a photo.")
            return
        }

        let barcodes = (try? await CardBarcodeScanner.scan(image)) ?? []

        // Every payload tried, not just the first — a card can carry two
        // codes (one for the vCard, one for a website), and a desk of six
        // cards can carry six vCards, which is six contacts from one photo.
        let fromQR = barcodes.compactMap { try? VCard.read($0) }
        if !fromQR.isEmpty {
            for card in fromQR { modelContext.insert(Contact(from: card)) }
            lastResult = .saved(count: fromQR.count)
            return
        }

        do {
            let segments = try await CardTextRecogniser.recognise(image)
            let cards = try CardParser.parse(segments)
            // A parsed card with nothing in it (a photographed page of notes,
            // say) is not worth keeping — matches Android's `worthKeeping`.
            var worthKeeping = cards.filter {
                $0.name != nil || $0.company != nil || !$0.phones.isEmpty || !$0.emails.isEmpty
            }
            guard !worthKeeping.isEmpty else {
                // A QR that is not a vCard, with nothing on the printed side
                // either, is still worth surfacing — the website it carries
                // may be the only thing the photo yielded at all. Matches
                // Android's `Outcome.NotAContact`.
                if let payload = barcodes.first {
                    lastResult = .qrNotAContact(payload)
                } else {
                    lastResult = .nothingFound
                }
                return
            }

            // Only when there is exactly one card to attach it to — with
            // several cards in frame there is no telling whose code it was.
            if worthKeeping.count == 1, let payload = barcodes.first {
                worthKeeping[0].urls.append(Field(value: payload, confidence: 1))
            }

            var straightThroughCount = 0
            for card in worthKeeping {
                if needsReview(card) {
                    reviewQueue.append((image: image, card: card))
                } else {
                    modelContext.insert(Contact(from: card))
                    straightThroughCount += 1
                }
            }
            if straightThroughCount > 0 && reviewQueue.isEmpty {
                lastResult = .saved(count: straightThroughCount)
            } else if straightThroughCount > 0 {
                savedCount += straightThroughCount
            }
        } catch {
            lastResult = .failed(String(describing: error))
        }
    }
}

private extension Contact {
    /// The scan-result → stored-record direction — the inverse of
    /// `Contact.asVCard`. Only `notes`, unlike the export direction, gets
    /// the parser's overflow text appended: `rawText` alone is for search
    /// (`Contact.searchable`), but a first save should surface anything the
    /// rules could not place, not bury it.
    convenience init(from card: BusinessCard) {
        self.init(
            name: card.name?.value ?? "",
            company: card.company?.value ?? "",
            rawText: card.rawText
        )
        title = card.title?.value ?? ""
        address = card.address?.value ?? ""
        notes = card.notes ?? ""
        phones = card.phones
        emails = card.emails.map(\.value)
        urls = card.urls.map(\.value)
    }
}

/// Long-press selection's own bar, matching Android's `SelectionHeader`: a
/// count, a way out that is not the same gesture that got in (Close), and
/// the two things worth doing to several contacts at once.
private struct SelectionHeader: View {
    let count: Int
    let onClose: () -> Void
    let onExport: () -> Void
    let onDelete: () -> Void

    var body: some View {
        HStack {
            Button(action: onClose) {
                Image(systemName: "xmark")
            }
            Text("\(count) selected")
                .font(.headline)
            Spacer()
            Button(action: onExport) {
                Image(systemName: "square.and.arrow.up")
            }
            Button(action: onDelete) {
                Image(systemName: "trash")
                    .foregroundStyle(.red)
            }
        }
        .buttonStyle(.plain)
        .listRowInsets(EdgeInsets())
        .padding()
        .background(Color(.secondarySystemBackground))
    }
}
