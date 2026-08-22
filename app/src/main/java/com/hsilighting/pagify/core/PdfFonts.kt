package com.hsilighting.pagify.core

/**
 * The fonts text can be written in.
 *
 * The PDF standard set, and only those. They need no embedding, cost nothing in
 * file size, and are present in every reader ever made — which matters because a
 * marked-up drawing is almost always going to somebody else. A font that renders
 * as something different on the recipient's machine is worse than a plain one.
 *
 * [family] is the nearest thing Android has, for drawing the preview. It is a
 * likeness, not the font: the preview is drawn with the phone's own faces and only
 * becomes the real thing once the page is re-rendered from the saved PDF. What the
 * two *do* agree on is where every glyph sits, because both lay out from
 * [advanceOf] rather than from whatever the phone happens to measure.
 */
enum class PdfFont(
    val label: String,
    /** The name PDFium's `FPDFText_LoadStandardFont` expects. */
    val wireName: String,
    val family: String,
    val bold: Boolean,
) {
    HELVETICA("Helvetica", "Helvetica", "sans-serif", false),
    HELVETICA_BOLD("Helvetica Bold", "Helvetica-Bold", "sans-serif", true),
    TIMES("Times", "Times-Roman", "serif", false),
    TIMES_BOLD("Times Bold", "Times-Bold", "serif", true),
    COURIER("Courier", "Courier", "monospace", false),
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
    val widths = widthTable()
    val index = character.code - FIRST_PRINTABLE
    val thousandths = widths.getOrNull(index) ?: widths[0]
    return thousandths * sizePoints / 1000f
}

/** How wide [text] runs, in points at [sizePoints]. */
fun PdfFont.widthOf(text: String, sizePoints: Float): Float =
    text.sumOf { advanceOf(it, sizePoints).toDouble() }.toFloat()

private fun PdfFont.widthTable(): IntArray = when (this) {
    PdfFont.HELVETICA -> HELVETICA_WIDTHS
    PdfFont.HELVETICA_BOLD -> HELVETICA_BOLD_WIDTHS
    PdfFont.TIMES -> TIMES_WIDTHS
    PdfFont.TIMES_BOLD -> TIMES_BOLD_WIDTHS
    PdfFont.COURIER -> COURIER_WIDTHS
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
    val atOnePoint = widthOf(text, 1f)
    if (atOnePoint <= 0f) return MAXIMUM_TEXT_POINTS
    return (availableWidth / atOnePoint).coerceIn(MINIMUM_TEXT_POINTS, MAXIMUM_TEXT_POINTS)
}
