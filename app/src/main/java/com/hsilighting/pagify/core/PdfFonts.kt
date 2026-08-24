package com.hsilighting.pagify.core

/**
 * The fonts text can be written in.
 *
 * Two kinds, and the difference decides how a caption reaches the file.
 *
 * The **standard-14** come first: Helvetica, Times, Courier. They need no
 * embedding, cost nothing in file size, and are present in every reader ever
 * made — which matters, because a marked-up drawing is almost always going to
 * somebody else. They also hold Latin-1 and nothing else.
 *
 * The rest are **bundled files**, embedded into any document they are used in.
 * That is the only way to write a script the standard set cannot draw, and the
 * only way to write a form that has no character of its own: a joined Arabic
 * letter, a Devanagari conjunct. Those need the text shaped before anything is
 * written down, which is what [BundledFonts] is for.
 *
 * [family] is the nearest thing Android has, used to draw the preview for a
 * standard-14 font. It is a likeness, not the font: the phone does not have
 * Helvetica, it has something that looks like it. What the preview and the file
 * *do* agree on is where every glyph sits, because both lay out from [advanceOf]
 * rather than from whatever the phone happens to measure. A bundled font has no
 * such gap — the preview draws the same file the document embeds.
 */
enum class PdfFont(
    val label: String,
    /** The name PDFium's `FPDFText_LoadStandardFont` expects. */
    val wireName: String,
    val family: String,
    val bold: Boolean,
    /**
     * The bundled file this font is, or null for one of the standard-14.
     *
     * Also the switch between the two write paths: a font with an asset is shaped
     * and written by glyph id, one without is written by character.
     */
    val asset: String? = null,
    /**
     * What it is for, shown under the name.
     *
     * The name itself is written in the script the font draws — a reader
     * looking for Persian finds نستعلیق by recognising it, which no amount of
     * English labelling achieves. This line is what tells everybody else what
     * they are looking at.
     */
    val script: String = "Latin",
) {
    HELVETICA("Helvetica", "Helvetica", "sans-serif", false),
    HELVETICA_BOLD("Helvetica Bold", "Helvetica-Bold", "sans-serif", true),
    TIMES("Times", "Times-Roman", "serif", false),
    TIMES_BOLD("Times Bold", "Times-Bold", "serif", true),
    COURIER("Courier", "Courier", "monospace", false),

    // Noto Sans and Serif carry Latin, Greek and Cyrillic between them, so most
    // European languages are covered by the two of them.
    NOTO_SANS(
        "Noto Sans", "Helvetica", "sans-serif", false,
        "NotoSans-Regular.ttf", "Latin · Greek · Cyrillic",
    ),
    NOTO_SANS_BOLD(
        "Noto Sans Bold", "Helvetica-Bold", "sans-serif", true,
        "NotoSans-Bold.ttf", "Latin · Greek · Cyrillic",
    ),
    NOTO_SERIF(
        "Noto Serif", "Times-Roman", "serif", false,
        "NotoSerif-Regular.ttf", "Latin · Greek · Cyrillic",
    ),
    NOTO_SERIF_BOLD(
        "Noto Serif Bold", "Times-Bold", "serif", true,
        "NotoSerif-Bold.ttf", "Latin · Greek · Cyrillic",
    ),

    NOTO_NASKH_ARABIC(
        "نسخ", "Helvetica", "sans-serif", false,
        "NotoNaskhArabic-Regular.ttf", "Arabic · Persian · Urdu",
    ),
    NOTO_NASKH_ARABIC_BOLD(
        "نسخ عريض", "Helvetica-Bold", "sans-serif", true,
        "NotoNaskhArabic-Bold.ttf", "Arabic · Persian · Urdu",
    ),
    // Two more Arabic hands, because Naskh is only one of them: Kufi is the
    // angular one used on signage and headings, and Noto Sans Arabic is the
    // upright sans that pairs with Latin body text.
    NOTO_KUFI_ARABIC(
        "كوفي", "Helvetica", "sans-serif", false,
        "NotoKufiArabic-Regular.ttf", "Arabic · Persian · Urdu",
    ),
    NOTO_SANS_ARABIC(
        "عربي", "Helvetica", "sans-serif", false,
        "NotoSansArabic-Regular.ttf", "Arabic · Persian · Urdu",
    ),
    // The two brought to this by hand. Nastaliq is the hand Persian is actually
    // written in, and neither has a Latin equivalent to fall back on.
    IRAN_NASTALIQ(
        "نستعلیق", "Helvetica", "sans-serif", false,
        "IranNastaliq.ttf", "Persian",
    ),
    SHEKASTEH(
        "شکسته", "Helvetica", "sans-serif", false,
        "Shekasteh.ttf", "Persian",
    ),

    NOTO_SANS_DEVANAGARI(
        "देवनागरी", "Helvetica", "sans-serif", false,
        "NotoSansDevanagari-Regular.ttf", "Hindi · Marathi · Nepali",
    ),
    NOTO_SANS_BENGALI(
        "বাংলা", "Helvetica", "sans-serif", false,
        "NotoSansBengali-Regular.ttf", "Bengali · Assamese",
    ),
    NOTO_SANS_TAMIL(
        "தமிழ்", "Helvetica", "sans-serif", false,
        "NotoSansTamil-Regular.ttf", "Tamil",
    ),
    NOTO_SANS_THAI(
        "ไทย", "Helvetica", "sans-serif", false,
        "NotoSansThai-Regular.ttf", "Thai",
    ),
    NOTO_SANS_HEBREW(
        "עברית", "Helvetica", "sans-serif", false,
        "NotoSansHebrew-Regular.ttf", "Hebrew",
    ),

    // The four CJK faces. Each is nine to sixteen megabytes and holds twenty to
    // thirty thousand glyphs, which is why nothing is embedded whole: a caption
    // is subset down to the handful of glyphs it uses before it goes in.
    //
    // Four rather than one, because "CJK" is not one script. Simplified and
    // traditional Chinese draw many of the same characters differently, and
    // Japanese draws some of them differently again — a shared file has to pick
    // one, and picks wrong for the other two.
    NOTO_SANS_SC(
        "简体中文", "Helvetica", "sans-serif", false,
        "NotoSansSC.ttf", "Simplified Chinese",
    ),
    NOTO_SANS_TC(
        "繁體中文", "Helvetica", "sans-serif", false,
        "NotoSansTC.ttf", "Traditional Chinese",
    ),
    NOTO_SANS_JP(
        "日本語", "Helvetica", "sans-serif", false,
        "NotoSansJP.ttf", "Japanese",
    ),
    NOTO_SANS_KR(
        "한국어", "Helvetica", "sans-serif", false,
        "NotoSansKR.ttf", "Korean",
    ),
    ;

    /** Whether writing in this font puts a font file inside the document. */
    val isEmbedded: Boolean get() = asset != null
}

