package com.hsilighting.pagify.ui.reader

import com.hsilighting.pagify.core.EditState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The state snapshot is pure, so what it derives can be pinned here.
 *
 * The genuinely interesting logic — the zoom/pin state machine — lives in
 * `PdfReaderViewModel`, which extends `AndroidViewModel` and so cannot be
 * reached from a JVM test at all. That is the next thing worth changing; see
 * the note at the foot of this file.
 */
class PdfReaderStateTest {

    @Test
    fun thePageLabelIsOneBased() {
        assertEquals(1, PdfReaderState(currentPage = 0, pageCount = 95).currentPageLabel)
        assertEquals(95, PdfReaderState(currentPage = 94, pageCount = 95).currentPageLabel)
    }

    /**
     * Guards the symptom behind the one-off "Page 17 of 149" on open: a stale
     * index arriving after the document changed must not label a page past the
     * end of the new one.
     */
    @Test
    fun aStaleIndexCannotLabelAPagePastTheEnd() {
        assertEquals(95, PdfReaderState(currentPage = 400, pageCount = 95).currentPageLabel)
    }

    @Test
    fun anEmptyDocumentStillLabelsSensibly() {
        assertEquals(1, PdfReaderState(currentPage = 0, pageCount = 0).currentPageLabel)
    }

    @Test
    fun onlyTheReadyPhaseIsReady() {
        assertTrue(PdfReaderState(phase = PdfReaderState.Phase.Ready).isReady)
        assertFalse(PdfReaderState(phase = PdfReaderState.Phase.Empty).isReady)
        assertFalse(PdfReaderState(phase = PdfReaderState.Phase.Loading).isReady)
        assertFalse(PdfReaderState(phase = PdfReaderState.Phase.Failed("nope")).isReady)
        assertFalse(
            PdfReaderState(phase = PdfReaderState.Phase.PasswordRequired(retry = false)).isReady,
        )
    }

    @Test
    fun aFreshStateIsEmptyAndUnzoomed() {
        val fresh = PdfReaderState()
        assertEquals(PdfReaderState.Phase.Empty, fresh.phase)
        assertEquals(PdfReaderState.FIT_WIDTH_ZOOM, fresh.zoom, 0f)
        assertEquals(null, fresh.zoomedPage)
        assertEquals(0, fresh.rotationQuarterTurns)
    }

    /** The pin is released at fit-width and held above it; see `zoomedPage`. */
    @Test
    fun theZoomBoundsBracketFitWidth() {
        assertTrue(PdfReaderState.MIN_ZOOM <= PdfReaderState.FIT_WIDTH_ZOOM)
        assertTrue(PdfReaderState.MAX_ZOOM > PdfReaderState.FIT_WIDTH_ZOOM)
    }

    // ----------------------------------------- turning pages while magnified --

    @Test
    fun aSwipePastTheEndMovesToTheNextPage() {
        val magnified = PdfReaderState(zoomedPage = 3, pageCount = 51)
        assertEquals(4, magnified.pageAfterTurn(1))
        assertEquals(2, magnified.pageAfterTurn(-1))
    }

    @Test
    fun thereIsNowhereToGoPastEitherEndOfTheDocument() {
        // The pull springs back instead, which is what says the document has
        // ended rather than the page.
        assertNull(PdfReaderState(zoomedPage = 0, pageCount = 5).pageAfterTurn(-1))
        assertNull(PdfReaderState(zoomedPage = 4, pageCount = 5).pageAfterTurn(1))
    }

    @Test
    fun nothingTurnsWhenNoPageIsMagnified() {
        // At fit-width the list scrolls; there is no pinned page to move from,
        // and a turn here would fight the scroll for the same gesture.
        assertNull(PdfReaderState(zoomedPage = null, pageCount = 51).pageAfterTurn(1))
    }

    /**
     * Both halves of "unsaved" count.
     *
     * Reordered pages and unwritten marks are separately losable, and a prompt
     * that only knew about one of them would let the other go silently — which is
     * the whole failure this guard exists to stop.
     */
    @Test
    fun unsavedMarksCountAsWorkToLose() {
        val clean = PdfReaderState()
        assertFalse(clean.hasUnsavedWork)
        assertTrue(clean.copy(unsavedMarkCount = 1).hasUnsavedWork)
        assertTrue(clean.copy(editState = EditState(dirty = true)).hasUnsavedWork)
    }
}

// ---------------------------------------------------------------------------
// NOT TESTED HERE, and worth fixing:
//
// setZoom / zoomBy / toggleZoom / zoomInOn hold the rule that has been patched
// four times ("Fix the page vanishing when a zoom gesture settles", "Fix the
// sudden jump when zooming", "Scope zoom to one page", "Make zoom actually
// work"). None of it is reachable from a JVM test, because it lives on an
// AndroidViewModel.
//
// The extraction that fixes this is small: move the decision to a pure function
// on the state itself, e.g.
//
//     fun PdfReaderState.withZoom(requested: Float, pinPage: Int? = null): PdfReaderState
//
// and let the ViewModel become `_state.update { it.withZoom(zoom, pinPage) }`.
// The pin latch, the clamp and the currentPage follow are then all testable
// here, in milliseconds, with no device involved.
// ---------------------------------------------------------------------------
