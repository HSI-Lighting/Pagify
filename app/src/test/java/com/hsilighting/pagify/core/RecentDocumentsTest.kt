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

    private fun doc(
        uri: String,
        name: String = "Report.pdf",
        openedAt: Long = 1_000L,
    ) = RecentDocument(
        uri = uri,
        name = name,
        sizeBytes = 2_500_000L,
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
