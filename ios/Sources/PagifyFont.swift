import Foundation

/// The faces a caption can be written in. Android's `core/PdfFonts.kt`.
///
/// Five standard-14 names PDFium already has, and eighteen files that get
/// embedded. The standard-14 have no Arabic, no Devanagari, no CJK — nothing but
/// Latin-1 — and they are not embedded, so there is nothing to add: a script they
/// cannot draw needs a real font file inside the document.
enum PagifyFont: String, CaseIterable, Identifiable {
    case helvetica, helveticaBold, times, timesBold, courier
    case notoSans, notoSansBold, notoSerif, notoSerifBold
    case notoNaskhArabic, notoNaskhArabicBold, notoKufiArabic, notoSansArabic
    case iranNastaliq, shekasteh
    case notoSansDevanagari, notoSansBengali, notoSansTamil, notoSansThai, notoSansHebrew
    case notoSansSC, notoSansTC, notoSansJP, notoSansKR

    var id: String { rawValue }

    /// What the ribbon calls it — in its own script, where it has one.
    var label: String {
        switch self {
        case .helvetica: return "Helvetica"
        case .helveticaBold: return "Helvetica Bold"
        case .times: return "Times"
        case .timesBold: return "Times Bold"
        case .courier: return "Courier"
        case .notoSans: return "Noto Sans"
        case .notoSansBold: return "Noto Sans Bold"
        case .notoSerif: return "Noto Serif"
        case .notoSerifBold: return "Noto Serif Bold"
        case .notoNaskhArabic: return "نسخ"
        case .notoNaskhArabicBold: return "نسخ عريض"
        case .notoKufiArabic: return "كوفي"
        case .notoSansArabic: return "عربي"
        case .iranNastaliq: return "نستعلیق"
        case .shekasteh: return "شکسته"
        case .notoSansDevanagari: return "देवनागरी"
        case .notoSansBengali: return "বাংলা"
        case .notoSansTamil: return "தமிழ்"
        case .notoSansThai: return "ไทย"
        case .notoSansHebrew: return "עברית"
        case .notoSansSC: return "简体中文"
        case .notoSansTC: return "繁體中文"
        case .notoSansJP: return "日本語"
        case .notoSansKR: return "한국어"
        }
    }

    /// The standard-14 name, as `FPDFText_LoadStandardFont` expects. An embedded
    /// face still carries one: it is what the mark falls back to.
    var wireName: String {
        switch self {
        case .helvetica: return "Helvetica"
        case .helveticaBold: return "Helvetica-Bold"
        case .times: return "Times-Roman"
        case .timesBold: return "Times-Bold"
        case .courier: return "Courier"
        case .notoSans: return "Helvetica"
        case .notoSansBold: return "Helvetica-Bold"
        case .notoSerif: return "Times-Roman"
        case .notoSerifBold: return "Times-Bold"
        case .notoNaskhArabic: return "Helvetica"
        case .notoNaskhArabicBold: return "Helvetica-Bold"
        case .notoKufiArabic: return "Helvetica"
        case .notoSansArabic: return "Helvetica"
        case .iranNastaliq: return "Helvetica"
        case .shekasteh: return "Helvetica"
        case .notoSansDevanagari: return "Helvetica"
        case .notoSansBengali: return "Helvetica"
        case .notoSansTamil: return "Helvetica"
        case .notoSansThai: return "Helvetica"
        case .notoSansHebrew: return "Helvetica"
        case .notoSansSC: return "Helvetica"
        case .notoSansTC: return "Helvetica"
        case .notoSansJP: return "Helvetica"
        case .notoSansKR: return "Helvetica"
        }
    }

