package com.hsilighting.pagify.ui.reader

import android.net.Uri
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.ImageBitmap
import com.hsilighting.pagify.core.AnnotationColors
import com.hsilighting.pagify.core.CaptureFill
import com.hsilighting.pagify.core.CaptureRequest
import com.hsilighting.pagify.core.Markup
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.MAXIMUM_TEXT_POINTS
import com.hsilighting.pagify.core.PdfFont
import com.hsilighting.pagify.core.TextFrame
import com.hsilighting.pagify.core.MarkupTool
import com.hsilighting.pagify.core.defaultMarkupSizes
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.EditState
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
    /** The thumbnail rail. On by default: it is the cheap way to move around. */
    val showThumbnails: Boolean = true,
    /**
     * The capture tool draws a ring instead of dragging a box.
     *
     * Sticky, like the pen colour: someone lifting several details off one
     * drawing should not have to re-choose the shape between each.
     */
    val captureLasso: Boolean = false,
    /**
     * What fills a capture where no page reaches.
     *
     * Sticky like the shape and the scale: someone lifting several
     * details onto the same slide wants the same fill behind each.
     */
    val captureFill: CaptureFill = CaptureFill.PAGE,
    /** A render-timeline recording is in progress. See `SessionRecorder`. */
    val isRecording: Boolean = false,

    // -------------------------------------------------------- document edits --
    /**
     * Undo depth, dirtiness and page count for edits to the *document* — pages
     * deleted, moved, rotated or inserted.
     *
     * Deliberately separate from [canUndo] and [canRedo] below, which are the
     * annotation history. They are different models with different lifetimes:
     * annotations live in an `AnnotationStore` and are not yet written into the
     * file, while these go through the engine's `Command` path and change the
     * document itself. A single shared undo stack would make the button's meaning
     * depend on which of the two the user last touched.
     */
    val editState: EditState = EditState(),
    /**
     * Bumped whenever the document changes under the pages on screen.
     *
     * A page render is skipped once it is already drawn at that scale. That is
     * what keeps scrolling cheap, and it is also what left an erased mark on
     * screen: PDFium draws saved annotations as part of the page, so the file had
     * changed and nothing asked for the pixels again.
     */
    val pageContentRevision: Int = 0,
    /**
     * Bumped on every open, so per-page work keyed on it runs again for a
     * different document.
     *
     * Saving reopens the file, and a page that keeps its index across that reopen
     * keeps its composable too — so a `LaunchedEffect(pageIndex)` never fires
     * again. That is what stopped saved marks being loaded after a save: the
     * eraser then found nothing to erase, on a page that visibly had a mark.
     */
    val documentRevision: Int = 0,
    /** The page-organisation sheet: reorder, rotate, delete, insert. */
    val showPageOrganiser: Boolean = false,
    /** Where a new sheet would go, while the reader is being asked about it. */
    val blankPageAfter: Int? = null,
    /**
     * The library's + has been pressed and the two things it can mean are being
     * offered: paper of their own, or a file they already have.
     */
    val showNewDocumentChooser: Boolean = false,
    /** They chose paper, and are describing it. */
    val showNewDocumentSheet: Boolean = false,
    /**
     * Somewhere the reader is trying to go, while they are being asked about
     * unsaved work. Null when nothing is being asked.
     */
    val pendingLeave: LeaveIntent? = null,
    /**
     * Somewhere they have settled on going. The UI acts on it once and clears it.
     *
     * Separate from [pendingLeave] because leaving is the UI's job — one of the
     * two destinations is a system file picker, which no view model can open.
     */
    val leaveNow: LeaveIntent? = null,
    /** A save is running. Blocks a second one and drives the progress indicator. */
    val isSaving: Boolean = false,
    /** One-shot message for the snackbar. Cleared by `messageShown`. */
    val message: String? = null,

    // ------------------------------------------------------------ annotation --
    val tool: AnnotationTool = AnnotationTool.None,
    val penColor: Long = AnnotationColors.YELLOW,
    /**
     * How heavy the drawing tools draw, in page points.
     *
     * One width for all of them rather than one each: a drawing marked up
     * with a fine pen wants fine boxes round it too, and five separate
     * settings would be four surprises.
     */
    val annotationStrokeWidth: Float = 2.4f,
    /** Solid, dashed or a centre line, for everything that draws a line. */
    val annotationStyle: MarkupStyle = MarkupStyle.SOLID,
    /**
     * What text is written in, and how big.
     *
     * Sticky like the colour and the nib: somebody labelling six details on a
     * drawing wants the sixth to match the first, and being asked again each time
     * is being asked a question already answered.
     */
    val textFont: PdfFont = PdfFont.HELVETICA,
    /**
     * How far a curved caption bends from end to end, in degrees.
     *
     * A setting rather than something drawn: 0 is a straight line, positive
     * arches upward, negative sags.
     */
    val textCurveDegrees: Float = 60f,
    /**
     * The caption the ribbon is currently editing, if one is picked up.
     *
     * Null means the controls set what the next caption will look like. With one
     * selected they do both: the caption changes, and the next one inherits it.
     */
    val selectedTextId: Long? = null,
    /**
     * The largest size the caption in hand can take and still fit the page.
     *
     * The ceiling is the sheet rather than a number: type is only as big as
     * useful while the words still fit across the page they are on. Falls back to
     * the backstop when nothing is selected.
     */
    val textSizeCeiling: Float = MAXIMUM_TEXT_POINTS,
    /**
     * Whether the bend still means anything for the caption in hand.
     *
     * False once it has more than one line: a block does not bend, so a slot
     * offering to bend it would be a control that does nothing.
     */
    val textBendApplies: Boolean = true,
    val textSizePoints: Float = 12f,
    /**
     * Where text is being typed, if it is.
     *
     * Holds the baseline it will sit on — two points for a tap, a traced curve for
     * the curved tool — so the same composer serves both and neither has to know
     * which it is.
     */
    val textBeingWritten: PendingText? = null,
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
     * Pages with no text the highlighter can select — after recognition has also
     * been tried and come back with nothing.
     *
     * A page can have no text objects at all: the words may be vector artwork, or
     * an image. The highlighter then has nothing to select and quietly produces
     * nothing, which is what lets the UI say so instead of leaving the tool
     * looking broken.
     */
    val pagesWithoutSelectableText: Set<Int> = emptySet(),
    /** Pages currently being read by [com.hsilighting.pagify.core.PageTextRecogniser]. */
    val pagesBeingRecognised: Set<Int> = emptySet(),
    /**
     * Where a note is being typed, if one is.
     *
     * Held here rather than in the layer that captured the tap because the text
     * is typed in a dialog: the page is gone from under the keyboard by then, and
     * a note placed against a composable that has since scrolled away would land
     * wherever the reader has got to instead of where it was tapped.
     */
    val pendingNote: PendingNote? = null,
    /**
     * A note the reader has tapped open.
     *
     * Without this the text a note holds could be typed and never read again: the
     * page only ever drew a marker, and nothing anywhere touched `Note.text`. The
     * tool looked like it added a dot and nothing else.
     */
    val openNote: com.hsilighting.pagify.core.Annotation.Note? = null,
    /**
     * Pages whose text was recognised rather than found in the file.
     *
     * Kept apart from ordinary text because it is not the same thing: recognition
     * misreads characters, and a user who selects something slightly wrong is owed
     * an explanation. It is also what a future "save the recognised layer into the
     * PDF" step would work from.
     */
    val pagesRecognised: Set<Int> = emptySet(),
    /** Marks on [currentPage], for the wording of the clear-page action. */
    val annotationsOnPage: Int = 0,
    /** Marks anywhere in the document, for the clear-all action. */
    val annotationsInDocument: Int = 0,
    /** Of those, the ones not yet written into the file. Drives the Save button. */
    val unsavedMarkCount: Int = 0,
    /**
     * A page the reader has been asked to scroll to and has not yet.
     *
     * Undo and redo set this when the edit they reversed belongs to a page you
     * are not looking at: the change would otherwise happen off screen, which is
     * indistinguishable from the button doing nothing. The reader clears it once
     * it has scrolled, so the same page can be asked for twice in a row.
     */
    val jumpToPage: Int? = null,

    // ------------------------------------------------------------- selection --
    /**
     * The text currently selected, if any.
     *
     * One selection at a time, and it belongs to a page: selecting across a page
     * boundary would mean a range over two character lists, and the reader has no
     * way to show a handle on a page that has scrolled away.
     */
    val selection: PageTextSelection? = null,

    // --------------------------------------------------------------- capture --
    /** A capture is being rendered. Blocks a second one and drives the spinner. */
    val isCapturing: Boolean = false,
    /**
     * The capture taken and not yet dismissed.
     *
     * Held rather than exported straight away: what to do with it — keep, send,
     * paste — is decided after seeing it, and the scale and format can still be
     * changed, which re-renders from the same crop.
     */
    val capture: CapturePreview? = null,
    /**
     * A capture ready to hand to another app. One-shot; the reader launches the
     * share sheet and calls `captureShared`.
     *
     * Minted here rather than in the screen because writing the file is I/O, and a
     * share sheet that appears before the bytes are on disk hands the receiving
     * app an empty file.
     */
    val captureToShare: CaptureShare? = null,
    /**
     * Marks drawn on the capture that is on screen, in page points.
     *
     * Cleared with the capture. They live on the picture and are deliberately not
     * written into the PDF: an annotation in the document is a different thing
     * with a different lifetime, and conflating the two would drag the whole write
     * path into a feature that produces a PNG.
     */
    val markup: List<Markup> = emptyList(),
    val markupTool: MarkupTool = MarkupTool.Pen,
    /**
     * Whether the markup tool is actually held.
     *
     * Separate from [markupTool] rather than making that nullable, so putting a
     * tool down and picking it back up returns the one you had.
     *
     * The editor used to draw with one finger unconditionally — a tool was always
     * held, on the reasoning that a finger on the picture had nothing else it
     * could mean. It does: while pinching to zoom, a finger that lands a moment
     * before or after its partner is one finger, and every one of those left a
     * mark on the picture. The reader has always been able to put its tools down;
     * this is the same escape.
     */
    val markupArmed: Boolean = true,
    /**
     * Solid, dashed or dash-dot, for the line tool.
     *
     * Sticky like the colour: a drawing marked up with dashed setting-out
     * lines wants the next one dashed too.
     */
    val markupStyle: MarkupStyle = MarkupStyle.SOLID,
    val markupColor: Long = AnnotationColors.RED,
    /**
     * How heavy each tool draws — nib width for the ones that draw a line, wash
     * intensity for the highlighter.
     *
     * Per tool, so setting a fine pen survives a trip through the highlighter and
     * back. Kept on the reader rather than inside the editor because it should
     * also survive the editor being closed and another capture taken.
     */
    val markupSizes: Map<MarkupTool, Float> = defaultMarkupSizes(),

    /** The caption on the capture the ribbon is editing, if one is picked up. */
    val selectedMarkupIndex: Int? = null,
) {
    val isReady: Boolean get() = phase is Phase.Ready
    /** 1-based, for display. */
    /**
     * The page a magnified swipe past the end would land on, or null for none.
     *
     * Null both when nothing is magnified — there is no pinned page to move from
     * — and at the two ends of the document, where the pull springs back instead.
     * Kept here rather than in the view model because it is a decision about a
     * snapshot, and this is the half of the reader a test can reach.
     */
    fun pageAfterTurn(delta: Int): Int? {
        val from = zoomedPage ?: return null
        val to = from + delta
        return to.takeIf { it in 0 until pageCount }
    }

    /**
     * Whether anything would be lost by walking away.
     *
     * Both halves count: pages moved or deleted, and marks made since the last
     * save. Either on its own is work somebody did.
     */
    val hasUnsavedWork: Boolean get() = editState.dirty || unsavedMarkCount > 0

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

    /**
     * A page's size as the reader currently lays it out — turned, if the view is.
     *
     * Distinct from [pageSizes], which stays the document's own: marks, text runs
     * and everything the engine is asked for are in upright page points, and a map
     * that quietly turned underneath them would put a mark a quarter turn from
     * where it belongs. Layout asks this; geometry asks [pageSizes].
     */
    fun displaySize(pageIndex: Int): PageSize? =
        pageSizes[pageIndex]?.turned(rotationQuarterTurns)

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

/**
 * A note the reader has asked for and not yet typed.
 *
 * Carries the page as well as the point: a note is anchored in page space, and
 * the reader may well have scrolled between the tap and the keyboard appearing.
 */
data class PendingNote(val pageIndex: Int, val anchor: Offset)

/**
 * Text the reader has selected on a page.
 *
 * The range is over that page's characters, and everything else is derived from
 * it — the rects to paint and the text to copy. Kept together so the three can
 * never disagree: a selection whose highlight says one thing and whose clipboard
 * says another is worse than no selection at all.
 */
data class PageTextSelection(
    val pageIndex: Int,
    val range: IntRange,
    val rects: List<Rect>,
    val text: String,
) {
    /** Where the handles go: the start of the first band and the end of the last. */
    val startHandle: Offset get() = rects.firstOrNull()?.let { Offset(it.left, it.bottom) } ?: Offset.Zero
    val endHandle: Offset get() = rects.lastOrNull()?.let { Offset(it.right, it.bottom) } ?: Offset.Zero
}

/**
 * A capture the reader has taken, with what it took and what it looks like.
 *
 * Not a data class on purpose: it carries a [ByteArray], whose `equals` compares
 * identity anyway, so a generated `equals` would be a lie about a deep comparison
 * it does not perform. Every capture is a fresh instance, which is exactly what
 * state comparison needs.
 */
class CapturePreview(
    val request: CaptureRequest,
    /** The encoded image — what gets saved, shared or pasted, byte for byte. */
    val bytes: ByteArray,
    val fileName: String,
    /** A downscaled copy for the preview. Never what is exported. */
    val preview: ImageBitmap,
) {
    val sizeLabel: String
        get() = when {
            bytes.size >= 1024 * 1024 -> "%.1f MB".format(bytes.size / (1024f * 1024f))
            else -> "${(bytes.size + 1023) / 1024} KB"
        }
}

/** A capture written to the cache and ready to leave the app. */
data class CaptureShare(val uri: Uri, val mimeType: String)

/**
 * Text that has been placed but not yet typed.
 *
 * The baseline is decided first — by a tap for straight text, by a traced curve
 * for curved — and the words come after. Keeping the two apart is what lets one
 * composer serve both: by the time anybody is typing, the difference between a
 * line and a curve is just how many points are in [path].
 */
data class PendingText(
    val pageIndex: Int,
    val path: List<Offset>,
    /**
     * What will be drawn around the words, if anything.
     *
     * Taken when the baseline was placed, not when the words come back: the
     * dialog is open across a keyboard, and a tool changed behind it must not
     * change what was already begun.
     */
    val frame: TextFrame = TextFrame.None,
    /** Whether the words run along a bent line. Taken with the frame, and why. */
    val bends: Boolean = false,
    /**
     * The caption being rewritten, when this is an edit rather than a new one.
     *
     * Null for a fresh caption. With an id the words replace that caption's, and
     * clearing them removes it — emptying it is the plainest way to say "delete".
     */
    val editing: Long? = null,
    /** The words already there, so the dialog opens on them. */
    val initial: String = "",
)

/**
 * Where the reader was heading when they were asked about unsaved work.
 *
 * Held so the answer can be acted on afterwards: "don't save" has to know what
 * it was you were doing before the question interrupted you.
 */
enum class LeaveIntent {
    /** Back to the document library. */
    Library,

    /** Opening a different document. */
    AnotherDocument,
}
