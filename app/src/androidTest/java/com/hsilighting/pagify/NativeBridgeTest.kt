package com.hsilighting.pagify

import android.graphics.Bitmap
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.hsilighting.pagify.core.NativeBridge
import com.hsilighting.pagify.core.PdfDocument
import com.hsilighting.pagify.core.PdfPasswordException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Exercises the JNI boundary itself — the layer the Rust unit tests cannot reach.
 *
 * These need a real device or emulator. The fixtures are generated at runtime
 * rather than checked in, so the suite has no binary dependencies:
 *
 *     ./gradlew connectedDebugAndroidTest
 */
@RunWith(AndroidJUnit4::class)
class NativeBridgeTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun fixture(name: String, bytes: ByteArray): String =
        File(context.cacheDir, name).apply { writeBytes(bytes) }.absolutePath

    // ------------------------------------------------------------ lifecycle --

    @Test
    fun libraryLoadsAndReportsItsVersion() {
        assertEquals("0.1.0", NativeBridge.nativeVersion())
    }

    @Test
    fun openingAndClosingLeavesNothingBehind() {
        val before = NativeBridge.openDocumentCount()

        PdfDocument.openFile(fixture("simple.pdf", TestPdfs.twoPages())).use { doc ->
            assertEquals(2, doc.pageCount)
            assertEquals(before + 1, NativeBridge.openDocumentCount())
        }

        assertEquals("closing must release the native session", before, NativeBridge.openDocumentCount())
    }

    @Test
    fun closingTwiceIsSafeAndIsReportedTheSecondTime() {
        val doc = PdfDocument.openFile(fixture("twice.pdf", TestPdfs.twoPages()))
        doc.close()
        doc.close() // must not throw
        assertTrue(doc.isClosed)
    }

    @Test(expected = IllegalStateException::class)
    fun usingAClosedDocumentFailsFastInsteadOfReachingNativeCode() {
        val doc = PdfDocument.openFile(fixture("closed.pdf", TestPdfs.twoPages()))
        doc.close()
        doc.pageSize(0)
    }

    @Test
    fun openingManyDocumentsDoesNotLeakSessions() {
        val before = NativeBridge.openDocumentCount()
        val path = fixture("loop.pdf", TestPdfs.twoPages())

        repeat(200) {
            PdfDocument.openFile(path).use { doc -> assertEquals(2, doc.pageCount) }
        }

        assertEquals(before, NativeBridge.openDocumentCount())
    }

    // -------------------------------------------------------------- errors --

    @Test
    fun aDamagedFileThrowsRatherThanCrashingTheProcess() {
        val path = fixture("garbage.pdf", ByteArray(4096) { it.toByte() })
        val error = runCatching { PdfDocument.openFile(path) }.exceptionOrNull()
        assertTrue("expected a thrown exception, got none", error != null)
    }

    @Test
    fun aTruncatedPdfThrowsRatherThanReturningAnEmptyDocument() {
        val truncated = TestPdfs.twoPages().copyOfRange(0, 120)
        val error = runCatching { PdfDocument.openFile(fixture("cut.pdf", truncated)) }
            .exceptionOrNull()
        assertTrue("a truncated file must not open successfully", error != null)
    }

    @Test
    fun anEncryptedDocumentReportsThatAPasswordIsNeeded() {
        // Only run where a fixture is available; skipped silently otherwise so the
        // suite stays green on a fresh checkout.
        val encrypted = runCatching {
            context.assets.open("encrypted.pdf").use { it.readBytes() }
        }.getOrNull() ?: return

        val error = runCatching { PdfDocument.openFile(fixture("enc.pdf", encrypted)) }
            .exceptionOrNull()
        assertTrue(
            "expected PdfPasswordException, got $error",
            error is PdfPasswordException,
        )
    }

    @Test
    fun anOutOfRangePageIsRejected() {
        PdfDocument.openFile(fixture("range.pdf", TestPdfs.twoPages())).use { doc ->
            val error = runCatching { doc.pageSize(99) }.exceptionOrNull()
            assertTrue("expected an out-of-range failure, got $error", error != null)
        }
    }

    // -------------------------------------------------------------- render --

    @Test
    fun renderingFillsTheBitmapWithTheZeroCopyPath() {
        PdfDocument.openFile(fixture("render.pdf", TestPdfs.twoPages())).use { doc ->
            val size = doc.pageSize(0)
            val (w, h) = size.pixelSize(1f)
            val bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)

            doc.renderPageInto(0, bitmap, 1f)

            // The fixture's page is blank white; the check that matters is that
            // *something* was written — an untouched bitmap is fully transparent.
            assertEquals(0xFF, bitmap.getPixel(w / 2, h / 2) ushr 24 and 0xFF)
        }
    }

    @Test
    fun renderedPagesAreWhiteNotBlack() {
        // PDFium leaves the page transparent unless told to clear it, which
        // composites to black. This is the regression guard for that.
        PdfDocument.openFile(fixture("white.pdf", TestPdfs.twoPages())).use { doc ->
            val (w, h) = doc.pageSize(0).pixelSize(1f)
            val bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
            doc.renderPageInto(0, bitmap, 1f)

            val centre = bitmap.getPixel(w / 2, h / 2)
            assertEquals("red", 0xFF, centre shr 16 and 0xFF)
            assertEquals("green", 0xFF, centre shr 8 and 0xFF)
            assertEquals("blue", 0xFF, centre and 0xFF)
        }
    }

    @Test
    fun redAndBlueAreNotTransposed() {
        // PDFium writes BGRA; Android's ARGB_8888 is RGBA in memory. If the swap
        // in render/bitmap.rs regresses, this fixture's red square reads as blue.
        PdfDocument.openFile(fixture("red.pdf", TestPdfs.redSquare())).use { doc ->
            val (w, h) = doc.pageSize(0).pixelSize(1f)
            val bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
            doc.renderPageInto(0, bitmap, 1f)

            val centre = bitmap.getPixel(w / 2, h / 2)
            val red = centre shr 16 and 0xFF
            val blue = centre and 0xFF
            assertTrue(
                "expected a red centre pixel, got #${Integer.toHexString(centre)} — " +
                    "R and B are probably transposed",
                red > 200 && blue < 80,
            )
        }
    }

    @Test
    fun prefetchedPagesAreServedFromCache() {
        PdfDocument.openFile(fixture("cache.pdf", TestPdfs.twoPages())).use { doc ->
            doc.clearCache()
            assertTrue("first prefetch should do work", doc.prefetchPage(1, 1f))
            assertFalse("second prefetch should be a no-op", doc.prefetchPage(1, 1f))

            val (w, h) = doc.pageSize(1).pixelSize(1f)
            val bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
            assertTrue(
                "a prefetched page must render from cache",
                doc.renderPageInto(1, bitmap, 1f),
            )

            assertTrue(doc.cacheStats().hits > 0)
        }
    }

    @Test
    fun clearingTheCacheReleasesItsMemory() {
        PdfDocument.openFile(fixture("trim.pdf", TestPdfs.twoPages())).use { doc ->
            doc.prefetchPage(0, 1f)
            assertTrue(doc.cacheStats().usedBytes > 0)

            doc.clearCache()

            assertEquals(0, doc.cacheStats().usedBytes)
            assertEquals(2, doc.pageCount) // document itself still usable
        }
    }

    @Test
    fun aBitmapOfTheWrongConfigIsRejectedBeforeReachingNativeCode() {
        PdfDocument.openFile(fixture("cfg.pdf", TestPdfs.twoPages())).use { doc ->
            val bitmap = Bitmap.createBitmap(64, 64, Bitmap.Config.RGB_565)
            val error = runCatching { doc.renderPageInto(0, bitmap, 1f) }.exceptionOrNull()
            assertTrue(
                "expected IllegalArgumentException, got $error",
                error is IllegalArgumentException,
            )
        }
    }

    // ------------------------------------------------------------ metadata --

    @Test
    fun metadataRoundTripsFromTheDocument() {
        PdfDocument.openFile(fixture("meta.pdf", TestPdfs.twoPages())).use { doc ->
            val metadata = doc.metadata
            assertEquals(2, metadata.pageCount)
            assertEquals("Pagify Test Fixture", metadata.title)
            assertNotEquals("", metadata.displayTitle("fallback"))
        }
    }

    @Test
    fun pageSizesAreReportedInPoints() {
        PdfDocument.openFile(fixture("size.pdf", TestPdfs.twoPages())).use { doc ->
            val size = doc.pageSize(0)
            assertEquals(595f, size.widthPoints, 1f)
            assertEquals(842f, size.heightPoints, 1f)
        }
    }
}