    /// The file to embed, or nil for a standard-14.
    var asset: String? {
        switch self {
        case .helvetica: return nil
        case .helveticaBold: return nil
        case .times: return nil
        case .timesBold: return nil
        case .courier: return nil
        case .notoSans: return "NotoSans-Regular.ttf"
        case .notoSansBold: return "NotoSans-Bold.ttf"
        case .notoSerif: return "NotoSerif-Regular.ttf"
        case .notoSerifBold: return "NotoSerif-Bold.ttf"
        case .notoNaskhArabic: return "NotoNaskhArabic-Regular.ttf"
        case .notoNaskhArabicBold: return "NotoNaskhArabic-Bold.ttf"
        case .notoKufiArabic: return "NotoKufiArabic-Regular.ttf"
        case .notoSansArabic: return "NotoSansArabic-Regular.ttf"
        case .iranNastaliq: return "IranNastaliq.ttf"
        case .shekasteh: return "Shekasteh.ttf"
        case .notoSansDevanagari: return "NotoSansDevanagari-Regular.ttf"
        case .notoSansBengali: return "NotoSansBengali-Regular.ttf"
        case .notoSansTamil: return "NotoSansTamil-Regular.ttf"
        case .notoSansThai: return "NotoSansThai-Regular.ttf"
        case .notoSansHebrew: return "NotoSansHebrew-Regular.ttf"
        case .notoSansSC: return "NotoSansSC.ttf"
        case .notoSansTC: return "NotoSansTC.ttf"
        case .notoSansJP: return "NotoSansJP.ttf"
        case .notoSansKR: return "NotoSansKR.ttf"
        }
    }

    /// What it is for, shown under the name so a face is choosable by somebody
    /// who cannot read its own label.
    var script: String {
        switch self {
        case .helvetica: return "Latin"
        case .helveticaBold: return "Latin"
        case .times: return "Latin"
        case .timesBold: return "Latin"
        case .courier: return "Latin"
        case .notoSans: return "Latin · Greek · Cyrillic"
        case .notoSansBold: return "Latin · Greek · Cyrillic"
        case .notoSerif: return "Latin · Greek · Cyrillic"
        case .notoSerifBold: return "Latin · Greek · Cyrillic"
        case .notoNaskhArabic: return "Arabic · Persian · Urdu"
        case .notoNaskhArabicBold: return "Arabic · Persian · Urdu"
        case .notoKufiArabic: return "Arabic · Persian · Urdu"
        case .notoSansArabic: return "Arabic · Persian · Urdu"
        case .iranNastaliq: return "Persian"
        case .shekasteh: return "Persian"
        case .notoSansDevanagari: return "Hindi · Marathi · Nepali"
        case .notoSansBengali: return "Bengali · Assamese"
        case .notoSansTamil: return "Tamil"
        case .notoSansThai: return "Thai"
        case .notoSansHebrew: return "Hebrew"
        case .notoSansSC: return "Simplified Chinese"
        case .notoSansTC: return "Traditional Chinese"
        case .notoSansJP: return "Japanese"
        case .notoSansKR: return "Korean"
        }
    }

    var isBold: Bool {
        switch self {
        case .helvetica: return false
        case .helveticaBold: return true
        case .times: return false
        case .timesBold: return true
        case .courier: return false
        case .notoSans: return false
        case .notoSansBold: return true
        case .notoSerif: return false
        case .notoSerifBold: return true
        case .notoNaskhArabic: return false
        case .notoNaskhArabicBold: return true
        case .notoKufiArabic: return false
        case .notoSansArabic: return false
        case .iranNastaliq: return false
        case .shekasteh: return false
        case .notoSansDevanagari: return false
        case .notoSansBengali: return false
        case .notoSansTamil: return false
        case .notoSansThai: return false
        case .notoSansHebrew: return false
        case .notoSansSC: return false
        case .notoSansTC: return false
        case .notoSansJP: return false
        case .notoSansKR: return false
        }
    }

    /// Whether writing in this font puts a font file inside the document.
    var isEmbedded: Bool { asset != nil }
}

// ------------------------------------------------------------------ metrics --

/// The first character the width tables cover.
private let firstPrintable = 32

extension PagifyFont {
    /// Widths in thousandths of an em, from the standard-14 AFMs. Only meaningful
    /// for a standard-14 — an embedded face is measured by shaping it.
    private var widthTable: [Int] {
        switch self {
        case .times, .notoSerif: return timesWidths
        case .timesBold, .notoSerifBold: return timesBoldWidths
        case .courier: return courierWidths
        case .helveticaBold, .notoSansBold, .notoNaskhArabicBold: return helveticaBoldWidths
        default: return helveticaWidths
        }
    }

    /// How far the pen moves after drawing `character`, in points.
    func advance(of character: Character, size: CGFloat) -> CGFloat {
        guard let scalar = character.unicodeScalars.first else { return 0 }
        let widths = widthTable
        let index = Int(scalar.value) - firstPrintable
        let thousandths = widths.indices.contains(index) ? widths[index] : widths[0]
        return CGFloat(thousandths) / 1000 * size
    }

