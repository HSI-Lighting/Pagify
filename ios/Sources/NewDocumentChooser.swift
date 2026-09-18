import SwiftUI

/// "A document you have, or one that does not exist yet?" — the first
/// question either way into the library asks, matching Android's own
/// `NewDocumentChooser`: a centered card with two rich rows (icon, title, one
/// line saying what the choice actually means), not a plain action-sheet
/// list of labels. SwiftUI's `.confirmationDialog` cannot carry a subtitle or
/// an icon per button, which is why this is a custom overlay rather than
/// that.
struct NewDocumentChooser: View {
    var onBlankPages: () -> Void
    var onOpenFile: () -> Void
    var onCancel: () -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        ZStack {
            Color.black.opacity(0.4)
                .ignoresSafeArea()
                .onTapGesture(perform: onCancel)

            VStack(spacing: 0) {
                Text("Add a document")
                    .font(.headline)
                    .padding(.top, 20)
                    .padding(.bottom, 14)

                VStack(spacing: 4) {
                    ChooserRow(
                        icon: "doc.badge.plus",
                        title: "Blank pages",
                        detail: "Paper to write on: how many, what size and colour",
                        action: onBlankPages)
                    ChooserRow(
                        icon: "folder",
                        title: "Open a file",
                        detail: "PDF, DWG, DXF, STEP",
                        action: onOpenFile)
                }
                .padding(.horizontal, 8)
                .padding(.bottom, 8)

                Divider()

                Button("Cancel", action: onCancel)
                    .font(.body.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 14)
            }
            .background(PagifyColor.surface(scheme), in: RoundedRectangle(cornerRadius: 20))
            .frame(maxWidth: 340)
            .padding(32)
            .shadow(color: .black.opacity(0.25), radius: 20, y: 8)
        }
    }
}

private struct ChooserRow: View {
    @Environment(\.colorScheme) private var scheme
    let icon: String
    let title: String
    let detail: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 14) {
                Image(systemName: icon)
                    .font(.system(size: 20))
                    .foregroundStyle(PagifyColor.primary(scheme))
                    .frame(width: 40, height: 40)

                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.primary)
                    Text(detail)
                        .font(.caption)
                        .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                        .multilineTextAlignment(.leading)
                }

                Spacer(minLength: 0)
            }
            .padding(10)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