/**
 * How far the pen moves after drawing [character], in points at [sizePoints].
 *
 * From the fonts' own metrics rather than from anything the phone measures. The
 * text is laid out once — here — and both the preview and the glyph positions
 * written into the file come from that layout, so a curve of text on screen sits
 * exactly where the curve of text in the PDF will.
 *
 * Anything outside the printable range takes the width of a space, which is the
 * least wrong answer available: it keeps the rest of the line where it belongs
 * instead of collapsing it.
 */
fun PdfFont.advanceOf(character: Char, sizePoints: Float): Float {
    // A bundled font's widths come from the font itself, through the shaper. One
    // character on its own is the only thing that can be answered here; anything
    // longer has to be shaped, because in most scripts the width of a letter
    // depends on its neighbours.
    if (asset != null) {
        val run = BundledFonts.shape(this, character.toString(), sizePoints)
        if (run.glyphs.isNotEmpty()) return run.width
    }
    val widths = widthTable()
    val index = character.code - FIRST_PRINTABLE
    val thousandths = widths.getOrNull(index) ?: widths[0]
    return thousandths * sizePoints / 1000f
}

/** How wide [text] runs, in points at [sizePoints]. */
fun PdfFont.widthOf(text: String, sizePoints: Float): Float {
    if (asset != null) {
        val run = BundledFonts.shape(this, text, sizePoints)
        if (run.glyphs.isNotEmpty()) return run.width
    }
    return text.sumOf { advanceOf(it, sizePoints).toDouble() }.toFloat()
}

