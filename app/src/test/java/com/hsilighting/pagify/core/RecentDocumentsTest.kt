package com.hsilighting.pagify.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Locale

/**
 * The library's ordering and its file.
 *
 * Both are wrong in ways nobody notices until the list is long: a document that
 * appears twice, a list that never stops growing, a file that loses everything
 * because one entry was written by an older build. Pure functions, so all of it
 * can be checked without a device.
 */
class RecentDocumentsTest {

    /**
     * A library entry, distinct from every other one by default.
     *
     * **The name and the size follow the URI unless a test says otherwise**,
     * because they are part of what makes two entries the same document — see
     * `promoteRecent`. Fixing them for every fixture made a list of ten
     * different documents a list of ten copies of one, so an ordering test
     * would have been checking the order of something the library correctly
     * refuses to hold.
     */
    private fun doc(
        uri: String,
        name: String = "Report $uri.pdf",
        openedAt: Long = 1_000L,
        sizeBytes: Long = 2_500_000L + uri.hashCode().toLong(),
    ) = RecentDocument(
        uri = uri,
        name = name,
        sizeBytes = sizeBytes,
        pageCount = 12,
        openedAtMillis = openedAt,
    )

    @Test
    fun `the newest document is first`() {
        val list = promoteRecent(listOf(doc("a"), doc("b")), doc("c"))
        assertEquals(listOf("c", "a", "b"), list.map { it.uri })
    }

    @Test
    fun `reopening a document moves it rather than duplicating it`() {
        val list = promoteRecent(listOf(doc("a"), doc("b"), doc("c")), doc("c", openedAt = 9_000L))

        assertEquals(listOf("c", "a", "b"), list.map { it.uri })
        assertEquals(9_000L, list.first().openedAtMillis)
    }

    /**
     * **One document opened two ways is one row.**
     *
     * The reported bug. A PDF handed over by another app arrives under that
     * app's own provider; the same file opened from this app's picker arrives
     * under storage's. The two strings have nothing in common and neither can
     * be turned into the other, so matching on the URI alone listed the
     * document once per route it had ever been opened by.
     */
    @Test
    fun `the same file opened from another app is not a second entry`() {
        val fromAnotherApp = doc(
            "content://com.google.android.apps.docs.storage/document/acc=1;doc=99",
            name = "Report.pdf",
            sizeBytes = 2_500_000L,
        )
        val fromThePicker = doc(
            "content://com.android.externalstorage.documents/document/primary%3ADownload%2FReport.pdf",
            name = "Report.pdf",
            openedAt = 9_000L,
            sizeBytes = 2_500_000L,
        )

        val list = promoteRecent(listOf(fromAnotherApp), fromThePicker)

        assertEquals(1, list.size)
        // The newest URI, because it is the one just proven to open.
        assertEquals(fromThePicker.uri, list.single().uri)
    }

    /** Two genuinely different files are still two rows. */
    @Test
    fun `a different file of the same name is still its own entry`() {
        val one = doc("content://x/1", name = "Report.pdf", sizeBytes = 2_500_000L)
        val other = doc("content://x/2", name = "Report.pdf", sizeBytes = 900_000L)

        assertEquals(2, promoteRecent(listOf(one), other).size)
    }

    /**
     * A size of zero is the provider declining to say, not a size.
     *
     * Two documents it would not measure must not collapse into each other on
     * the strength of a shared name.
     */
    @Test
    fun `an unknown size matches nothing`() {
        val one = doc("content://x/1", name = "scan.pdf", sizeBytes = 0L)
        val other = doc("content://x/2", name = "scan.pdf", sizeBytes = 0L)

        assertEquals(2, promoteRecent(listOf(one), other).size)
    }

    /** A drawing and a document of the same name are different things. */
    @Test
    fun `entries of different kinds do not collapse`() {
        val document = doc("content://x/1", name = "Plan", sizeBytes = 4_000L)
        val drawing = document.copy(uri = "content://x/2", kind = RecentKind.Drawing)

        assertEquals(2, promoteRecent(listOf(document), drawing).size)
    }

