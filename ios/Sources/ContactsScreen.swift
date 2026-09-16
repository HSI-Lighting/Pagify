import SwiftData
import SwiftUI

/// The minimal end of the pipeline in §B: pick a photo, read it, save what
/// was found. **Deliberately skips the real review sheet** (§B.5 — numbered
/// fields, swipe-to-correct, the two coordinate-space bugs) to get a
/// genuinely working path end to end first; every card found is saved as-is.
/// Wiring in the review step is the next slice, not a hidden requirement of
/// this one.
struct ContactsScreen: View {
    @Environment(\.modelContext) private var modelContext
    @Query(sort: \Contact.capturedAt, order: .reverse) private var contacts: [Contact]

    @State private var pickerShowing = false
    @State private var isProcessing = false
    @State private var lastResult: ScanResult?

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
                            VStack(alignment: .leading, spacing: 2) {
                                Text(contact.name.isEmpty ? "(no name)" : contact.name)
                                    .font(.headline)
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
            for card in worthKeeping {
                modelContext.insert(Contact(from: card))
            }
            lastResult = .saved(count: worthKeeping.count)
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
