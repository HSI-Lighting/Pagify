import SwiftUI

/// What the app opens on: the documents you have read before.
///
/// The reader used to be the front door, which meant every launch began with an
/// empty grey screen and a file picker — the app asking a question it already
/// knew the answer to. Almost every session is the document from last time, or
/// the one before it.
///
/// Only documents that actually opened are here. A file that failed, or that
/// asked for a password and never got one, is not something to offer again as
/// though it worked.
struct LibraryScreen: View {
    let documents: [RecentDocument]
    let onOpen: (RecentDocument) -> Void
    let onForget: (RecentDocument) -> Void
    /// Show the chooser. Both the button that floats over the list and the one
    /// on the empty screen ask this same question — "a document you have, or one
    /// that does not exist yet?" — so neither of them reaches the file picker
    /// directly.
    let onPickDocument: () -> Void

    @Environment(\.colorScheme) private var scheme
    @State private var query = ""
    @FocusState private var searchFocused: Bool

    private var shown: [RecentDocument] {
        searchRecents(documents, query: query)
    }

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            VStack(spacing: 0) {
                header

                Text("Document Library")
                    .font(.title2.weight(.bold))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 20)
                    .padding(.bottom, 12)

                searchField

                // A fixed gap. `Spacer(minLength:)` is flexible, and in a column
                // that fills the screen it grows to swallow whatever is left —
                // pushing the list down past the fold on a tall phone.
                Spacer().frame(height: 12)

                if documents.isEmpty {
                    EmptyLibrary(onPickDocument: onPickDocument)
                } else if shown.isEmpty {
                    NothingMatched(query: query)
                } else {
                    list
                }
            }

            Button(action: onPickDocument) {
                Image(systemName: "plus")
                    .font(.system(size: 22, weight: .semibold))
                    .frame(width: 56, height: 56)
                    .background(PagifyColor.primary(scheme),
                                in: RoundedRectangle(cornerRadius: 17))
                    .foregroundStyle(PagifyColor.onPrimary(scheme))
                    .shadow(color: .black.opacity(0.2), radius: 5, y: 2)
            }
            .padding(20)
            .accessibilityLabel("Add a document")
        }
        .background(PagifyColor.background(scheme))
    }

    private var list: some View {
        ScrollView {
            LazyVStack(spacing: 10) {
                ForEach(shown) { document in
                    Button {
                        onOpen(document)
                    } label: {
                        DocumentRow(document: document)
                    }
                    .buttonStyle(.plain)
                    // Long press is the single path to forgetting a row, the way
                    // it is on Android. A swipe action would be a second one,
                    // and a destructive action with two doors is a destructive
                    // action people find by accident.
                    .contextMenu {
                        Button(role: .destructive) {
                            onForget(document)
                        } label: {
                            Label("Remove from library", systemImage: "trash")
                        }
                    }
                }
            }
            .padding(.horizontal, 16)
            // Room at the bottom for the button that floats over it.
            .padding(.bottom, 96)
        }
    }

    /// The app's name and mark.
    ///
    /// The launcher artwork rather than a second drawing of it: the thing
    /// someone tapped to get here is the thing that should greet them, and two
    /// versions of a logo drift apart the moment one of them is edited.
    private var header: some View {
        HStack(spacing: 12) {
            ZStack {
                Circle().fill(Color(hex: 0x3B00E6))
                Image("LibraryMark")
                    .resizable()
                    .frame(width: logoSize, height: logoSize)
            }
            .frame(width: logoSize, height: logoSize)

            Text("Pagify")
                .font(.title3.weight(.bold))
                .foregroundStyle(PagifyColor.primary(scheme))

            Spacer()

            Button { searchFocused = true } label: {
                Image(systemName: "magnifyingglass")
            }
            .accessibilityLabel("Search the library")
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 16)
    }

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
            TextField("Search files\u{2026}", text: $query)
                .focused($searchFocused)
                .submitLabel(.search)
                // Searching is what the return key means here; there is nothing
                // to submit, so it only has the keyboard to put away.
                .onSubmit { searchFocused = false }
            if !query.isEmpty {
                Button { query = "" } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                }
                .accessibilityLabel("Clear the search")
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 11)
        .background(PagifyColor.surface(scheme), in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14)
            .strokeBorder(Color.secondary.opacity(0.25)))
        .padding(.horizontal, 20)
    }
}