    /**
     * And a library that already has the twins in it loses them on first read.
     *
     * The duplicates are written to the file on phones this ships to, so
     * waiting for each document to be opened once more would leave the list
     * looking broken until it happened to be.
     */
    @Test
    fun `duplicates already on disk are collapsed when read`() {
        val documents = listOf(
            doc("content://picker/Report.pdf", name = "Report.pdf", openedAt = 9_000L, sizeBytes = 2_500_000L),
            doc("content://other-app/99", name = "Report.pdf", openedAt = 1_000L, sizeBytes = 2_500_000L),
            doc("content://x/2", name = "Invoice.pdf", sizeBytes = 700_000L),
        )

        val read = recentsFromJson(documents.toRecentsJson())

        assertEquals(listOf("Report.pdf", "Invoice.pdf"), read.map { it.name })
        assertEquals("content://picker/Report.pdf", read.first().uri)
    }

    @Test
    fun `the list stops growing`() {
        var list = emptyList<RecentDocument>()
        repeat(RECENT_DOCUMENT_LIMIT + 10) { index -> list = promoteRecent(list, doc("uri-$index")) }

        assertEquals(RECENT_DOCUMENT_LIMIT, list.size)
        // The oldest are the ones dropped, not the newest.
        assertEquals("uri-${RECENT_DOCUMENT_LIMIT + 9}", list.first().uri)
    }

    @Test
    fun `a document can be forgotten`() {
        val list = forgetRecent(listOf(doc("a"), doc("b")), "a")
        assertEquals(listOf("b"), list.map { it.uri })
    }

    @Test
    fun `search matches part of a name, in any case`() {
        val documents = listOf(
            doc("a", name = "2024-NDA-final.pdf"),
            doc("b", name = "Invoice 88.pdf"),
        )

        assertEquals(listOf("a"), searchRecents(documents, "nda").map { it.uri })
        assertEquals(listOf("b"), searchRecents(documents, "INVOICE").map { it.uri })
    }

    @Test
    fun `an empty search is not a filter`() {
        val documents = listOf(doc("a"), doc("b"))
        assertEquals(documents, searchRecents(documents, ""))
        assertEquals(documents, searchRecents(documents, "   "))
    }

    @Test
    fun `the file round trips`() {
        val documents = listOf(doc("content://x/1", name = "One.pdf"), doc("content://x/2"))
        assertEquals(documents, recentsFromJson(documents.toRecentsJson()))
    }

    @Test
    fun `one unreadable entry costs one row, not the library`() {
        // The failure this is really about: a file written by an older build, or a
        // write cut short. Losing the whole list over it would be worse than the
        // bug that caused it.
        val json = """[{"uri":"content://x/1","name":"Kept.pdf"},{"name":"No URI"},"not an object"]"""
        val documents = recentsFromJson(json)

        assertEquals(1, documents.size)
        assertEquals("Kept.pdf", documents.single().name)
    }

    @Test
    fun `nonsense on disk reads as an empty library`() {
        assertEquals(emptyList<RecentDocument>(), recentsFromJson("this is not json"))
        assertEquals(emptyList<RecentDocument>(), recentsFromJson(""))
    }

    @Test
    fun `sizes read the way a person would say them`() {
        assertEquals("2.4 MB", formatFileSize(2_500_000L))
        assertEquals("156 KB", formatFileSize(159_000L))
        // Not "0 KB" for a size the provider would not give: it says nothing, so
        // it should take up no room.
        assertEquals("", formatFileSize(0L))
    }

    @Test
    fun `the subtitle drops what is not known`() {
        val known = recentSubtitle(doc("a", openedAt = 1_698_000_000_000L))
        assertTrue(known, known.contains("12 pages"))
        assertTrue(known, known.contains("2.4 MB"))

        val bare = recentSubtitle(
            RecentDocument("a", "One.pdf", sizeBytes = 0, pageCount = 0, openedAtMillis = 0),
        )
        assertEquals("", bare)
    }

    @Test
    fun `one page is not one pages`() {
        val single = recentSubtitle(
            RecentDocument("a", "One.pdf", sizeBytes = 0, pageCount = 1, openedAtMillis = 0),
        )
        assertEquals("1 page", single)
    }

    @Test
    fun `a date is formatted the way the library shows it`() {
        // Fixed locale, or this passes in London and fails everywhere else.
        assertEquals("Oct 24, 2023", formatOpenedAt(1_698_140_000_000L, Locale.UK))
        assertEquals("", formatOpenedAt(0L, Locale.UK))
    }

    // ---- models in the same library -------------------------------------------

    private fun model(uri: String = "content://part") = RecentDocument(
        uri = uri,
        name = "IMPELLER.stp",
        sizeBytes = 2_400_000,
        pageCount = 0,
        openedAtMillis = 1_698_140_000_000L,
        kind = RecentKind.Model,
    )

