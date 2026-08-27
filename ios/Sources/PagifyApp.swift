import SwiftUI

@main
struct PagifyApp: App {
    @StateObject private var appSettings = AppSettingsStore()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(appSettings)
                .preferredColorScheme(appSettings.settings.theme.colorScheme)
        }
    }
}

/// The two places the app can be: in a document, or not.
///
/// The tab bar belongs to "not". A reader with a tab bar under it is a reader
/// with a strip of its page missing — and the page is the whole point — so
/// opening a document takes the screen and closing it gives the tabs back.
///
/// That is why the reader is a full-screen cover rather than a pushed
/// destination. A push arrives with a back chevron and an interactive swipe, and
/// the swipe cannot be asked to wait: it dismisses the reader past the "you have
/// unsaved marks" prompt, which is the one moment the app must be able to hold
/// on to.
struct RootView: View {
    @EnvironmentObject private var appSettings: AppSettingsStore
    @StateObject private var recents = RecentDocumentsStore()
    @StateObject private var model = ReaderModel()

    /// Survives being backgrounded: turning the phone or coming back to the app
    /// while reading the settings should not quietly put you in the library.
    @SceneStorage("tab") private var tab: HomeTab = .library

    @State private var inDocument = false
    @State private var chooserShowing = false
    @State private var isPicking = false
    @State private var showingBlankSheet = false

    /// Reader chrome, held here only until it has somewhere better to live.
    ///
    /// Both of these belong to the open document rather than to the app, so
    /// neither is persisted — a rail someone hid on a phone should not come back
    /// hidden on an iPad where it fits, and a diagnostic recording should not
    /// survive the launch that ended it. They sit on `RootView` because
    /// `ReaderModel` does not own them yet; the settings screen already asks the
    /// right questions of them.
    @State private var showThumbnails = true
    @State private var isRecording = false

    var body: some View {
        TabView(selection: $tab) {
            LibraryScreen(
                documents: recents.documents,
                onOpen: { open($0) },
                onForget: { recents.forget($0) },
                onPickDocument: { chooserShowing = true }
            )
            .tabItem { Label(HomeTab.library.label, systemImage: HomeTab.library.systemImage) }
            .tag(HomeTab.library)

            SettingsScreen(
                settings: appSettings.settings,
                onThemeChange: { choice in
                    appSettings.update { var updated = $0; updated.theme = choice; return updated }
                },
                onShowViewfinder: { showing in
                    appSettings.update {
                        var updated = $0
                        updated.showViewfinder = showing
                        return updated
                    }
                },
                showThumbnails: showThumbnails,
                onShowThumbnails: { showThumbnails = $0 },
                isRecording: isRecording,
                onToggleRecording: { isRecording.toggle() },
                libraryCount: recents.documents.count,
                onClearLibrary: { recents.clear() }
            )
            .tabItem { Label(HomeTab.settings.label, systemImage: HomeTab.settings.systemImage) }
            .tag(HomeTab.settings)
        }
        .fullScreenCover(isPresented: $inDocument) {
            ReaderView(model: model)
                .environmentObject(appSettings)
        }
        .task {
            model.start(recents: recents)
            openLaunchArgumentDocument()
        }
        // A file handed to us by Files, Mail, or another app.
        .onOpenURL { url in
            model.open(picked: url)
            enterReader()
        }
        .confirmationDialog("Add a document", isPresented: $chooserShowing,
                            titleVisibility: .visible) {
            // Both ways in — the button floating over the list and the one on the
            // empty screen — ask this same question first, so neither of them is
            // a shortcut past the other's answer.
            Button("Blank pages\u{2026}") { showingBlankSheet = true }
            Button("Open a file\u{2026}") { isPicking = true }
            Button("Cancel", role: .cancel) {}
        }
        .sheet(isPresented: $isPicking) {
            DocumentPicker { url in
                isPicking = false
                model.open(picked: url)
                enterReader()
            }
        }
        .sheet(isPresented: $showingBlankSheet) {
            BlankDocumentSheet { pages, size, ruling, fill in
                showingBlankSheet = false
                model.createBlank(pages: pages, size: size, ruling: ruling, fill: fill)
                enterReader()
            }
        }
        .alert("Something went wrong", isPresented: Binding(
            get: { model.failure != nil },
            set: { if !$0 { model.failure = nil } }
        )) {
            Button("OK") { model.failure = nil }
        } message: {
            Text(model.failure ?? "")
        }
    }

    /// `-openDocument <path>`, so the reader can be driven from a script.
    ///
    /// There is no way to tap a simulator from a terminal, and the reader is two
    /// taps past the library — which makes every screenshot of it a manual step.
    /// This is that step, automated.
    private func openLaunchArgumentDocument() {
        let arguments = ProcessInfo.processInfo.arguments
        guard let flag = arguments.firstIndex(of: "-openDocument"),
              arguments.index(after: flag) < arguments.endIndex else { return }
        let path = arguments[arguments.index(after: flag)]
        guard FileManager.default.fileExists(atPath: path) else { return }

        model.open(url: URL(fileURLWithPath: path), scoped: false)
        enterReader()
    }

    /// Open a row from the library.
    ///
    /// A row can outlive the file it points at, so a bookmark that no longer
    /// resolves is reported as what it is rather than as a mysterious open
    /// failure — and the row is left for the reader to remove.
    private func open(_ document: RecentDocument) {
        guard let resolved = recents.resolve(document) else {
            model.failure = "\(document.name) has moved or been deleted."
            return
        }
        model.open(url: resolved.url, scoped: resolved.scoped)
        enterReader()
    }

    /// Take the screen, if there is now something to show on it.
    ///
    /// One funnel rather than the same line after each of the four ways in, so
    /// that when the reader gains a state for "opening" or "asking for a
    /// password" there is a single place that decides they are still the reader
    /// rather than a bounce back to the library.
    private func enterReader() {
        inDocument = model.document != nil
    }
}

/// Where the tab bar can take you.
///
/// Two, and both of them earn their slot: the library is the app's front door
/// and settings is the only other thing that outlives a document. A bar padded
/// out with places that are really one screen is a bar that teaches people to
/// ignore it.
enum HomeTab: String {
    case library = "library"
    case settings = "settings"

    var label: String {
        switch self {
        case .library: return "Library"
        case .settings: return "Settings"
        }
    }

    var systemImage: String {
        switch self {
        case .library: return "books.vertical"
        case .settings: return "gearshape.fill"
        }
    }
}