/// One document: what it is called, and enough about it to recognise it.
///
/// Long press offers to forget it. A row can outlive the file it points at —
/// moved, deleted, or a bookmark that no longer resolves — and without a way to
/// remove it the library slowly fills with rows that only ever fail.
private struct DocumentRow: View {
    @Environment(\.colorScheme) private var scheme
    let document: RecentDocument

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: "doc.text")
                .font(.system(size: 20))
                .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                .frame(width: 46, height: 46)
                .background(PagifyColor.surfaceVariant(scheme),
                            in: RoundedRectangle(cornerRadius: 10))
                // The row answers a long press with a menu, and the tile is the
                // only part of it shaped like an icon.
                .overlay(LongPressHint().fill(PagifyColor.onSurfaceVariant(scheme)))

            VStack(alignment: .leading, spacing: 2) {
                Text(document.name)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                    .truncationMode(.tail)

                let subtitle = recentSubtitle(document)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.caption)
                        .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(PagifyColor.surface(scheme), in: RoundedRectangle(cornerRadius: 16))
        .contentShape(Rectangle())
    }
}

/// The mark that says "there is more under this one".
///
/// A small wedge in the bottom-right corner, the way a menu that opens on a long
/// press has been signposted since long before either platform. It exists
/// because a long press is invisible: a control that only answers to one is a
/// control most people never find.
///
/// Drawn rather than a glyph: it is three points, it has to sit exactly in the
/// corner of whatever it marks, and an SF Symbol would bring its own padding and
/// its own baseline to argue with.
private struct LongPressHint: Shape {
    var size: CGFloat = 6
    var inset: CGFloat = 4

    func path(in rect: CGRect) -> Path {
        let right = rect.maxX - inset
        let bottom = rect.maxY - inset

        // A right triangle filling the corner: across, down, and back to the
        // point. The hypotenuse faces up and left, which is what makes it read
        // as an arrow aimed at the corner rather than as a stray dot.
        var path = Path()
        path.move(to: CGPoint(x: right, y: bottom - size))
        path.addLine(to: CGPoint(x: right, y: bottom))
        path.addLine(to: CGPoint(x: right - size, y: bottom))
        path.closeSubpath()
        return path
    }
}

private struct EmptyLibrary: View {
    @Environment(\.colorScheme) private var scheme
    let onPickDocument: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            Text("Nothing here yet")
                .font(.headline)
            Spacer().frame(height: 6)
            Text("Documents you open show up here, newest first.")
                .font(.subheadline)
                .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                .multilineTextAlignment(.center)
            Spacer().frame(height: 16)
            Button("Open a PDF", action: onPickDocument)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(32)
    }
}

/// What a search that found nothing says.
///
/// The query is quoted back untrimmed and unaltered, because the surprise is
/// usually in what was typed — a stray space, a paste that brought a newline —
/// and a tidied-up echo hides exactly the thing that would explain the result.
/// There is no button: nothing here can be opened, and offering the file picker
/// would answer a question nobody asked.
private struct NothingMatched: View {
    @Environment(\.colorScheme) private var scheme
    let query: String

    var body: some View {
        VStack(spacing: 0) {
            Text("No document called \u{201C}\(query)\u{201D}")
                .font(.headline)
                .multilineTextAlignment(.center)
            Spacer().frame(height: 6)
            Text("The library only holds documents you have opened before.")
                .font(.subheadline)
                .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(32)
    }
}

/// The mark in the header, matching the size of a toolbar icon and its label.
private let logoSize: CGFloat = 34
