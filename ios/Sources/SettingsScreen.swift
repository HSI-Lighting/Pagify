import SwiftUI

/// The handful of things that are actually settings.
///
/// Deliberately short. Everything about *this document* — the tools, the
/// rotation, the page organiser — belongs in the reader where it can be seen
/// taking effect; what is left is what outlives a document.
///
/// Values and callbacks rather than the store, so the screen can be drawn from a
/// preview and so that nothing here can write the settings file by accident.
struct SettingsScreen: View {
    let settings: AppSettings
    let onThemeChange: (ThemeChoice) -> Void
    let onShowViewfinder: (Bool) -> Void
    let showThumbnails: Bool
    let onShowThumbnails: (Bool) -> Void
    let isRecording: Bool
    let onToggleRecording: () -> Void
    let libraryCount: Int
    let onClearLibrary: () -> Void

    @Environment(\.colorScheme) private var scheme
    @State private var confirmingClear = false

    private var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text("Settings")
                    .font(.title2.weight(.bold))
                    .padding(.top, 24)
                    .padding(.bottom, 16)

                SectionLabel("Appearance")
                SettingCard {
                    VStack(alignment: .leading, spacing: 0) {
                        Text("Theme")
                            .font(.subheadline.weight(.semibold))
                        Spacer().frame(height: 2)
                        Text("Follow the phone, or hold the app light or dark whatever it does.")
                            .font(.caption)
                            .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                        Spacer().frame(height: 12)
                        Picker("Theme", selection: Binding(
                            get: { settings.theme },
                            set: { onThemeChange($0) }
                        )) {
                            ForEach(ThemeChoice.allCases) { choice in
                                Text(choice.label).tag(choice)
                            }
                        }
                        .pickerStyle(.segmented)
                    }
                    .padding(16)
                }

                SectionLabel("Reading")
                // Two cards rather than two rows in one: they are unrelated
                // settings that happen to be about reading, and a single card
                // reads as a group where changing one might affect the other.
                SettingCard {
                    ToggleRow(
                        title: "Show page thumbnails",
                        detail: "The strip beside the page. It hides itself on a narrow screen.",
                        isOn: showThumbnails,
                        onChange: onShowThumbnails)
                }
                SettingCard {
                    ToggleRow(
                        title: "Show the viewfinder",
                        detail: "The small map of the page that appears while zoomed in, "
                            + "for jumping about without panning. It can also be folded away "
                            + "to a handle from the map itself.",
                        isOn: settings.showViewfinder,
                        onChange: onShowViewfinder)
                }

                SectionLabel("Library")
                SettingCard {
                    ActionRow(
                        title: "Clear the library",
                        detail: libraryCount == 0
                            ? "Nothing to clear."
                            : "Forget \(libraryCount) document\(libraryCount == 1 ? "" : "s"). "
                                + "The files themselves are untouched.",
                        enabled: libraryCount > 0,
                        onTap: { confirmingClear = true })
                }

                SectionLabel("Diagnostics")
                SettingCard {
                    ToggleRow(
                        title: "Record a render timeline",
                        detail: "Writes what the reader drew and how long each render took, "
                            + "for chasing a slow or blank page.",
                        isOn: isRecording,
                        onChange: { _ in onToggleRecording() })
                }

                SectionLabel("About")
                SettingCard {
                    VStack(alignment: .leading, spacing: 0) {
                        Text("Pagify").font(.subheadline.weight(.semibold))
                        Spacer().frame(height: 4)
                        Text(version.isEmpty ? "PDF reader and markup" : "Version \(version)")
                            .font(.caption)
                            .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                    }
                    .padding(16)
                }

                Spacer().frame(height: 32)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
        }
        .background(PagifyColor.background(scheme))
        .alert("Clear the library?", isPresented: $confirmingClear) {
            Button("Clear", role: .destructive) { onClearLibrary() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This forgets which documents you have opened. "
                 + "The documents themselves are not touched.")
        }
    }
}

private struct SectionLabel: View {
    @Environment(\.colorScheme) private var scheme
    let text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text.uppercased())
            .font(.caption2)
            .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
            .padding(.leading, 4)
            .padding(.top, 20)
            .padding(.bottom, 8)
    }
}

private struct SettingCard<Content: View>: View {
    @Environment(\.colorScheme) private var scheme
    private let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        content
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(PagifyColor.surface(scheme), in: RoundedRectangle(cornerRadius: 16))
    }
}

/// A setting and the sentence explaining it, with the switch at the end.
///
/// The whole row answers a tap, not just the switch: the words are the part
/// people aim at, and a 51-point target at the far edge of a wide screen is a
/// setting most people miss on the first try.
private struct ToggleRow: View {
    @Environment(\.colorScheme) private var scheme
    let title: String
    let detail: String
    let isOn: Bool
    let onChange: (Bool) -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 0) {
                Text(title).font(.subheadline.weight(.semibold))
                Spacer().frame(height: 2)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Toggle("", isOn: Binding(get: { isOn }, set: { onChange($0) }))
                .labelsHidden()
        }
        .padding(16)
        .contentShape(Rectangle())
        .onTapGesture { onChange(!isOn) }
        .accessibilityElement(children: .combine)
    }
}

