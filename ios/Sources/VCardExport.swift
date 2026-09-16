import Foundation

/// A group export shares one moment across every card in it — see
/// `VCard.write(_:exportedAt:)` — written once, here, to a real `.vcf`
/// file so the share sheet hands Mail, Contacts or AirDrop something
/// they recognise rather than a bare block of text.
///
/// Shared across every export entry point (bulk selection, a single
/// contact's own detail screen, a whole group) rather than reimplemented
/// per screen — the `.vcf`-writing step is the same regardless of how many
/// cards it started from or which screen asked for it.
struct VCardExport: Identifiable {
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

extension VCardExport {
    /// Matches Android's `ContactStore.exportToVCard(chosen, group)`: one
    /// timestamp, stamped on every contact's `exportedAt`/`exportCount` and,
    /// for a group export, the group's own `lastExportedAt` — all from the
    /// same instant, per Android's own reasoning for doing it this way:
    /// "they were sent together, so nothing may disagree about when."
    ///
    /// The stamping is not a side effect of exporting — as far as this
    /// feature is concerned it *is* the export — so every export entry point
    /// (bulk selection, a single contact, a whole group) must go through
    /// this rather than building a `VCardExport` from raw cards directly.
    static func stamping(_ contacts: [Contact], group: ContactGroup? = nil) -> VCardExport {
        let now = Date.now
        for contact in contacts {
            contact.exportedAt = now
            contact.exportCount += 1
        }
        group?.lastExportedAt = now
        return VCardExport(cards: contacts.map(\.asVCard))
    }
}
