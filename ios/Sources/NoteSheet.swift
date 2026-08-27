import SwiftUI

/// What a note says.
///
/// A note is placed and then written, never committed empty — an empty note is a
/// dot on the page that cannot be read, cannot be edited, and reads as the note
/// not having been added.
struct NoteSheet: View {
    /// The words already there, when this is a note being read rather than made.
    let existing: String?
    let onCommit: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @FocusState private var focused: Bool

    private var isReading: Bool { existing != nil }

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 12) {
                TextEditor(text: $text)
                    .font(.system(size: 17))
                    .focused($focused)
                    .frame(minHeight: 140)
                    .padding(6)
                    .background(Color(.secondarySystemBackground),
                                in: RoundedRectangle(cornerRadius: 10))
                Spacer()
            }
            .padding(16)
            .navigationTitle(isReading ? "Note" : "New note")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(isReading ? "Done" : "Cancel") { dismiss() }
                }
                if !isReading {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Add") {
                            onCommit(text)
                            dismiss()
                        }
                        .fontWeight(.semibold)
                        .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
            .onAppear {
                if let existing { text = existing } else { focused = true }
            }
        }
    }
}
