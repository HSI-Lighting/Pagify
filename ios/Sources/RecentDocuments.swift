import Foundation

/// A document the reader has opened before.
///
/// Held by URL string rather than by path because that is what the app is
/// actually given: a document arrives from the picker or from another app as a
/// URL, and the security-scoped bookmark taken alongside it is what makes it
/// openable again after a relaunch.
///
/// The name, size and page count are copied rather than looked up when the list
/// is drawn. The library has to render instantly and offline, and asking the file
/// system about a couple of dozen files — some of them on iCloud Drive and not
/// downloaded — is neither.
struct RecentDocument: Identifiable, Equatable {
    /// The file's location. Also the identity: reopening the same file from a
    /// different picker session promotes the existing entry rather than adding a
    /// twin.
    let uri: String
    let name: String
    /// Bytes, or 0 when the file system would not say.
    var sizeBytes: Int64
    /// Pages, or 0 when the document failed before it was counted.
    var pageCount: Int
    /// Epoch milliseconds, matching Android's field byte for byte so the two
    /// builds' library files describe the same thing. 0 means "never recorded",
    /// which the subtitle reads as "say nothing" rather than as 1970.
    var openedAtMillis: Int64

    /// The security-scoped bookmark, which is the iOS twin of Android's
    /// `takePersistableUriPermission`.
    ///
    /// **Without this a row is dead the next time the app starts.** The grant the
    /// picker hands over dies with the process, so a path alone reopens as
    /// "permission denied" — and it only shows up after a force-quit, never by
    /// navigating back. Android has no equivalent field, so it is written under a
    /// key of its own and read as optional.
    var bookmark: Data?

    var id: String { uri }
}

/// The list after opening `document`, newest first.
///
/// Pure, and separate from the file it is stored in, because the ordering rules
/// are the part that can be quietly wrong: a document opened twice must move
/// rather than appear twice, and the list must stop growing at some point. Both
/// are invisible until the list is long, which is exactly when nobody is
/// watching.
func promoteRecent(_ existing: [RecentDocument], opening document: RecentDocument,
                   limit: Int = recentDocumentLimit) -> [RecentDocument] {
    var promoted = existing.filter { $0.uri != document.uri }
    promoted.insert(document, at: 0)
    return Array(promoted.prefix(limit))
}

/// How many documents the library remembers.
///
/// Long enough to cover the file you were reading last week, short enough that
/// the list is still something you scan rather than search.
let recentDocumentLimit = 40

/// Drop one entry — for a file that has been moved, deleted, or whose bookmark
/// no longer resolves.
func forgetRecent(_ existing: [RecentDocument], uri: String) -> [RecentDocument] {
    existing.filter { $0.uri != uri }
}

/// The documents whose names match what was typed.
///
/// Case-insensitive and unanchored, so "nda" finds `2024-NDA-final.pdf`. A blank
/// query is not a filter — it returns everything rather than nothing, which is
/// the difference between a search box that is empty and one that has been
/// cleared.
func searchRecents(_ documents: [RecentDocument], query: String) -> [RecentDocument] {
    let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !needle.isEmpty else { return documents }
    return documents.filter { $0.name.range(of: needle, options: .caseInsensitive) != nil }
}

// ----------------------------------------------------------------- labels --

/// A file size someone can read at a glance.
///
/// Binary units, decimal only where it says something: "2.4 MB" is worth the
/// character, "2.4 KB" is noise next to a page count. Written out rather than
/// handed to `ByteCountFormatter`, which counts in powers of ten and rounds down
/// — a 1,000-byte file would read "1 KB" on iOS and "1 KB" on Android for two
/// different reasons, and a 1,500,000-byte one would disagree outright.
func formatFileSize(_ bytes: Int64) -> String {
    if bytes <= 0 { return "" }
    if bytes >= 1_048_576 {
        return String(format: "%.1f MB", Double(bytes) / 1_048_576)
    }
    return "\((bytes + 1023) / 1024) KB"
}

/// When a document was last opened, in the form the library shows.
///
/// The date rather than "3 days ago": a list sorted by recency already says
/// which is newer, so the date is there to be recognised — "that is the one from
/// the October meeting" — and a relative label cannot do that.
func formatOpenedAt(_ millis: Int64) -> String {
    guard millis > 0 else { return "" }
    return openedAtFormatter.string(from: Date(timeIntervalSince1970: Double(millis) / 1000))
}

/// A fixed pattern with localised month names, which is what Android's
/// `SimpleDateFormat("MMM d, yyyy", Locale.getDefault())` also produces.
private let openedAtFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateFormat = "MMM d, yyyy"
    return formatter
}()

/// "Oct 4, 2025  ·  12 pages  ·  2.4 MB", with any part dropped when it is not
/// known — and the whole line dropped by the caller when nothing is.
func recentSubtitle(_ document: RecentDocument) -> String {
    [
        formatOpenedAt(document.openedAtMillis),
        document.pageCount > 0
            ? "\(document.pageCount) page\(document.pageCount == 1 ? "" : "s")"
            : "",
        formatFileSize(document.sizeBytes),
    ]
    .filter { !$0.isEmpty }
    .joined(separator: "  \u{00B7}  ")
}

// ---------------------------------------------------------------- storage --

