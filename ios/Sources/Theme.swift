import SwiftUI

/// Pagify's colours, taken from Android's `ui/theme/Theme.kt` so the two builds
/// are the same app rather than two apps with the same name.
enum PagifyColor {
    // Light. The page is white and the space around it is kept slightly grey — a
    // pure-white background would make page edges vanish.
    private static let lightPrimary = Color(hex: 0x3F5F90)
    private static let lightBackground = Color(hex: 0xF4F4F7)
    private static let lightSurface = Color(hex: 0xFDFBFF)
    private static let lightOnSurface = Color(hex: 0x1A1B21)
    private static let lightSurfaceVariant = Color(hex: 0xE0E2EC)

    private static let lightOnPrimary = Color(hex: 0xFFFFFF)
    // Material's own baseline pair, which Android inherits by not overriding it.
    private static let lightOnSurfaceVariant = Color(hex: 0x49454F)
    private static let lightOutlineVariant = Color(hex: 0xCAC4D0)

    private static let darkPrimary = Color(hex: 0xA8C7FA)
    private static let darkBackground = Color(hex: 0x121316)
    private static let darkSurface = Color(hex: 0x1A1B1F)
    private static let darkOnSurface = Color(hex: 0xE3E2E9)
    private static let darkSurfaceVariant = Color(hex: 0x2B2C31)
    private static let darkOnPrimary = Color(hex: 0x0A305F)
    private static let darkOnSurfaceVariant = Color(hex: 0xCAC4D0)
    private static let darkOutlineVariant = Color(hex: 0x49454F)

    static func primary(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? darkPrimary : lightPrimary
    }
    static func background(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? darkBackground : lightBackground
    }
    /// What the floating ribbons and sheets are made of.
    static func surface(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? darkSurface : lightSurface
    }
    static func onSurface(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? darkOnSurface : lightOnSurface
    }
    /// The ground and ink for a destructive chip. Theme values rather than the
    /// system red, which is dynamic and tracks neither palette.
    static func errorContainer(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color(hex: 0x93000A) : Color(hex: 0xFFDAD6)
    }
    static func onErrorContainer(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color(hex: 0xFFDAD6) : Color(hex: 0x410002)
    }

    static func surfaceVariant(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? darkSurfaceVariant : lightSurfaceVariant
    }
    /// What sits on a filled primary ground — a selected tool button, and nothing
    /// else. Pale in light, near-navy in dark, because the primary flips too.
    static func onPrimary(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? darkOnPrimary : lightOnPrimary
    }
    /// The ribbon's plain ink: every glyph that is *not* the live choice.
    static func onSurfaceVariant(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? darkOnSurfaceVariant : lightOnSurfaceVariant
    }
    /// The hairline round an unselected swatch, which has to be visible without
    /// competing with the 3pt ring that means "chosen".
    static func outlineVariant(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? darkOutlineVariant : lightOutlineVariant
    }

    /// What marks the live choice, everywhere in the mark ribbon.
    ///
    /// A warm amber rather than the theme's own accent, and **not** themed. Every
    /// slot in that row is a glyph made of several parts with one of them current,
    /// and the part picked out has to read against a colour swatch, a stack of grey
    /// lines and a group of grey icons alike. The theme accent is a blue close
    /// enough to the surface tint that "which one is on" had to be worked out
    /// rather than seen.
    static let ribbonAccent = Color(hex: 0xF2A93B)

    /// What sits *on* the amber. Dark, because the amber is light in either theme —
    /// `onPrimary` is a pale colour meant for the theme's own accent, and on this
    /// one it disappears.
    static let accentInk = Color(hex: 0x241B08)
}

extension Color {
    init(hex: UInt32) {
        self.init(.sRGB,
                  red: Double((hex >> 16) & 0xFF) / 255,
                  green: Double((hex >> 8) & 0xFF) / 255,
                  blue: Double(hex & 0xFF) / 255,
                  opacity: 1)
    }
}
