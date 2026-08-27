import SwiftUI

/// What to do with the text that is selected. Android's
/// `ui/components/TextSelectionBar.kt`.
///
/// Anchored above the tool ribbon rather than floating over the selection, which
/// is where a context menu would normally go. A menu at the finger covers the
/// words it belongs to — and this selection can run over several lines, so there
/// is no "beside it" that is not on top of something.
///
/// The count is there because a selection dragged across a column break can pick
/// up far more than it appears to, and a number is the cheapest way to notice
/// before it lands on the clipboard.
struct TextSelectionBar: View {
    let characters: Int
    let onCopy: () -> Void
    let onHighlight: () -> Void
    let onDismiss: () -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: 4) {
            Text("\(characters) selected")
                .font(.footnote.weight(.medium))
                .lineLimit(1)
                .layoutPriority(1)
                .padding(.leading, 6)
                .padding(.trailing, 2)

            Button(action: onCopy) {
                Label("Copy", systemImage: "doc.on.doc")
            }
            .padding(.horizontal, 4)

            Button(action: onHighlight) {
                Label("Highlight", systemImage: "highlighter")
            }
            .padding(.horizontal, 4)

            Button(action: onDismiss) {
                Image(systemName: "xmark")
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel("Clear the selection")
        }
        // Never abbreviated. "Highl…" is not a word, and a bar that truncates
        // its own verbs is worse than one that is a little wider.
        .font(.footnote)
        .lineLimit(1)
        .fixedSize(horizontal: true, vertical: false)
        .labelStyle(.titleAndIcon)
        .buttonStyle(.borderless)
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(PagifyColor.surfaceVariant(scheme),
                    in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .shadow(color: .black.opacity(scheme == .dark ? 0.5 : 0.18), radius: 8, y: 3)
    }
}
