import SwiftUI

/// The two things `+` can mean.
///
/// Asked rather than assumed, because they are not variations of one action:
/// opening a file you have and making paper you do not are different intentions
/// that happen to start from the same button.
struct NewDocumentChooser: View {
    let onBlankPages: () -> Void
    let onOpenFile: () -> Void
    let onDismiss: () -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Add a document")
                .font(.title3.weight(.semibold))
                .padding(.bottom, 16)

            VStack(spacing: 10) {
                Choice(icon: "doc.badge.plus",
                       title: "Blank pages",
                       detail: "Paper to write on: how many, what size and colour",
                       action: onBlankPages)
                Choice(icon: "folder",
                       title: "Open a file",
                       detail: "A PDF already on this phone or in your storage",
                       action: onOpenFile)
            }

            // No confirm button: both answers are in the list, and a dialog whose
            // real choices sit above an OK invites people to press the OK.
            HStack {
                Spacer()
                Button("Cancel") { onDismiss() }
            }
            .padding(.top, 16)
        }
        .padding(20)
        .background(PagifyColor.background(scheme))
    }
}

private struct Choice: View {
    let icon: String
    let title: String
    let detail: String
    let action: () -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: action) {
            HStack(spacing: 14) {
                Image(systemName: icon)
                    .font(.system(size: 20))
                    .foregroundStyle(PagifyColor.primary(scheme))
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(PagifyColor.onSurface(scheme))
                    Text(detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.leading)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(PagifyColor.surfaceVariant(scheme), in: RoundedRectangle(cornerRadius: 14))
        }
        .buttonStyle(.plain)
    }
}
