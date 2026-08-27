import SwiftUI

/// What a caption says, before it becomes glyphs.
///
/// A sheet rather than an inline field: the words can run to several lines, the
/// face and size are being chosen at the same time on the ribbon, and a caret
/// floating over the page with a keyboard under it leaves nowhere to show either.
struct TextEditorSheet: View {
    let font: PagifyFont
    let size: CGFloat
    let color: MarkColor
    /// The words already there, when this is an edit rather than a new caption.
    var initial: String = ""
    let onCommit: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 12) {
                TextEditor(text: $text)
                    .font(.system(size: 18))
                    .focused($focused)
                    .frame(minHeight: 120)
                    .padding(6)
                    .background(Color(.secondarySystemBackground),
                                in: RoundedRectangle(cornerRadius: 10))

                // Whether the chosen face can actually draw what was typed. The
                // standard-14 have nothing but Latin-1, so Persian in Helvetica
                // is a row of empty boxes — said here rather than discovered on
                // the page.
                if !text.isEmpty, font.isEmbedded, !PagifyFonts.covers(font, text) {
                    Label("\(font.label) cannot draw every character here.",
                          systemImage: "exclamationmark.triangle")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }

                Text("\(font.label) · \(font.script) · \(String(format: "%.0f", size))pt")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Spacer()
            }
            .padding(16)
            .navigationTitle("Caption")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(initial.isEmpty ? "Add" : "Save") {
                        onCommit(text)
                        dismiss()
                    }
                    .fontWeight(.semibold)
                    // Emptying the words is how a caption is deleted, so an
                    // empty box is allowed when editing one.
                    .disabled(initial.isEmpty
                              && text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            .onAppear {
                if text.isEmpty { text = initial }
                focused = true
            }
        }
    }
}
