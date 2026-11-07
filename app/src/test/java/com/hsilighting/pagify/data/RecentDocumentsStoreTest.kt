package com.hsilighting.pagify.data

import com.hsilighting.pagify.core.ReadingPlace
import com.hsilighting.pagify.core.RecentDocument
import com.hsilighting.pagify.core.RecentKind
import com.hsilighting.pagify.core.recentsFromJson
import com.hsilighting.pagify.core.toRecentsJson
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * The library file against a change that arrives before it has been read.
 *
 * A cold start from another app's "Open with" remembers the opened drawing
 * while `load()` is still running. Before the fix that wrote out a list of one
 * row over the whole library — seen twice on the phone on 2026-10-12. Here the
 * worst case is forced: the change comes before `load()` is called at all.
 */
class RecentDocumentsStoreTest {

    @get:Rule
    val folder = TemporaryFolder()

    private fun drawing(uri: String) = RecentDocument(
        uri = uri,
        name = "$uri.dwg",
        sizeBytes = 1_000L + uri.length,
        pageCount = 0,
        openedAtMillis = 1_000L,
        kind = RecentKind.Drawing,
    )

    private fun libraryOf(vararg uris: String) = folder.newFile().apply {
        writeText(uris.map(::drawing).toRecentsJson())
    }

    @Test
    fun `remembering before the library has loaded keeps what was already there`() = runBlocking<Unit> {
        val file = libraryOf("a", "b")
        val store = RecentDocumentsStore(file)

        store.remember(drawing("opened"))

        assertEquals(listOf("opened", "a", "b"), recentsFromJson(file.readText()).map { it.uri })
        assertEquals(listOf("opened", "a", "b"), store.documents.value.map { it.uri })
    }

    @Test
    fun `forgetting before the library has loaded drops only that row`() = runBlocking<Unit> {
        val file = libraryOf("a", "b", "c")
        val store = RecentDocumentsStore(file)

        store.forget("b")
        store.load()

        assertEquals(listOf("a", "c"), recentsFromJson(file.readText()).map { it.uri })
        assertEquals(listOf("a", "c"), store.documents.value.map { it.uri })
    }

    /** Where a document is being read is stored on its row, and only its row. */
    @Test
    fun `a reading place is stored on its own document and survives a restart`() = runBlocking<Unit> {
        val file = libraryOf("a", "b")
        val place = ReadingPlace(page = 3, zoom = 2f, x = 0.4f, y = 0.6f)

        RecentDocumentsStore(file).rememberPlace("b", place)

        val reread = RecentDocumentsStore(file).apply { load() }.documents.value
        assertEquals(listOf("a", "b"), reread.map { it.uri })
        assertEquals(listOf(null, place), reread.map { it.place })
    }

    /** What `open()` restores from: the row as stored, place carried across. */
    @Test
    fun `remembering a document hands back where it was being read`() = runBlocking<Unit> {
        val file = libraryOf("a")
        val store = RecentDocumentsStore(file)
        val place = ReadingPlace(page = 5, y = 0.25f)
        store.rememberPlace("a", place)

        assertEquals(place, store.remember(drawing("a")).place)
    }

    /** Turning the setting off: no record of where anything was read, every row kept. */
    @Test
    fun `forgetting places keeps every document`() = runBlocking<Unit> {
        val file = libraryOf("a", "b")
        RecentDocumentsStore(file).apply {
            rememberPlace("a", ReadingPlace(page = 2))
            rememberPlace("b", ReadingPlace(page = 4, zoom = 3f))
            forgetPlaces()
        }

        val reread = recentsFromJson(file.readText())
        assertEquals(listOf("a", "b"), reread.map { it.uri })
        assertEquals(listOf(null, null), reread.map { it.place })
    }
}