/**
 * The metric table to lay out from.
 *
 * A bundled font reaches here only when shaping failed, and then the table for
 * whatever standard face it most resembles is the least wrong answer available:
 * it keeps the line roughly the right length instead of collapsing it.
 */
private fun PdfFont.widthTable(): IntArray = when (wireName) {
    "Helvetica-Bold" -> HELVETICA_BOLD_WIDTHS
    "Times-Roman" -> TIMES_WIDTHS
    "Times-Bold" -> TIMES_BOLD_WIDTHS
    "Courier" -> COURIER_WIDTHS
    else -> HELVETICA_WIDTHS
}

/** Space, the first character the tables cover. */
private const val FIRST_PRINTABLE = 32

/**
 * Advance widths in thousandths of the point size, for space through tilde.
 *
 * The fonts' published metrics. Written out rather than measured at runtime
 * because the whole point is that the app and the file agree, and the only way to
 * agree with a font nobody has is to hold its numbers.
 */
private val HELVETICA_WIDTHS = intArrayOf(
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
)

private val HELVETICA_BOLD_WIDTHS = intArrayOf(
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611,
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556,
    333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611,
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
)

private val TIMES_WIDTHS = intArrayOf(
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278,
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444,
    921, 722, 667, 667, 722, 611, 556, 722, 722, 333, 389, 722, 611, 889, 722, 722,
    556, 722, 667, 556, 611, 722, 722, 944, 722, 722, 611, 333, 278, 333, 469, 500,
    333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500, 278, 778, 500, 500,
    500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541,
)

private val TIMES_BOLD_WIDTHS = intArrayOf(
    250, 333, 555, 500, 500, 1000, 833, 278, 333, 333, 500, 570, 250, 333, 250, 278,
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500,
    930, 722, 667, 722, 722, 667, 611, 778, 778, 389, 500, 778, 667, 944, 722, 778,
    611, 778, 722, 556, 667, 722, 722, 1000, 722, 722, 667, 333, 278, 333, 581, 500,
    333, 500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556, 278, 833, 556, 500,
    556, 556, 444, 389, 333, 556, 500, 722, 500, 500, 444, 394, 220, 394, 520,
)

/** Courier is monospaced: every glyph the same, which is the point of it. */
private val COURIER_WIDTHS = IntArray(95) { 600 }

/**
 * The largest size at which [text] still fits across [availableWidth].
 *
 * The ceiling on a caption is the page, not a number: type is only as big as
 * useful while the words still fit on the sheet they are written on. Widths
 * scale linearly with the point size, so this is one division rather than a
 * search.
 */
fun PdfFont.sizeThatFits(text: String, availableWidth: Float): Float {
    if (text.isEmpty() || availableWidth <= 0f) return MAXIMUM_TEXT_POINTS
    // The widest line decides: a block is as wide as its longest line, and
    // measuring the whole string as one run would make two short lines look far
    // too wide to fit and hold the caption down to a size it did not need.
    val atOnePoint = text.captionLines().maxOf { widthOf(it, 1f) }
    if (atOnePoint <= 0f) return MAXIMUM_TEXT_POINTS
    return (availableWidth / atOnePoint).coerceIn(MINIMUM_TEXT_POINTS, MAXIMUM_TEXT_POINTS)
}