    /**
     * **An existing library survives the upgrade.**
     *
     * Entries written before models existed carry no kind, and there is no
     * version number in the file to tell them apart. If a missing kind did not
     * mean "document", every PDF anyone has ever opened would try to reopen in
     * the 3D viewer — and the file is rewritten on the next open, so it would
     * not even be recoverable by going back to the older build.
     */
    @Test
    fun `an entry saved before models existed is still a document`() {
        val old = """
            [{"uri":"content://one","name":"Report.pdf","sizeBytes":1024,
              "pageCount":12,"openedAtMillis":1698140000000}]
        """.trimIndent()

        val read = recentsFromJson(old)

        assertEquals(1, read.size)
        assertEquals(RecentKind.Document, read[0].kind)
        assertEquals(12, read[0].pageCount)
    }

    /** And a kind nobody recognises is not a reason to lose the row. */
    @Test
    fun `an unknown kind reads as a document rather than vanishing`() {
        val read = recentsFromJson(
            """[{"uri":"content://one","name":"Report.pdf","kind":"hologram"}]""",
        )

        assertEquals(1, read.size)
        assertEquals(RecentKind.Document, read[0].kind)
    }

    @Test
    fun `a model survives being written and read back`() {
        val read = recentsFromJson(listOf(model()).toRecentsJson())

        assertEquals(1, read.size)
        assertEquals(RecentKind.Model, read[0].kind)
        assertEquals("IMPELLER.stp", read[0].name)
    }

    /**
     * The row says what it is instead of showing a count of nothing.
     *
     * A model has no pages, so without this the only thing separating a part
     * from a document in the library is a missing figure — which reads as a
     * document that failed to open.
     */
    @Test
    fun `a model says so rather than showing no pages`() {
        val line = recentSubtitle(model())

        assertTrue(line, line.contains("3D model"))
        assertFalse(line, line.contains("page"))
    }

    // ---- which viewer opens what -------------------------------------------

    /**
     * **The file name decides, and the mistakes are silent ones.**
     *
     * Opening a DWG in the PDF reader gives an error about a corrupt
     * document; opening a PDF in the drawing viewer gives an empty sheet.
     * Neither says "wrong viewer", which is why this is worth pinning rather
     * than leaving to a `when` somebody edits later.
     */
    @Test
    fun `the extension decides which viewer opens a file`() {
        assertEquals(RecentKind.Drawing, kindOfFile("PROPOSED LIGHTING LAYOUT.dwg"))
        assertEquals(RecentKind.Drawing, kindOfFile("plan.dxf"))
        assertEquals(RecentKind.Model, kindOfFile("IMPELLER.stp"))
        assertEquals(RecentKind.Model, kindOfFile("Amplifier_ZK1002T.step"))
        assertEquals(RecentKind.Document, kindOfFile("Invoice 88.pdf"))
    }

    /** Whatever case it is written in. */
    @Test
    fun `the extension is read whichever case it is in`() {
        assertEquals(RecentKind.Drawing, kindOfFile("3L MASTER BASE V6.DWG"))
        assertEquals(RecentKind.Model, kindOfFile("BUTTON NEW - V01.STEP"))
    }

    /**
     * A name with dots in it is read from the last one.
     *
     * "Villa-MI....dwg" is a real file on this machine, and reading from the
     * first dot makes it a document.
     */
    @Test
    fun `only the last extension counts`() {
        assertEquals(RecentKind.Drawing, kindOfFile("Villa-MI....dwg"))
        assertEquals(RecentKind.Document, kindOfFile("report.dwg.pdf"))
    }

    /** And anything unrecognised goes to the reader, which explains itself. */
    @Test
    fun `an unknown file is offered to the reader`() {
        assertEquals(RecentKind.Document, kindOfFile("notes"))
        assertEquals(RecentKind.Document, kindOfFile("archive.zip"))
        assertEquals(RecentKind.Document, kindOfFile(""))
    }

    /** Both kinds live in one list, newest first, with no duplicates. */
    @Test
    fun `a model and a document share the one library`() {
        val document = RecentDocument(
            "content://doc",
            "Report.pdf",
            sizeBytes = 10,
            pageCount = 3,
            openedAtMillis = 1,
        )

        val library = promoteRecent(promoteRecent(emptyList(), document), model())

        assertEquals(2, library.size)
        assertEquals(RecentKind.Model, library[0].kind)
        assertEquals(RecentKind.Document, library[1].kind)

        // Reopening the model promotes it rather than adding a twin.
        val again = promoteRecent(library, model())
        assertEquals(2, again.size)
    }
}