/// The list as the file holds it.
///
/// Written by hand rather than through `JSONEncoder` so the keys come out in the
/// order Android writes them, and so the bookmark — which Android has no field
/// for — is an addition to that shape rather than a reordering of it. A library
/// file copied between the two builds should read cleanly in both.
func recentsJSON(_ documents: [RecentDocument]) -> String {
    let rows = documents.map { document -> String in
        var fields = [
            "\(jsonQuoted("uri")):\(jsonQuoted(document.uri))",
            "\(jsonQuoted("name")):\(jsonQuoted(document.name))",
            "\(jsonQuoted("sizeBytes")):\(document.sizeBytes)",
            "\(jsonQuoted("pageCount")):\(document.pageCount)",
            "\(jsonQuoted("openedAtMillis")):\(document.openedAtMillis)",
        ]
        if let bookmark = document.bookmark {
            fields.append("\(jsonQuoted("bookmark")):\(jsonQuoted(bookmark.base64EncodedString()))")
        }
        return "{\(fields.joined(separator: ","))}"
    }
    return "[\(rows.joined(separator: ","))]"
}

/// Read the list back, skipping anything that will not parse.
///
/// Lenient per row on purpose, and never all-or-nothing. This file is a
/// convenience, not the user's data — the documents themselves are untouched —
/// so a half-written or older-format entry should cost that one row. Decoding
/// the array as a whole is what turns a single bad row into an empty library,
/// and because the next write then overwrites the file, that loss is permanent.
func recentsFromJSON(_ data: Data) -> [RecentDocument] {
    guard let rows = (try? JSONSerialization.jsonObject(with: data)) as? [Any] else { return [] }

    return rows.compactMap { entry in
        guard let row = entry as? [String: Any] else { return nil }
        guard let uri = row["uri"] as? String, !uri.isEmpty else { return nil }

        let stored = row["name"] as? String
        let name = (stored?.isEmpty == false)
            ? stored!
            : (uri.split(separator: "/").last.map(String.init) ?? uri)

        return RecentDocument(
            uri: uri,
            name: name,
            sizeBytes: (row["sizeBytes"] as? NSNumber)?.int64Value ?? 0,
            pageCount: (row["pageCount"] as? NSNumber)?.intValue ?? 0,
            openedAtMillis: (row["openedAtMillis"] as? NSNumber)?.int64Value ?? 0,
            bookmark: (row["bookmark"] as? String).flatMap { Data(base64Encoded: $0) })
    }
}

/// One string as a JSON literal.
///
/// Hand-rolled because `JSONSerialization` will only encode a whole container,
/// and a container is exactly what loses the key order above.
private func jsonQuoted(_ value: String) -> String {
    var out = "\""
    for scalar in value.unicodeScalars {
        switch scalar {
        case "\"": out += "\\\""
        case "\\": out += "\\\\"
        case "\n": out += "\\n"
        case "\r": out += "\\r"
        case "\t": out += "\\t"
        default:
            if scalar.value < 0x20 {
                out += String(format: "\\u%04x", scalar.value)
            } else {
                out.unicodeScalars.append(scalar)
            }
        }
    }
    return out + "\""
}

/// The library's list of documents, kept across launches.
///
/// A JSON file in the app's own storage rather than a database: it is one flat
/// list of at most a few dozen rows, read once at startup and rewritten when a
/// document is opened. A schema, a migration path and a query language would all
/// be machinery for a problem this does not have.
///
/// Nothing here is the user's data. The documents are wherever they always were;
/// this is a memory of having seen them, and losing it costs a list, not a file.
/// That is why every failure below is swallowed rather than raised.
@MainActor
final class RecentDocumentsStore: ObservableObject {
    @Published private(set) var documents: [RecentDocument] = []

    private let file: URL

    init() {
        let support = FileManager.default.urls(for: .applicationSupportDirectory,
                                               in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: support, withIntermediateDirectories: true)
        file = support.appendingPathComponent("recent-documents.json")
        load()
    }

    private func load() {
        guard let data = try? Data(contentsOf: file) else { return }
        documents = recentsFromJSON(data)
    }

    private func write() {
        try? Data(recentsJSON(documents).utf8).write(to: file, options: .atomic)
    }

    /// Remember a document the reader just opened, taking a bookmark for it.
    func remember(url: URL, name: String, pageCount: Int) {
        // Only a security-scoped URL yields a scoped bookmark, and only inside an
        // access. A file in our own container needs no scope and no bookmark.
        let bookmark = try? url.bookmarkData(options: .minimalBookmark,
                                             includingResourceValuesForKeys: nil,
                                             relativeTo: nil)

        // Two casts, in this order. `attributesOfItem` returns `Any` values, and
        // the size arrives as an `NSNumber` whose Swift bridge is `Int`, not
        // `Int64` — casting straight to `Int64` fails on every file and the size
        // silently reads 0.
        let attributes = try? FileManager.default.attributesOfItem(atPath: url.path)
        let size = (attributes?[.size] as? NSNumber)?.int64Value ?? 0

        documents = promoteRecent(documents, opening: RecentDocument(
            uri: url.absoluteString,
            name: name,
            sizeBytes: size,
            pageCount: pageCount,
            openedAtMillis: Int64(Date().timeIntervalSince1970 * 1000),
            bookmark: bookmark))
        write()
    }

    /// Resolve a row back to a URL that can actually be read.
    ///
    /// A stale bookmark is not an error worth showing: the file has been moved or
    /// deleted, and what the reader wants is to be told the row is dead so it can
    /// be removed, which is what the caller does with `nil`.
    func resolve(_ document: RecentDocument) -> (url: URL, scoped: Bool)? {
        if let bookmark = document.bookmark {
            var stale = false
            if let url = try? URL(resolvingBookmarkData: bookmark, options: [],
                                  relativeTo: nil, bookmarkDataIsStale: &stale) {
                return (url, url.startAccessingSecurityScopedResource())
            }
        }
        guard let url = URL(string: document.uri),
              FileManager.default.fileExists(atPath: url.path) else { return nil }
        return (url, false)
    }

    func forget(_ document: RecentDocument) {
        documents = forgetRecent(documents, uri: document.uri)
        write()
    }

    /// Forget every document. The files themselves are not touched.
    func clear() {
        documents = []
        write()
    }
}