/// The same shape as a `ToggleRow` without the switch: a row that does something
/// once rather than holding a state.
private struct ActionRow: View {
    @Environment(\.colorScheme) private var scheme
    let title: String
    let detail: String
    let enabled: Bool
    let onTap: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(enabled
                    ? PagifyColor.onSurface(scheme)
                    : PagifyColor.onSurfaceVariant(scheme))
            Spacer().frame(height: 2)
            Text(detail)
                .font(.caption)
                .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .contentShape(Rectangle())
        .onTapGesture { if enabled { onTap() } }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }
}

// ------------------------------------------------------------------ model --

/// Light, dark, or whatever the phone is doing.
///
/// `system` is the default because a reader is one app among many and following
/// the phone is what most people expect — but only most: a document is a white
/// page whatever the app around it does, and someone reading in bed with the
/// phone in dark mode may still want the chrome light, or the other way about.
/// That is the whole reason this is a setting rather than a rule.
///
/// Stored by name rather than by ordinal, so reordering the cases cannot quietly
/// change what a stored file means.
enum ThemeChoice: String, CaseIterable, Identifiable {
    case system = "SYSTEM"
    case light = "LIGHT"
    case dark = "DARK"

    var id: String { rawValue }

    var label: String {
        switch self {
        case .system: return "System"
        case .light: return "Light"
        case .dark: return "Dark"
        }
    }

    /// What to hand `preferredColorScheme`. `nil` is how SwiftUI spells "follow
    /// the phone", which is why this is not a `Bool`.
    var colorScheme: ColorScheme? {
        switch self {
        case .system: return nil
        case .light: return .light
        case .dark: return .dark
        }
    }
}

/// Read a stored choice back, falling back to following the phone.
///
/// Lenient because the alternative is worse: a settings file written by a newer
/// build, or half-written, should leave the app looking like the phone rather
/// than refusing to start.
func themeChoiceFrom(_ stored: String?) -> ThemeChoice {
    ThemeChoice.allCases.first { $0.rawValue == stored } ?? .system
}

/// What a capture is encoded as.
enum CaptureFormat: String, CaseIterable, Identifiable {
    /// Lossless, and the right default: a page is line art and type, where PNG
    /// is both exact and small.
    case png = "PNG"
    /// For the other case. A scanned page is a photograph, and a lossless encode
    /// of a photograph is enormous.
    case jpeg = "JPEG"

    var id: String { rawValue }

    /// What the engine's `captureRegion` expects.
    var wireName: String { self == .png ? "png" : "jpeg" }

    /// What the pasteboard should call these bytes, so a paste lands as a picture
    /// rather than as a file nobody can preview.
    var pasteboardType: String { self == .png ? "public.png" : "public.jpeg" }
    var fileExtension: String { self == .png ? "png" : "jpg" }
    var mimeType: String { self == .png ? "image/png" : "image/jpeg" }
}

/// How sharp a capture to take, as a multiple of the page's natural size.
///
/// Named by what they are for rather than by their factor. "1×, 2×, 4×" is a
/// multiple of something the reader never sees — not the screen, not the file,
/// but the page's natural size — so it gave no way to tell which one you wanted.
enum CaptureScale: String, CaseIterable, Identifiable {
    case high = "HIGH"
    case medium = "MEDIUM"
    case low = "LOW"

    var id: String { rawValue }

    var factor: CGFloat {
        switch self {
        case .high: return 4
        case .medium: return 2
        case .low: return 1
        }
    }

    var label: String {
        switch self {
        case .high: return "Hi"
        case .medium: return "Mid"
        case .low: return "Lo"
        }
    }
}

/// What fills the part of a capture that is not page.
enum CaptureFill: String, CaseIterable, Identifiable {
    case page = "PAGE"
    case white = "WHITE"
    case black = "BLACK"
    /// No fill at all: those pixels are cut out of the picture. Only PNG can
    /// carry it, so choosing it moves the export to PNG.
    case transparent = "TRANSPARENT"

    var id: String { rawValue }

    var label: String {
        switch self {
        case .page: return "Page"
        case .white: return "White"
        case .black: return "Black"
        case .transparent: return "None"
        }
    }

    /// `nil` means "leave the page showing", which is not the same as any colour.
    var colour: UInt32? {
        switch self {
        case .page: return nil
        case .white: return 0xFF_FF_FF_FF
        case .black: return 0xFF_00_00_00
        case .transparent: return 0x00_00_00_00
        }
    }
}

