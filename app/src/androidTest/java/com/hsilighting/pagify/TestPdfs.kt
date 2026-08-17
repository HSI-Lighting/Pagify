package com.hsilighting.pagify

/**
 * Minimal, valid PDFs built at runtime.
 *
 * Generated rather than checked in so the test suite carries no binary fixtures
 * and every byte a test depends on is visible right here. They are hand-written
 * to the spec — correct `xref` offsets included — because PDFium's leniency
 * varies and a fixture that only *happens* to load is a flaky test waiting to
 * happen.
 */
object TestPdfs {

    /** A4, two blank pages, with a title in the document information dictionary. */
    fun twoPages(): ByteArray = build(
        listOf(
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] >>",
            "<< /Title (Pagify Test Fixture) /Author (Pagify) >>",
        ),
        rootObject = 1,
        infoObject = 5,
    )

    /**
     * A single 200x200 page painted pure red.
     *
     * Red specifically: it is the channel that moves when the BGRA/RGBA swap in
     * `render/bitmap.rs` is wrong, so a transposition shows up as blue rather
     * than as a subtle shift.
     */
    fun redSquare(): ByteArray = solidSquare("1 0 0 rg")

    /**
     * A single 200x200 page painted orange — R 255, G ~128, B 0.
     *
     * Red alone cannot distinguish "R and B are swapped" from "the fixture is
     * actually blue". An asymmetric colour pins the byte order down: only one
     * interpretation puts 128 in the green channel with 255 and 0 either side of
     * it, and which side they land on is the whole question.
     */
    fun orangeSquare(): ByteArray = solidSquare("1 0.5 0 rg")

    private fun solidSquare(colorOperator: String): ByteArray {
        val contentStream = "$colorOperator\n0 0 200 200 re\nf\n"
        return build(
            listOf(
                "<< /Type /Catalog /Pages 2 0 R >>",
                "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>",
                "<< /Length ${contentStream.length} >>\nstream\n${contentStream}endstream",
            ),
            rootObject = 1,
            infoObject = null,
        )
    }

    /**
     * Assembles numbered objects into a complete file.
     *
     * The `xref` table stores each object's *byte offset from the start of the
     * file*, so the body has to be laid out before the table can be written — a
     * table with stale offsets produces a file most viewers reject outright.
     */
    private fun build(
        objects: List<String>,
        rootObject: Int,
        infoObject: Int?,
    ): ByteArray {
        val header = "%PDF-1.7\n"
        val body = StringBuilder(header)
        val offsets = mutableListOf<Int>()

        objects.forEachIndexed { index, content ->
            // ASCII-only content, so character count equals byte count.
            offsets += body.length
            body.append("${index + 1} 0 obj\n").append(content).append("\nendobj\n")
        }

        val xrefOffset = body.length
        // Entry count is objects + 1 for the mandatory free object 0.
        body.append("xref\n0 ${objects.size + 1}\n")
        body.append("0000000000 65535 f \n")
        offsets.forEach { offset ->
            // Exactly 20 bytes per entry, as the spec requires.
            body.append(String.format("%010d 00000 n \n", offset))
        }

        body.append("trailer\n<< /Size ${objects.size + 1} /Root $rootObject 0 R")
        if (infoObject != null) body.append(" /Info $infoObject 0 R")
        body.append(" >>\n")
        body.append("startxref\n$xrefOffset\n%%EOF\n")

        return body.toString().toByteArray(Charsets.US_ASCII)
    }
}
