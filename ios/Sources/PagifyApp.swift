import SwiftUI
import SwiftData
import UserNotifications

@main
struct PagifyApp: App {
    @StateObject private var appSettings = AppSettingsStore()

    init() {
        UNUserNotificationCenter.current().delegate = reminderCenterDelegate
        Reminders.registerCategories()
        Reminders.requestAuthorizationIfNeeded()
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(appSettings)
                .preferredColorScheme(appSettings.settings.theme.colorScheme)
        }
        // One container for the whole CRM. `ContactGroup` and `Meeting` are
        // reachable from `Contact` alone (`groups`, `meetings`), so listing
        // them separately here only matters if a screen ever needs to query
        // one of them with no `Contact` in hand.
        .modelContainer(for: [Contact.self, ContactGroup.self, Meeting.self])
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
    @Environment(\.modelContext) private var modelContext

    /// Survives being backgrounded: turning the phone or coming back to the app
    /// while reading the settings should not quietly put you in the library.
    @SceneStorage("tab") private var tab: HomeTab = .library

    @State private var inDocument = false
    @State private var chooserShowing = false
    @State private var isPicking = false
    @State private var showingBlankSheet = false
    /// Drives the STEP/drawing full-screen covers below — a separate pair
    /// from `inDocument`/`model`, since neither viewer is `ReaderModel`'s
    /// concern: `ReaderModel` stays exactly the PDF-only 1500-line file it
    /// already was, and a model or drawing never enters it at all.
    @State private var openModel: OpenedFile?
    @State private var openDrawing: OpenedFile?
    @State private var sharingDocument: SharingDocument?

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
                onShare: { shareDocument($0) },
                onPickDocument: { chooserShowing = true }
            )
            .tabItem { Label(HomeTab.library.label, systemImage: HomeTab.library.systemImage) }
            .tag(HomeTab.library)

            ContactsScreen()
                .tabItem { Label(HomeTab.contacts.label, systemImage: HomeTab.contacts.systemImage) }
                .tag(HomeTab.contacts)

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
        .fullScreenCover(item: $openModel) { file in
            ModelViewerScreen(file: file, recents: recents) { openModel = nil }
        }
        .fullScreenCover(item: $openDrawing) { file in
            DrawingViewerScreen(file: file, recents: recents) { openDrawing = nil }
        }
        .task {
            model.start(recents: recents)
            openLaunchArgumentDocument()
            // The delegate is created before SwiftData's environment exists,
            // so this is the first moment it can actually reach a context.
            reminderCenterDelegate.context = modelContext
            Reminders.resyncAll(context: modelContext)
        }
        // A file handed to us by Files, Mail, or another app.
        .onOpenURL { url in
            openAny(url: url)
        }
        // Both ways in — the button floating over the list and the one on the
        // empty screen — ask this same question first, so neither of them is
        // a shortcut past the other's answer. A custom centered overlay
        // rather than `.confirmationDialog`, matching Android's own rich
        // AlertDialog: `.confirmationDialog` can't carry an icon or a
        // subtitle line per button.
        .overlay {
            if chooserShowing {
                NewDocumentChooser(
                    onBlankPages: { chooserShowing = false; showingBlankSheet = true },
                    onOpenFile: { chooserShowing = false; isPicking = true },
                    onCancel: { chooserShowing = false }
                )
                .transition(.opacity)
            }
        }
        .animation(.default, value: chooserShowing)
        .sheet(isPresented: $isPicking) {
            DocumentPicker { url in
                isPicking = false
                openAny(url: url)
            }
        }
        .sheet(item: $sharingDocument) { sharing in
            ShareSheet(items: [sharing.url])
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

        openAny(url: URL(fileURLWithPath: path))
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
        switch document.kind {
        case .document:
            model.open(url: resolved.url, scoped: resolved.scoped)
            enterReader()
        case .model:
            openModel = OpenedFile(url: resolved.url, alreadyScoped: resolved.scoped)
        case .drawing:
            openDrawing = OpenedFile(url: resolved.url, alreadyScoped: resolved.scoped)
        }
    }

    /// Hand a library row's file to the system share sheet, matching
    /// Android's own persistent per-row Share button.
    ///
    /// The security-scoped access `resolve()` starts is deliberately never
    /// stopped here — the share sheet needs the file to stay reachable for
    /// as long as it's up, and unlike opening a document there is no later
    /// "close" moment to hang the matching `stop` off. The same trade a
    /// library row already makes for reading; the OS reclaims every scope
    /// this process holds when it backgrounds or terminates.
    private func shareDocument(_ document: RecentDocument) {
        guard let resolved = recents.resolve(document) else {
            model.failure = "\(document.name) has moved or been deleted."
            return
        }
        sharingDocument = SharingDocument(url: resolved.url)
    }

    /// The other three ways in — the picker, `.onOpenURL`, and (PDF only)
    /// the `-openDocument` launch argument — hand over a fresh URL with no
    /// access taken yet, unlike a library row's already-resolved one, so
    /// each destination takes it in the same call that opens the file.
    private func openAny(url: URL) {
        switch RecentKind.of(filename: url.lastPathComponent) {
        case .document:
            model.open(picked: url)
            enterReader()
        case .model:
            openModel = OpenedFile(url: url, alreadyScoped: false)
        case .drawing:
            openDrawing = OpenedFile(url: url, alreadyScoped: false)
        }
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

/// A URL handed to `ModelViewerScreen`/`DrawingViewerScreen`, wrapped so
/// `.fullScreenCover(item:)` has an `Identifiable` to key off.
///
/// `alreadyScoped` distinguishes a library row's URL (`recents.resolve`
/// already called `startAccessingSecurityScopedResource()`) from a fresh
/// one from the picker or `.onOpenURL` (nothing called yet) — the viewer's
/// own `open()` only calls it itself when this is false, so the two paths
/// never double-start the same access.
struct OpenedFile: Identifiable {
    let url: URL
    let alreadyScoped: Bool
    var id: String { url.absoluteString }
}

/// A library row's file, on its way to the system share sheet.
struct SharingDocument: Identifiable {
    let url: URL
    var id: String { url.absoluteString }
}

/// Where the tab bar can take you.
///
/// Two, and both of them earn their slot: the library is the app's front door
/// and settings is the only other thing that outlives a document. A bar padded
/// out with places that are really one screen is a bar that teaches people to
/// ignore it.
enum HomeTab: String {
    case library = "library"
    case contacts = "contacts"
    case settings = "settings"

    var label: String {
        switch self {
        case .library: return "Library"
        case .contacts: return "Contacts"
        case .settings: return "Settings"
        }
    }

    var systemImage: String {
        switch self {
        case .library: return "books.vertical"
        case .contacts: return "person.crop.rectangle.stack"
        case .settings: return "gearshape.fill"
        }
    }
}
