import SwiftUI

/// Nothing open yet.
struct EmptyState: View {
    let onPickDocument: () -> Void

    var body: some View {
        VStack(spacing: 6) {
            Text("No document open").font(.headline)
            Text("Choose a PDF to start reading.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
            Button("Open a PDF", action: onPickDocument).padding(.top, 10)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(32)
    }
}

/// The document is encrypted.
///
/// The password lives in this view's own state and nowhere else — not on the
/// model, not in the recents file, not in a bookmark. It is used once, to open
/// the document, and then it is gone.
struct PasswordPrompt: View {
    let retry: Bool
    let onUnlock: (String) -> Void

    @State private var password = ""

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: "lock.doc")
                .font(.system(size: 34))
                .foregroundStyle(.secondary)
            Text("This document is protected").font(.headline)
            if retry {
                Text("That password was not accepted.")
                    .font(.subheadline)
                    .foregroundStyle(.red)
            }
            SecureField("Password", text: $password)
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: 280)
                .submitLabel(.go)
                .onSubmit { if !password.isEmpty { onUnlock(password) } }
            Button("Unlock") { onUnlock(password) }
                .buttonStyle(.borderedProminent)
                .disabled(password.isEmpty)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(32)
    }
}

/// Something went wrong that a password will not fix.
struct ReaderMessage: View {
    let title: String
    let detail: String
    let actionLabel: String
    let action: () -> Void

    var body: some View {
        VStack(spacing: 6) {
            Text(title).font(.headline)
            Text(detail)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Button(actionLabel, action: action).padding(.top, 10)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(32)
    }
}