/// The settings that outlive a document.
///
/// One value rather than a property per key, so reading and writing them is one
/// decision made once. The alternative — a getter and a setter per setting, each
/// remembering to write the file — is how a settings file ends up half-updated.
///
/// Note what is *not* here: whether the thumbnail rail is showing. That belongs
/// to the open document and lives on `ReaderModel`, because persisting it means
/// a rail someone hid on a phone comes back hidden on an iPad where it fits.
struct AppSettings: Equatable {
    var theme: ThemeChoice = .system
    /// Whether the viewfinder appears at all while zoomed. The hard off:
    /// no minimap and no handle to bring one back. `viewfinderMinimized` is the
    /// soft one, for when it is wanted but not right now.
    var showViewfinder: Bool = true
    /// Whether the viewfinder is collapsed to its handle. Remembered across
    /// documents, because someone who finds it distracting finds it distracting
    /// on the next drawing too — and the handle is right there when they want it
    /// back.
    var viewfinderMinimized: Bool = false
    /// Where the folded handle sits, as fractions of the reader area.
    ///
    /// Fractions rather than points, so it stays where it was put when the phone
    /// is turned or the rail appears — a handle remembered at 900 points down a
    /// portrait screen is off the bottom of a landscape one.
    var viewfinderHandleX: CGFloat = 1
    var viewfinderHandleY: CGFloat = 0.5
    /// How the last capture was exported. Kept here rather than with the open
    /// document because it is a habit, not a property of the file.
    var captureScale: CaptureScale = .high
    var captureFormat: CaptureFormat = .png
    var captureFill: CaptureFill = .page
}

/// The settings as the file holds them: one flat object, keys in this order.
func settingsJSON(_ settings: AppSettings) -> String {
    let fields = [
        "\"theme\":\"\(settings.theme.rawValue)\"",
        "\"showViewfinder\":\(settings.showViewfinder)",
        "\"viewfinderMinimized\":\(settings.viewfinderMinimized)",
        "\"viewfinderHandleX\":\(Double(settings.viewfinderHandleX))",
        "\"viewfinderHandleY\":\(Double(settings.viewfinderHandleY))",
        "\"captureScale\":\"\(settings.captureScale.rawValue)\"",
        "\"captureFormat\":\"\(settings.captureFormat.rawValue)\"",
        "\"captureFill\":\"\(settings.captureFill.rawValue)\"",
    ]
    return "{\(fields.joined(separator: ","))}"
}

/// Read them back, defaulting anything missing or unreadable.
///
/// Lenient per key rather than all-or-nothing: a file written by an older build
/// has fewer keys than this one expects, and losing the theme because the
/// viewfinder setting did not exist yet would be a poor trade.
func settingsFrom(json data: Data) -> AppSettings {
    guard let stored = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
        return AppSettings()
    }
    let defaults = AppSettings()

    func named<T: RawRepresentable>(_ key: String, _ fallback: T) -> T where T.RawValue == String {
        (stored[key] as? String).flatMap(T.init(rawValue:)) ?? fallback
    }
    // Clamped on the way in, not on the way out: a handle written outside the
    // area — by a build with a different idea of what the fractions mean, or by
    // a truncated write — would otherwise park itself off screen for good.
    func fraction(_ key: String, _ fallback: CGFloat) -> CGFloat {
        guard let value = (stored[key] as? NSNumber)?.doubleValue else { return fallback }
        return min(max(CGFloat(value), 0), 1)
    }

    return AppSettings(
        theme: themeChoiceFrom(stored["theme"] as? String),
        showViewfinder: (stored["showViewfinder"] as? NSNumber)?.boolValue
            ?? defaults.showViewfinder,
        viewfinderMinimized: (stored["viewfinderMinimized"] as? NSNumber)?.boolValue
            ?? defaults.viewfinderMinimized,
        viewfinderHandleX: fraction("viewfinderHandleX", defaults.viewfinderHandleX),
        viewfinderHandleY: fraction("viewfinderHandleY", defaults.viewfinderHandleY),
        captureScale: named("captureScale", defaults.captureScale),
        captureFormat: named("captureFormat", defaults.captureFormat),
        captureFill: named("captureFill", defaults.captureFill))
}

/// The settings that outlive a document, kept across launches.
///
/// One small JSON file, read once at startup and rewritten when something
/// changes — the same shape as `RecentDocumentsStore`, and for the same reasons:
/// a handful of values does not need a database, and a file that fails to read
/// should cost the defaults rather than the launch.
@MainActor
final class AppSettingsStore: ObservableObject {
    @Published private(set) var settings = AppSettings()

    private let file: URL

    init() {
        let support = FileManager.default.urls(for: .applicationSupportDirectory,
                                               in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: support, withIntermediateDirectories: true)
        file = support.appendingPathComponent("settings.json")
        if let data = try? Data(contentsOf: file) {
            settings = settingsFrom(json: data)
        }
    }

    /// Change one setting and write them all back.
    ///
    /// Takes the change as a transform rather than a value so a caller cannot
    /// accidentally write a stale copy of everything else alongside its own
    /// edit. A change that changes nothing writes nothing — the viewfinder
    /// handle reports its position on every drag end, and most of those land
    /// where it already was.
    func update(_ change: (AppSettings) -> AppSettings) {
        let updated = change(settings)
        guard updated != settings else { return }
        settings = updated
        try? Data(settingsJSON(updated).utf8).write(to: file, options: .atomic)
    }
}
