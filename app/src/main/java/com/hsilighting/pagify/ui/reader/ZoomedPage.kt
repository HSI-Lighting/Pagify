package com.hsilighting.pagify.ui.reader

import android.graphics.Bitmap
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import com.hsilighting.pagify.core.Annotation
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.CaptureTile
import com.hsilighting.pagify.core.PlacedPage
import com.hsilighting.pagify.core.SessionRecorder
import com.hsilighting.pagify.core.captureMaskFor
import com.hsilighting.pagify.core.captureTilesFor
import com.hsilighting.pagify.core.zoomedPageBounds
import com.hsilighting.pagify.core.PageMapping
import com.hsilighting.pagify.core.PageSize
import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.TextSegment
import com.hsilighting.pagify.core.RenderScale
import com.hsilighting.pagify.ui.components.ViewportWindow
import com.hsilighting.pagify.ui.components.annotationLayer
import com.hsilighting.pagify.ui.components.captureOverlay
import com.hsilighting.pagify.ui.components.doubleTapToZoom
import com.hsilighting.pagify.ui.components.pinchToZoom
import com.hsilighting.pagify.ui.components.twoFingerPanXY
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.animation.core.animate
import kotlinx.coroutines.launch
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlin.math.roundToInt

/**
 * A single page, magnified.
 *
 * Zooming deliberately leaves the continuous list behind and scopes the view to
 * one page: panning a magnified page should never wander into its neighbours,
 * which is disorienting and loses your place.
 *
 * ## Why this does its own transform instead of using scroll containers
 *
 * The position of the content is held here as an explicit [Offset], and applied
 * with `graphicsLayer`. That is what makes pinch anchoring *exact*: the new
 * offset is computed and applied in the same frame as the gesture event.
 *
 * The earlier attempt drove layout width from zoom and let two `ScrollState`s
 * position it. Anchoring then had to be deferred, because a scroll range does not
 * exist until the page has been re-measured — and a pinch fires dozens of events,
 * each restarting and cancelling the pending correction. The surviving one
 * applied a stale ratio against a scroll offset that had already moved, so the
 * zoom drifted away from the fingers. Nothing here waits for a frame.
 *
 * Sharpness is preserved separately: the page is *laid out* at `committedScale`,
 * and once a gesture settles that catches up to the live scale, so the page is
 * re-rasterised rather than left as an upscaled bitmap.
 */