    /// How wide `text` is, in points.
    ///
    /// An embedded face is measured by **shaping** it, not by adding up character
    /// widths. In most of the world's scripts those are not the same number:
    /// Arabic letters join into forms that have no character of their own, and
    /// Devanagari reorders.
    func width(of text: String, size: CGFloat) -> CGFloat {
        // `!glyphs.isEmpty` matters: a shaper that succeeds with nothing to say
        // would otherwise report a width of zero, which collapses the baseline and
        // makes the caption invisible on screen *and* in the file.
        if isEmbedded, let shaped = PagifyFonts.shape(self, text), !shaped.glyphs.isEmpty {
            return shaped.glyphs.reduce(0) { $0 + $1.advance } * size
        }
        return text.reduce(0) { $0 + advance(of: $1, size: size) }
    }
}

private let helveticaWidths: [Int] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
        556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
        1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
        667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
        333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
        556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584
]

private let helveticaBoldWidths: [Int] = [
        278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278,
        556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611,
        975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
        667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556,
        333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611,
        611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584
]

private let timesWidths: [Int] = [
        250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278,
        500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444,
        921, 722, 667, 667, 722, 611, 556, 722, 722, 333, 389, 722, 611, 889, 722, 722,
        556, 722, 667, 556, 611, 722, 722, 944, 722, 722, 611, 333, 278, 333, 469, 500,
        333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500, 278, 778, 500, 500,
        500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541
]

private let timesBoldWidths: [Int] = [
        250, 333, 555, 500, 500, 1000, 833, 278, 333, 333, 500, 570, 250, 333, 250, 278,
        500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500,
        930, 722, 667, 722, 722, 667, 611, 778, 778, 389, 500, 778, 667, 944, 722, 778,
        611, 778, 722, 556, 667, 722, 722, 1000, 722, 722, 667, 333, 278, 333, 581, 500,
        333, 500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556, 278, 833, 556, 500,
        556, 556, 444, 389, 333, 556, 500, 722, 500, 500, 444, 394, 220, 394, 520
]

/// Courier is monospaced: every character is 600 thousandths wide.
private let courierWidths = [Int](repeating: 600, count: 95)

// ------------------------------------------------------------------ drawing --

#if canImport(UIKit)
import UIKit

extension PagifyFont {
    /// The face to draw this mark's preview in.
    ///
    /// The bundled file where there is one, so what is on screen is the face the
    /// words will be written in. A caption previewed in the system font and saved
    /// in Naskh is two different captions, and the reader only sees the first.
    func uiFont(size: CGFloat) -> UIFont {
        if let asset, let registered = PagifyFonts.registeredUIFont(asset: asset, size: size) {
            return registered
        }
        // The standard-14 have close enough system equivalents, and PDFium draws
        // the real thing when it writes the page.
        switch self {
        case .times, .timesBold, .notoSerif, .notoSerifBold:
            let base = UIFont.systemFont(ofSize: size, weight: isBold ? .bold : .regular)
            let design = base.fontDescriptor.withDesign(.serif) ?? base.fontDescriptor
            return UIFont(descriptor: design, size: size)
        case .courier:
            return UIFont.monospacedSystemFont(ofSize: size, weight: .regular)
        default:
            return UIFont.systemFont(ofSize: size, weight: isBold ? .bold : .regular)
        }
    }
}
#endif

// ------------------------------------------------------------------ shaping --

/// One glyph as the shaper hands it back.
struct ShapedGlyph {
    /// The characters this glyph stands for — one glyph, but not always one
    /// `Character`. Carried even for an embedded face, because it is what the
    /// glyph *means*: the ToUnicode is built from it, and without that the words
    /// draw perfectly and cannot be copied.
    let text: String
    /// The glyph's id in the embedded font.
    let id: UInt32
    /// Fractions of the point size, so the app scales them by whatever size the
    /// reader chose without asking again.
    let advance: CGFloat
    let offsetX: CGFloat
    let offsetY: CGFloat
}

struct ShapedRun {
    let rightToLeft: Bool
    let glyphs: [ShapedGlyph]
}

/// Registering the bundled faces with the engine, and shaping through it.
///
/// Registered lazily rather than all at once: the four CJK faces are nine to
/// sixteen megabytes each, and loading twenty-two files to write one caption in
/// one of them is most of a second nobody asked for.
enum PagifyFonts {
    private static var registered = Set<String>()

