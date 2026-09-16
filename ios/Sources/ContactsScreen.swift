import SwiftData
import SwiftUI

struct ContactsScreen: View {
    @Environment(\.modelContext) private var modelContext
    @Query(sort: \Contact.capturedAt, order: .reverse) private var contacts: [Contact]

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

    enum ScanResult: Identifiable {
        case saved(count: Int)
        case nothingFound
        case failed(String)

        var id: String {
            switch self {
            case .saved(let count): return "saved-\(count)"
            case .nothingFound: return "nothing"
            case .failed(let message): return "failed-\(message)"
            }
        }
    }

    var body: some View {
        NavigationStack {
            Group {
                if contacts.isEmpty && !isProcessing {
                    ContentUnavailableView(
                        "No Contacts Yet",
                        systemImage: "person.crop.rectangle.badge.plus",
                        description: Text("Scan a business card to get started.")
                    )
                } else {
                    List {
                        ForEach(contacts) { contact in
                            Button {
                                openingContact = contact
                            } label: {
                                HStack(alignment: .top) {
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
                            }
                            // Without this, List gives a Button-labelled row
                            // its default interactive style, which tints the
                            // whole label — including the `.secondary` text —
                            // as if it were a link, `.foregroundStyle(.primary)`
                            // on the name notwithstanding.
                            .buttonStyle(.plain)
                        }
                        .onDelete { indices in
                            for index in indices { modelContext.delete(contacts[index]) }
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
            .navigationTitle("Contacts")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        pickerShowing = true
                    } label: {
                        Label("Scan Card", systemImage: "plus")
                    }
                    .disabled(isProcessing)
                }
            }
            .sheet(isPresented: $pickerShowing) {
                PhotoPicker { data in
                    Task { await process(data) }
                }
                .ignoresSafeArea()
            }
            .sheet(item: $openingContact) { contact in
                ProgressSheet(contact: contact)
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
            .alert(item: $lastResult) { result in
                switch result {
                case .saved(let count):
                    Alert(title: Text(count == 1 ? "Contact Added" : "\(count) Contacts Added"))
                case .nothingFound:
                    Alert(title: Text("No Card Found"),
                          message: Text("Nothing on that photo looked like a business card."))
                case .failed(let message):
                    Alert(title: Text("Could Not Read Card"), message: Text(message))
                }
            }
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

    private func process(_ data: Data) async {
        isProcessing = true
        defer { isProcessing = false }

        guard let image = CardImage.load(data) else {
            lastResult = .failed("That did not look like a photo.")
            return
        }

        do {
            let segments = try await CardTextRecogniser.recognise(image)
            let cards = try CardParser.parse(segments)
            // A parsed card with nothing in it (a photographed page of notes,
            // say) is not worth keeping — matches Android's `worthKeeping`.
            let worthKeeping = cards.filter {
                $0.name != nil || $0.company != nil || !$0.phones.isEmpty || !$0.emails.isEmpty
            }
            guard !worthKeeping.isEmpty else {
                lastResult = .nothingFound
                return
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
