package com.hsilighting.pagify.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The Rust side omits absent keys rather than sending nulls, which is why the
 * parser cannot use `optString` — that would turn "this document declares no
 * author" into an empty-string author and put a blank line in the metadata
 * sheet. These pin that distinction.
 */
class PdfMetadataTest {

    @Test
    fun aFullyPopulatedDocumentParses() {
        val json = """
            {"title":"Catalogue","author":"HSI","subject":"Lighting","keywords":"lamps",
             "creator":"InDesign","producer":"Distiller",
             "creationDate":"D:20170131120000Z","modificationDate":"D:20170201090000Z",
             "pageCount":95}
        """.trimIndent()

        val metadata = PdfMetadata.fromJson(json)

        assertEquals("Catalogue", metadata.title)
        assertEquals("HSI", metadata.author)
        assertEquals("Lighting", metadata.subject)
        assertEquals("lamps", metadata.keywords)
        assertEquals("InDesign", metadata.creator)
        assertEquals("Distiller", metadata.producer)
        assertEquals("D:20170131120000Z", metadata.creationDate)
        assertEquals("D:20170201090000Z", metadata.modificationDate)
        assertEquals(95, metadata.pageCount)
    }

    @Test
    fun absentKeysBecomeNullNotEmptyString() {
        val metadata = PdfMetadata.fromJson("""{"pageCount":3}""")

        assertNull(metadata.title)
        assertNull(metadata.author)
        assertNull(metadata.creationDate)
        assertEquals(3, metadata.pageCount)
    }

    @Test
    fun explicitJsonNullsAlsoBecomeNull() {
        val metadata = PdfMetadata.fromJson("""{"title":null,"author":null,"pageCount":1}""")
        assertNull(metadata.title)
        assertNull(metadata.author)
    }

    @Test
    fun aDocumentDeclaringNothingStillParses() {
        val metadata = PdfMetadata.fromJson("{}")
        assertNull(metadata.title)
        assertEquals(0, metadata.pageCount)
    }

    @Test
    fun theTitleBarFallsBackToTheFilenameWhenTheTitleIsUnusable() {
        assertEquals("file.pdf", PdfMetadata().displayTitle("file.pdf"))
        assertEquals("file.pdf", PdfMetadata(title = "").displayTitle("file.pdf"))
        assertEquals("file.pdf", PdfMetadata(title = "   ").displayTitle("file.pdf"))
        assertEquals("Real Title", PdfMetadata(title = "Real Title").displayTitle("file.pdf"))
    }
}

/**
 * `PdfCacheStats` is a measuring instrument, so its arithmetic is worth pinning:
 * a wrong hit rate does not break the app, it misleads whoever is tuning it.
 */
class PdfCacheStatsTest {

    @Test
    fun statsParseFromTheEngineJson() {
        val stats = PdfCacheStats.fromJson(
            """{"hits":12,"misses":4,"entries":3,"usedBytes":50331648,"budgetBytes":167772160}""",
        )
        assertEquals(12L, stats.hits)
        assertEquals(4L, stats.misses)
        assertEquals(3, stats.entries)
        assertEquals(50331648L, stats.usedBytes)
        assertEquals(167772160L, stats.budgetBytes)
        assertEquals(0.75f, stats.hitRate, 1e-6f)
    }

    @Test
    fun anIdleCacheReportsNoHitRateRatherThanDividingByZero() {
        val stats = PdfCacheStats(hits = 0, misses = 0, entries = 0, usedBytes = 0, budgetBytes = 1)
        assertEquals(0f, stats.hitRate, 0f)
    }

    @Test
    fun aPerfectAndAHopelessCacheBothReportSensibly() {
        assertEquals(1f, PdfCacheStats(9, 0, 1, 0, 1).hitRate, 1e-6f)
        assertEquals(0f, PdfCacheStats(0, 9, 1, 0, 1).hitRate, 1e-6f)
    }
}