    /// Where to find the font files, when it is not the app bundle. The host
    /// probe has no bundle, and a text path only tested inside the app is a text
    /// path only tested where it is hardest to look at.
    static var directoryOverride: URL?

    /// Hand the engine a font file, under the name the wire format uses for it.
    @discardableResult
    static func register(_ font: PagifyFont) -> Bool {
        guard let asset = font.asset else { return true }
        if registered.contains(asset) { return true }

        let stem = (asset as NSString).deletingPathExtension
        let located = directoryOverride?.appendingPathComponent(asset)
            ?? Bundle.main.url(forResource: stem, withExtension: "ttf", subdirectory: "fonts")
            ?? Bundle.main.url(forResource: stem, withExtension: "ttf")
        guard let url = located, let data = try? Data(contentsOf: url) else { return false }

        let ok = data.withUnsafeBytes { buffer -> Bool in
            guard let base = buffer.bindMemory(to: UInt8.self).baseAddress else { return false }
            return pagify_register_font(asset, base, buffer.count) == PAGIFY_OK
        }
        if ok { registered.insert(asset) }
        return ok
    }

    /// Shape `text` in `font`, or nil for a face with nothing to shape with.
    static func shape(_ font: PagifyFont, _ text: String) -> ShapedRun? {
        guard let asset = font.asset, !text.isEmpty, register(font) else { return nil }
        guard let json = PagifyEngine.string(pagify_shape_text_json(asset, text)),
              let data = json.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            return nil
        }

        let utf8 = Array(text.utf8)
        let glyphs = (o["glyphs"] as? [[String: Any]] ?? []).map { g -> ShapedGlyph in
            // `from`/`to` are byte offsets into the text: which characters this
            // glyph stands for. They come back with the glyph so the ToUnicode
            // can be built from them.
            // Clamped at both ends. These are byte offsets from another
            // language's idea of the string, and an out-of-range slice is a trap,
            // not an error anyone gets to see.
            let raw = g["from"] as? Int ?? 0
            let lo = min(max(raw, 0), utf8.count)
            let hi = min(max(g["to"] as? Int ?? lo, lo), utf8.count)
            let slice = lo < hi ? String(decoding: utf8[lo..<hi], as: UTF8.self) : ""
            return ShapedGlyph(text: slice,
                               id: UInt32(clamping: g["id"] as? Int ?? 0),
                               advance: g["advance"] as? CGFloat ?? 0,
                               offsetX: g["dx"] as? CGFloat ?? 0,
                               offsetY: g["dy"] as? CGFloat ?? 0)
        }
        return ShapedRun(rightToLeft: o["rtl"] as? Bool ?? false, glyphs: glyphs)
    }

    #if canImport(UIKit)
    /// Register a bundled face with Core Text as well as with the engine, so the
    /// preview can draw in it.
    ///
    /// Cached: `CTFontManager` refuses a second registration of the same file,
    /// and building a `UIFont` per glyph per frame is not free.
    private static var uiFonts: [String: String] = [:]

    static func registeredUIFont(asset: String, size: CGFloat) -> UIFont? {
        if let name = uiFonts[asset] { return UIFont(name: name, size: size) }

        let stem = (asset as NSString).deletingPathExtension
        guard let url = directoryOverride?.appendingPathComponent(asset)
                ?? Bundle.main.url(forResource: stem, withExtension: "ttf", subdirectory: "fonts")
                ?? Bundle.main.url(forResource: stem, withExtension: "ttf"),
              let data = try? Data(contentsOf: url),
              let provider = CGDataProvider(data: data as CFData),
              let cgFont = CGFont(provider) else {
            return nil
        }

        // A failure here is usually "already registered", which is fine — the
        // name lookup below is what actually matters.
        CTFontManagerRegisterGraphicsFont(cgFont, nil)
        guard let name = cgFont.postScriptName as String? else { return nil }
        uiFonts[asset] = name
        return UIFont(name: name, size: size)
    }
    #endif

    /// Whether a face can draw every character of some text — what lets the app
    /// pick a font the reader did not, so typing Persian into a caption set in
    /// Helvetica produces Persian rather than a row of empty boxes.
    static func covers(_ font: PagifyFont, _ text: String) -> Bool {
        guard let asset = font.asset, register(font) else { return false }
        return pagify_font_covers(asset, text)
    }
}
