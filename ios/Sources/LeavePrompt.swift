import SwiftUI

/// Asked on the way out of a document with work in it.
///
/// Android's `ui/components/LeavePrompt.kt`, and the same three answers in the
/// same places. The one thing worth restating is the button order:
///
///     [ Exit ]  [ Save as ]              [ Save ]
///
/// Exit sits furthest from Save so the two opposite meanings are not neighbours
/// under a thumb. And the way out of the question is a cross rather than a
/// Cancel button, because the buttons are things to do about the marks, and
/// closing the dialog is not one of them.
struct LeavePrompt: View {
    let onSave: () -> Void
    let onSaveAs: () -> Void
    let onExit: () -> Void
    let onClose: () -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        ZStack {
            // The scrim answers a tap the same way the cross does: the reader is
            // put back exactly where it was, on the same page, with every mark
            // still on it.
            Color.black.opacity(0.32)
                .ignoresSafeArea()
                .onTapGesture(perform: onClose)

            VStack(alignment: .leading, spacing: 16) {
                HStack(alignment: .firstTextBaseline) {
                    Text("Save your changes?")
                        .font(.title3.weight(.semibold))

                    Spacer(minLength: 12)

                    Button(action: onClose) {
                        Image(systemName: "xmark")
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .frame(width: 32, height: 32)
                            .contentShape(Rectangle())
                    }
                    .accessibilityLabel("Close")
                }

                Text("This document has marks that have not been saved.")
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                // The padding is the gap. Material's text buttons carry their own
                // internal padding, so a 4pt row still reads as separate actions;
                // SwiftUI's borderless buttons carry none, and "Exit Save as" ran
                // together into one phrase.
                HStack(spacing: 4) {
                    Button("Exit", action: onExit)
                        .padding(.horizontal, 10)
                    Button("Save as", action: onSaveAs)
                        .padding(.horizontal, 10)
                    Spacer(minLength: 16)
                    Button("Save", action: onSave)
                        .fontWeight(.semibold)
                        .padding(.horizontal, 10)
                }
                .buttonStyle(.borderless)
                .padding(.horizontal, -10)
            }
            .padding(20)
            .frame(maxWidth: 340)
            .background(PagifyColor.surface(scheme),
                        in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .shadow(color: .black.opacity(scheme == .dark ? 0.6 : 0.22), radius: 24, y: 8)
            .padding(24)
        }
        // The whole question is one thing to a screen reader, and the cross is
        // reachable without hunting for it.
        .accessibilityAddTraits(.isModal)
    }
}
