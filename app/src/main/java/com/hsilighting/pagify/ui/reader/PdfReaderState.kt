package com.hsilighting.pagify.ui.reader

import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.PdfMetadata
import com.hsilighting.pagify.core.PenMode

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
    /** The thumbnail rail. On by default: it is the cheap way to move around. */
    val showThumbnails: Boolean = true,
    /** A render-timeline recording is in progress. See `SessionRecorder`. */
    val isRecording: Boolean = false,

    // ------------------------------------------------------------ annotation --
    val tool: AnnotationTool = AnnotationTool.None,
    val penMode: PenMode = PenMode.Highlight,
    val penColor: Long = AnnotationColors.YELLOW,
    /**
     * Bumped whenever the annotations change at all.
     *
     * The store itself is mutable and identity-stable, so Compose cannot see
     * changes inside it; this counter is what makes a new mark actually redraw.
     */
    val annotationRevision: Int = 0,
    val canUndo: Boolean = false,
    val canRedo: Boolean = false,
    /**
     * Pages checked for a text layer and found to have none.
     *
     * A scan has no text objects, so the highlighter has nothing to select and
     * quietly produces nothing. Knowing which pages those are is what lets the UI
     * say so instead of leaving the tool looking broken.
     */
    val pagesWithoutSelectableText: Set<Int> = emptySet(),
    /** Marks on [currentPage], for the wording of the clear-page action. */
    val annotationsOnPage: Int = 0,
    /** Marks anywhere in the document, for the clear-all action. */
    val annotationsInDocument: Int = 0,
    /**
     * A page the reader has been asked to scroll to and has not yet.
     *
     * Undo and redo set this when the edit they reversed belongs to a page you
     * are not looking at: the change would otherwise happen off screen, which is
     * indistinguishable from the button doing nothing. The reader clears it once
     * it has scrolled, so the same page can be asked for twice in a row.
     */
    val jumpToPage: Int? = null,
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