@Composable
fun ZoomedPage(
    pageIndex: Int,
    initialZoom: Float,
    /** The page's own size, upright — which is the space marks are stored in. */
    pageSize: PageSize?,
    /** View rotation, clockwise. The page is laid out and drawn turned by this. */
    quarterTurns: Int = 0,
    /** Called when a gesture settles, with the scale to render and prefetch at. */
    onZoomSettled: (Float) -> Unit,
    /** Fired on every zoom gesture event, to drive the blank-frame watcher. */
    onZoomActivity: () -> Unit,
    onWindowChanged: (ViewportWindow) -> Unit,
    pageSizeProvider: suspend (Int) -> PageSize?,
    renderer: suspend (pageIndex: Int, zoom: Float) -> Bitmap?,
    /** Whatever the list last drew for this page; avoids a blank first frame. */
    initialBitmap: Bitmap?,
    /**
     * The width, in pixels, that the list draws this page at.
     *
     * This is what `scale = 1.0` has to mean, and getting it wrong is visible as a
     * jump. This view replaces the whole row including the thumbnail rail, so its
     * own viewport is *wider* than the reader area the page was just occupying —
     * measuring the base against the viewport made entering zoom silently enlarge
     * the page by the width of the rail and its gaps before any gesture applied.
     */
    basePageWidthPx: Float,
    /** Where to centre when the view opens, as a 0..1 fraction of the page. */
    initialFocus: Offset?,
    /**
     * Marks already on this page, and the state of the tool ribbon.
     *
     * Annotating has to work here too. This view is a separate render path from
     * the list, and it was built before the tools existed, so it drew the page
     * bitmap and nothing else: magnifying a page made its highlights disappear,
     * and the pen had no surface to draw on — every one-finger drag went to the
     * pan handler below instead.
     */
    annotations: List<Annotation>,
    /** Bumped whenever the marks change; see `annotationLayer`'s `revision`. */
    annotationRevision: Int,
    textSegments: List<TextSegment>,
    tool: AnnotationTool,
    /** How heavy the drawing tools are, and what kind of line they draw. */
    strokeWidth: Float,
    lineStyle: MarkupStyle,
    penColor: Long,
    onAddAnnotation: (Annotation) -> Unit,
    /** The Note tool was tapped at this page point. */
    onRequestNote: (pageIndex: Int, anchor: Offset) -> Unit,
    /**
     * The text tools, which this view had none of.
     *
     * Their absence is why nothing could be written, picked up or moved while
     * magnified: the layer was built here without them, so every tap and drag a
     * text tool made went nowhere. Zooming in to place a caption exactly is
     * precisely when you would want them.
     */
    onPlaceText: (pageIndex: Int, path: List<Offset>) -> Unit,
    onMoveText: (id: Long, delta: Offset) -> Unit,
    onSelectText: (id: Long?) -> Unit,
    /** The caption the ribbon is editing, drawn picked out. */
    selectedText: Long?,
    /** Two fingers with a caption in hand: that big. */
    onScaleText: (factor: Float) -> Unit,
    /** A caption was double-tapped; rewrite its words. */
    onEditText: (id: Long) -> Unit,
    /**
     * Move to the next page (+1) or the previous one (-1), staying magnified.
     *
     * @return true when there was a page to move to. False springs the pull back
     *   instead, which is what says "this is the end of the document".
     */
    onTurnPage: (delta: Int) -> Boolean,
    /** This page is on screen; load any marks the file already holds for it. */
    onPageMarksNeeded: (Int) -> Unit,
    onOpenNote: (com.hsilighting.pagify.core.Annotation.Note) -> Unit,
    onEraseStart: () -> Unit,
    onErase: (point: Offset, tolerancePoints: Float) -> Unit,
    onEraseEnd: () -> Unit,
    /** Whether the capture tool draws a ring instead of dragging a box. */
    captureLasso: Boolean,
    /** A box was dragged around part of the page; capture what it framed. */
    onCaptureViewport: (
        tiles: List<CaptureTile>,
        area: Rect,
        background: Long,
        originPage: Int,
        mask: List<Offset>,
    ) -> Unit,
    modifier: Modifier = Modifier,
) {
    BoxWithConstraints(modifier.fillMaxSize().clipToBounds()) {
        val density = LocalDensity.current
        val viewportW = with(density) { maxWidth.toPx() }
        val viewportH = with(density) { maxHeight.toPx() }

        // Everything about the box on screen — its shape, and the scale the raster
        // is asked for — is the *turned* page. Only the marks stay upright.
        val laidOut = pageSize?.turned(quarterTurns)
        val aspect = laidOut?.aspectRatio ?: DEFAULT_ASPECT
        // Scale 1.0 is exactly what the list was showing, so handing over is
        // pixel-identical and every zoom grows continuously from there.
        val baseW = if (basePageWidthPx > 0f) basePageWidthPx else viewportW
        val baseH = if (aspect > 0f) baseW / aspect else viewportH

        var scale by remember { mutableFloatStateOf(initialZoom) }
        var committedScale by remember { mutableFloatStateOf(initialZoom) }
        var offset by remember { mutableStateOf(Offset.Zero) }

        /**
         * How far the page has been pulled past its own end, in pixels.
         *
         * Signed the way the offset is: negative is pulled up past the bottom,
         * positive pulled down past the top. It gives a little and springs back,
         * which says "the page has run out" without a word on screen — and a
         * fresh swipe that pulls far enough turns to the next page.
         */
        var pull by remember { mutableFloatStateOf(0f) }

        /** Where to sit on a page just turned to, applied once its size is known. */
        var landing by remember { mutableStateOf<Int?>(null) }
        val scope = rememberCoroutineScope()

        /**
         * Keep the content covering the viewport, and centred on any axis where it
         * is smaller. Without this a pan could strand the page off screen.
         */
        fun clamp(candidate: Offset, atScale: Float): Offset {
            val contentW = baseW * atScale
            val contentH = baseH * atScale
            val x = if (contentW <= viewportW) {
                (viewportW - contentW) / 2f
            } else {
                candidate.x.coerceIn(viewportW - contentW, 0f)
            }
            val y = if (contentH <= viewportH) {
                (viewportH - contentH) / 2f
            } else {
                candidate.y.coerceIn(viewportH - contentH, 0f)
            }
            return Offset(x, y)
        }

        // A page just turned to sits at the edge the reader arrived from: the top
        // when moving forward, the bottom when moving back, and always at the same
        // horizontal position, so reading down one column carries on in the same
        // column of the next page.
        LaunchedEffect(pageIndex, baseH, landing) {
            val towards = landing ?: return@LaunchedEffect
            offset = clamp(
                Offset(offset.x, if (towards > 0) 0f else viewportH - baseH * scale),
                scale,
            )
        }

        // The marks on a page turned to have to be read, exactly as the list reads
        // them when a page scrolls into view. Without this a magnified page turned
        // to came up with none of its highlights.
        LaunchedEffect(pageIndex) { onPageMarksNeeded(pageIndex) }

        // Open centred on whatever the entering gesture was aimed at.
        LaunchedEffect(initialFocus, baseW, baseH) {
            val focus = initialFocus ?: return@LaunchedEffect
            offset = clamp(
                Offset(
                    viewportW / 2f - focus.x * baseW * scale,
                    viewportH / 2f - focus.y * baseH * scale,
                ),
                scale,
            )
        }

        /**
         * Let the pull go: turn the page if it went far enough, or spring back.
         *
         * The turn keeps the zoom and the horizontal position; only the page
         * changes. Nowhere to go — the first page or the last — springs back too,
         * which is what says the document has ended rather than the page.
         */
        fun settle() {
            val pulled = pull
            val towards = when {
                pulled <= -PULL_TO_TURN -> 1
                pulled >= PULL_TO_TURN -> -1
                else -> 0
            }
            if (towards != 0 && onTurnPage(towards)) {
                landing = towards
                pull = 0f
                return
            }
            if (pulled == 0f) return
            scope.launch {
                animate(initialValue = pulled, targetValue = 0f) { value, _ -> pull = value }
            }
        }

        /**
         * Scale about [focus], a point in viewport coordinates.
         *
         * The content point under the focus is `(focus - offset) / scale`. For it
         * to stay under the focus at the new scale, the offset must become
         * `focus - thatPoint * newScale`, which reduces to the expression below.
         */
        fun zoomAbout(factor: Float, focus: Offset) {
            val newScale = (scale * factor).coerceIn(
                PdfReaderState.MIN_ZOOM,
                PdfReaderState.MAX_ZOOM,
            )
            if (newScale == scale) return
            val ratio = newScale / scale
            offset = clamp(focus - (focus - offset) * ratio, newScale)
            scale = newScale
        }

        // Re-render at the settled scale, and let the rest of the app know. Held
        // back until the gesture stops so a pinch does not rasterise the page at
        // every intermediate size.
        LaunchedEffect(Unit) {
            snapshotFlow { scale }.collectLatest { settled ->
                delay(SETTLE_MILLIS)
                SessionRecorder.record("ZOOM_SETTLED", "scale=$settled")
                committedScale = settled
                onZoomSettled(settled)
            }
        }

        // The rasterised page. Kept here rather than in PdfPageView because the
        // zoomed view draws it itself.
        // Seeded synchronously from whatever the list last drew for this page, so
        // the very first composed frame already has pixels. Without this there is
        // a measurable window — 54 ms on this device — where the content area is
        // completely blank while the proxy render is still in flight.
        var pageBitmap by remember(pageIndex) { mutableStateOf(initialBitmap) }
        var bitmapScale by remember(pageIndex) { mutableStateOf(0f) }

        val renderScale = remember(laidOut, committedScale, baseW) {
            val size = laidOut ?: return@remember null
            RenderScale.forPage(size, baseW * committedScale)
        }

        // Something to show *immediately*, before the sharp render exists.
        //
        // Entering zoom composes this view fresh, with no bitmap, so the canvas
        // had nothing to draw and the page flashed blank — worst on exactly the
        // documents where the sharp render takes longest. The proxy is the same
        // cheap raster the list already drew, so it is almost always a cache hit
        // and appears at once; it is then replaced, never blanked.
        LaunchedEffect(pageIndex, laidOut) {
            val size = laidOut ?: return@LaunchedEffect
            if (pageBitmap != null) return@LaunchedEffect // already seeded or drawn
            val proxy = RenderScale.proxyFor(size, baseW)
            renderer(pageIndex, proxy)?.let {
                if (bitmapScale < proxy) {
                    pageBitmap = it
                    bitmapScale = proxy
                }
            }
        }

        LaunchedEffect(pageIndex, renderScale) {
            val target = renderScale ?: return@LaunchedEffect
            if (bitmapScale >= target) return@LaunchedEffect
            // Assigned only on success, so a failed or slow render leaves whatever
            // is on screen in place rather than clearing it.
            renderer(pageIndex, target)?.let {
                pageBitmap = it
                bitmapScale = target
            }
        }

        // Report the visible region for the navigator.
        LaunchedEffect(offset, scale, baseW, baseH, viewportW, viewportH) {
            val contentW = baseW * scale
            val contentH = baseH * scale
            onWindowChanged(
                if (contentW <= 0f || contentH <= 0f) {
                    ViewportWindow.Full
                } else {
                    val w = (viewportW / contentW).coerceIn(0f, 1f)
                    val h = (viewportH / contentH).coerceIn(0f, 1f)
                    ViewportWindow(
                        left = (-offset.x / contentW).coerceIn(0f, 1f - w),
                        top = (-offset.y / contentH).coerceIn(0f, 1f - h),
                        width = w,
                        height = h,
                    )
                },
            )
        }

        // Pixels per page point at the size the canvas is currently drawing, which
        // is the live scale rather than the committed one: the bitmap may still be
        // the previous rasterisation stretched to fit, and a mark has to sit on the
        // text as it appears now, not as it will appear once the render catches up.
        val shown = Offset(offset.x, offset.y + pull)
        val mapping = laidOut
            ?.takeIf { it.widthPoints > 0f }
            ?.let {
                PageMapping(
                    scale = baseW * scale / it.widthPoints,
                    origin = shown,
                    quarterTurns = quarterTurns,
                    pageWidthPoints = pageSize?.widthPoints ?: 0f,
                    pageHeightPoints = pageSize?.heightPoints ?: 0f,
                )
            }
            ?: PageMapping.Unmeasured

        // With a tool live one finger belongs to the tool, exactly as in the list.
        // Leaving the pan on one finger is what made the pen appear disabled here:
        // `detectDragGestures` consumed the drag before the drawing layer saw it.
        val toolActive = tool != AnnotationTool.None

        // Read where the theme is in scope: what shows between or beside pages in
        // a capture is a reader decision, not the engine's.
        val captureBackground = MaterialTheme.colorScheme.surfaceVariant
            .toArgb()
            .toLong() and 0xFFFFFFFFL

        Box(
            Modifier
                .fillMaxSize()
                .pinchToZoom { factor, centroid ->
                    // A caption in hand takes the pinch here too: magnified is
                    // exactly where you would size one carefully.
                    if (selectedText != null) {
                        onScaleText(factor)
                        return@pinchToZoom
                    }
                    onZoomActivity()
                    zoomAbout(factor, centroid)
                }
                // Two fingers always pan, for the same reason as in the list: the
                // pinch handler claims every two-finger event, so nothing else
                // would ever receive one.
                // As with the zoom: a caption in hand takes the whole gesture,
                // so the page does not slide out from under it.
                .twoFingerPanXY { drag ->
                    if (selectedText == null) offset = clamp(offset + drag, scale)
                }
                // One finger pans only when it is not busy annotating.
                .then(
                    if (toolActive) {
                        Modifier
                    } else {
                        Modifier.pointerInput(Unit) {
                            detectDragGestures(
                                // A fresh swipe turns the page, not a long drag
                                // that runs off the end: the drag stops dead at
                                // the edge, and lifting is what decides. You
                                // cannot shoot through three pages by flicking.
                                onDragEnd = { settle() },
                                onDragCancel = { settle() },
                            ) { change, drag ->
                                change.consume()
                                // The reader is moving under their own hand now,
                                // so wherever the turn put them is where they are.
                                landing = null
                                val wanted = offset + drag
                                val held = clamp(wanted, scale)
                                offset = held
                                // Whatever the clamp refused vertically is the
                                // page having run out. It gives, dampened, up to
                                // a limit, and springs back when the finger
                                // lifts unless it went far enough to turn.
                                val refused = wanted.y - held.y
                                if (refused != 0f) {
                                    pull = (pull + refused * PULL_DAMPING)
                                        .coerceIn(-PULL_LIMIT, PULL_LIMIT)
                                }
                            }
                        }
                    },
                )
                // Watched rather than detected, for the same reason as the list:
                // the annotation surface below handles taps and consumes them, so
                // a detector here is starved. See `doubleTapToZoom`.
                .doubleTapToZoom { position ->
                    SessionRecorder.record("ZOOM_DTAP_PINNED", "scale=$scale")
                    onZoomActivity()
                    // Back to fit-width, about the tapped point. Reported
                    // immediately rather than after the settle delay so the
                    // pinned page is released without a visible lag.
                    if (scale > PdfReaderState.FIT_WIDTH_ZOOM + 0.01f) {
                        zoomAbout(PdfReaderState.FIT_WIDTH_ZOOM / scale, position)
                    } else {
                        zoomAbout(DOUBLE_TAP_ZOOM / scale, position)
                    }
                    committedScale = scale
                    onZoomSettled(scale)
                }
                // Capture is dragged here too, not only in the list. The zoomed
                // view is a separate composable, and leaving it out is what made
                // the snapshot tool do nothing the moment a page was zoomed into.
                .then(
                    if (tool == AnnotationTool.Snapshot && pageSize != null) {
                        Modifier.captureOverlay(lasso = captureLasso) { box, ring ->
                            // The page is drawn translated by `offset` at `scale`,
                            // in this element's own pixels — the same frame the drag
                            // is reported in, so nothing needs converting.
                            val onScreen = zoomedPageBounds(shown, baseW, baseH, scale)
                            val tiles = captureTilesFor(
                                box,
                                listOf(PlacedPage(pageIndex, onScreen, pageSize)),
                            )
                            SessionRecorder.record(
                                kind = "CAPTURE_BOX",
                                detail = "zoomed page=$pageIndex " +
                                    "box=${box.left.toInt()},${box.top.toInt()}.." +
                                    "${box.right.toInt()},${box.bottom.toInt()} " +
                                    "page=${onScreen.left.toInt()},${onScreen.top.toInt()}.." +
                                    "${onScreen.right.toInt()},${onScreen.bottom.toInt()} " +
                                    "tiles=${tiles.size}",
                            )
                            // No translation here: the ring, the box and the
                            // page rectangle are all in this element's own
                            // pixels already.
                            onCaptureViewport(
                                tiles,
                                box,
                                captureBackground,
                                pageIndex,
                                captureMaskFor(box, ring),
                            )
                        }
                    } else {
                        Modifier
                    },
                )
                // Last in the chain, so it is the innermost input receiver and a
                // one-finger drag reaches the tool before anything else can claim
                // it — and the innermost draw, so marks land over the page.
                .annotationLayer(
                    pageIndex = pageIndex,
                    annotations = annotations,
                    revision = annotationRevision,
                    textSegments = textSegments,
                    tool = tool,
                    strokeWidth = strokeWidth,
                    lineStyle = lineStyle,
                    penColor = penColor,
                    // Carries the translation too: the page is drawn offset, so the
                    // layer has to be told where its top-left corner actually is.
                    mapping = mapping,
                    onAdd = onAddAnnotation,
                    onRequestNote = { anchor -> onRequestNote(pageIndex, anchor) },
                    onPlaceText = { path -> onPlaceText(pageIndex, path) },
                    onMoveText = onMoveText,
                    onSelectText = onSelectText,
                    selectedText = selectedText,
                    onEditText = onEditText,
                    onOpenNote = onOpenNote,
                    onEraseStart = onEraseStart,
                    onErase = onErase,
                    onEraseEnd = onEraseEnd,
                ),
        ) {
            // The page is *drawn*, not laid out, at its magnified size. Laying out
            // a composable at `baseW * scale` meant a 4x zoom asked for a
            // ~6400x9100 px layer, past the GPU's maximum texture size, and the
            // layer silently failed — the page vanished the instant a gesture
            // settled and the layout caught up to the scale. Here the canvas is
            // always viewport-sized and only the destination rectangle grows, so
            // there is no size that can break it.
            Canvas(Modifier.fillMaxSize()) {
                val bmp = pageBitmap ?: return@Canvas
                drawImage(
                    image = bmp.asImageBitmap(),
                    dstOffset = IntOffset(shown.x.roundToInt(), shown.y.roundToInt()),
                    dstSize = IntSize(
                        (baseW * scale).roundToInt().coerceAtLeast(1),
                        (baseH * scale).roundToInt().coerceAtLeast(1),
                    ),
                    // Bilinear, so the stretch between a settle and the next
                    // re-rasterisation is smooth rather than blocky.
                    filterQuality = FilterQuality.Medium,
                )
            }
        }
    }
}

private const val DEFAULT_ASPECT = 595f / 842f
private const val DOUBLE_TAP_ZOOM = 2.5f

/** How long after the last gesture event to re-rasterise at the new scale. */
private const val SETTLE_MILLIS = 180L

/**
 * How much of a drag past the page's end actually moves it.
 *
 * Less than all of it, so the edge feels like an edge: the page follows the
 * finger at half speed once it has run out, which is what tells you it has.
 */
private const val PULL_DAMPING = 0.45f

/** As far as the page will give, however hard it is pulled. */
private const val PULL_LIMIT = 240f

/**
 * How far it has to be pulled for a lift to turn the page.
 *
 * Comfortably short of [PULL_LIMIT], so the page turns before the pull runs out
 * of room and the gesture stops meaning anything.
 */
private const val PULL_TO_TURN = 110f
