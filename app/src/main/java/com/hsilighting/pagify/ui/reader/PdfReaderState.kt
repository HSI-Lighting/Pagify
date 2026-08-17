package com.hsilighting.pagify.ui.reader

import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PdfMetadata

/**
 * Everything the reader screen draws from. One immutable snapshot per emission,
 * so a recomposition can never observe a half-applied update.
 */
data class PdfReaderState(
    val phase: Phase = Phase.Empty,
    val documentName: String = "",
    val metadata: PdfMetadata? = null,
    val pageCount: Int = 0,
    /** Zero-based index of the page the user is looking at. */
    val currentPage: Int = 0,
    val zoom: Float = 1f,
    /**
     * The page zoom is locked to, or null when at fit-width.
     *
     * Zooming scopes the view to a single page rather than magnifying the whole
     * document: panning around a magnified page should never wander into its
     * neighbours, which is disorienting at high zoom and makes it easy to lose the
     * page you were reading. Set when zoom first rises above fit-width, cleared
     * when it returns.
     */
    val zoomedPage: Int? = null,
    val rotationQuarterTurns: Int = 0,
    /** Natural sizes, indexed by page. Empty entries have not been measured yet. */
    val pageSizes: Map<Int, PageSize> = emptyMap(),
    val showMetadataSheet: Boolean = false,
) {
    val isReady: Boolean get() = phase is Phase.Ready
    /** 1-based, for display. */
    val currentPageLabel: Int get() = (currentPage + 1).coerceAtMost(pageCount.coerceAtLeast(1))

    sealed interface Phase {
        /** No document chosen yet. */
        data object Empty : Phase

        data object Loading : Phase

        data object Ready : Phase

        /** The document is encrypted; [retry] marks a rejected attempt. */
        data class PasswordRequired(val retry: Boolean) : Phase

        data class Failed(val message: String) : Phase
    }

    companion object {
        /** Page exactly fills the viewport width. Below this, zoom is released. */
        const val FIT_WIDTH_ZOOM = 1f
        const val MIN_ZOOM = 1f
        const val MAX_ZOOM = 8f

        /**
         * How many pages either side of the visible one to pre-render.
         *
         * One is deliberate: it covers a swipe in either direction, while a larger
         * window would rasterise pages the user will likely never see and evict
         * the ones they are actually reading.
         */
        const val PREFETCH_RADIUS = 1
    }
}
