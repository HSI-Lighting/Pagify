package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.draw.drawWithContent
import com.hsilighting.pagify.core.Annotation
import com.hsilighting.pagify.core.AnnotationTool
import com.hsilighting.pagify.core.NOTE_MARKER_RADIUS_POINTS
import com.hsilighting.pagify.core.PageMapping
import com.hsilighting.pagify.core.isHitBy


import com.hsilighting.pagify.core.MarkupStyle
import com.hsilighting.pagify.core.cloudOutline
import com.hsilighting.pagify.core.dashed
import com.hsilighting.pagify.core.isDragged
import com.hsilighting.pagify.core.tracesPath
import com.hsilighting.pagify.core.shapeStrokes
import com.hsilighting.pagify.core.SessionRecorder
import com.hsilighting.pagify.core.TextSegment
import com.hsilighting.pagify.core.TextSelection
import com.hsilighting.pagify.ui.reader.PageTextSelection

/**
 * Draws a page's annotations and, when a tool is active, captures the input that
 * creates new ones.
 *
 * Everything here works in **page points**, converting to pixels only at draw
 * time via [mapping]. Storing a stroke in screen pixels would freeze it to the
 * zoom level it was drawn at, and it would drift the moment the page was
 * re-rendered at another size — or turned.
 *
 * When no tool is selected this adds no pointer input at all, so scrolling and
 * zooming behave exactly as before — an always-on input layer would swallow the
 * gestures the reader depends on.
 */
@Composable
fun Modifier.annotationLayer(
    pageIndex: Int,
    annotations: List<Annotation>,
    /**
     * Bumped whenever the marks change at all.
     *
     * Captured by the draw lambda below, which is the only reason it is here: a
     * lambda whose captures are unchanged is reused, the draw node is never
     * updated, and the page keeps painting the picture it painted last time. That
     * is not a theory — undo took a mark out of the store, the page recomposed
     * with a shorter list, and the mark stayed on screen until a zoom happened to
     * force a redraw. An `Int` that differs is what makes this a different lambda.
     */
    revision: Int,
    textSegments: List<TextSegment>,
    tool: AnnotationTool,
    /** How heavy the drawing tools are set, in page points. */
    strokeWidth: Float,
    /** Solid, dashed or a centre line — baked into the strokes on commit. */
    lineStyle: MarkupStyle,
    penColor: Long,
    /**
     * How a page point becomes a pixel here: the scale, the page's corner, and
     * the view rotation.
     *
     * The corner is zero in the list, where the layer is applied straight to the
     * page. The magnified view draws a translated page into a viewport-sized
     * canvas, so without it the layer would map page points as though the page
     * still began at the corner — marks would land in the wrong place and a touch
     * would be read as pointing somewhere else entirely.
     */
    mapping: PageMapping,
    onAdd: (Annotation) -> Unit,
    onRequestNote: (Offset) -> Unit,
    /** A note marker was tapped; show what it says. */
    onOpenNote: (Annotation.Note) -> Unit = {},
    /** Opens an eraser stroke; everything it takes until [onEraseEnd] is one undo. */
    onEraseStart: () -> Unit = {},
    /** Rub out whatever is at this page point, within this tolerance in points. */
    onErase: (point: Offset, tolerancePoints: Float) -> Unit = { _, _ -> },
    onEraseEnd: () -> Unit = {},
    /**
     * A highlight drag that swept the page and selected nothing.
     *
     * The honest signal that the highlighter cannot work here, and better than
     * counting runs: a scan often carries one stray run — a watermark, a page
     * number — so "no text at all" misses exactly the pages that look like they
     * should work and do not.
     */
    onHighlightMissed: () -> Unit = {},
    /**
     * The text selected on this page, if any.
     *
     * Drawn here rather than in a layer of its own because it has to sit exactly
     * where the text does, and this is the one place that already knows the page
     * scale and where the page's corner is on screen.
     */
    selection: PageTextSelection? = null,
    /** A long press landed here; select the word under it. */
    onSelectWord: (Offset) -> Unit = {},
    /** A selection handle was dragged. */
    onMoveSelectionHandle: (isStart: Boolean, point: Offset) -> Unit = { _, _ -> },
    onClearSelection: () -> Unit = {},
): Modifier {
    // Read through `rememberUpdatedState`: the pointerInput block below is keyed
    // on the tool, so without this it would capture the colour and mode that were
    // current when the gesture handler started rather than the latest ones.
    android.util.Log.i("AnnotationLayer", "compose page=$pageIndex marks=${annotations.size} tool=$tool")
    val currentColor by rememberUpdatedState(penColor)
    val currentStyle by rememberUpdatedState(lineStyle)
    val currentWidth by rememberUpdatedState(strokeWidth)
    val currentSegments by rememberUpdatedState(textSegments)
    val add by rememberUpdatedState(onAdd)
    val requestNote by rememberUpdatedState(onRequestNote)
    val openNote by rememberUpdatedState(onOpenNote)
    val currentAnnotations by rememberUpdatedState(annotations)
    val at by rememberUpdatedState(mapping)
    val eraseStart by rememberUpdatedState(onEraseStart)
    val erase by rememberUpdatedState(onErase)
    val eraseEnd by rememberUpdatedState(onEraseEnd)
    val highlightMissed by rememberUpdatedState(onHighlightMissed)
    val currentSelection by rememberUpdatedState(selection)
    val selectWord by rememberUpdatedState(onSelectWord)
    val moveHandle by rememberUpdatedState(onMoveSelectionHandle)
    val clearSelection by rememberUpdatedState(onClearSelection)

    /** Live stroke, in page points, while a marker drag is in progress. */
    var wetStroke by remember { mutableStateOf<List<Offset>>(emptyList()) }
    /** Live highlight rects while a highlight drag is in progress. */
    var wetHighlight by remember { mutableStateOf<List<Rect>>(emptyList()) }
    /** Live shape, in page points, while a line or box is being dragged. */
    var wetShape by remember { mutableStateOf<List<List<Offset>>>(emptyList()) }
    /** Where the eraser is, in page points, while it is down. Drawn as a ring. */
    var eraserAt by remember { mutableStateOf<Offset?>(null) }

    /**
     * How many marks the last draw actually painted.
     *
     * A plain array, not state: it is written from inside the draw pass, and a
     * snapshot write there would schedule another frame to record the frame that
     * just happened.
     */
    val painted = remember { intArrayOf(-1) }

    fun toPage(position: Offset): Offset = at.toPage(position)

    // A fixed touch radius in dp, converted to page points through the current
    // scale. Expressing the eraser's reach in points instead would make it a
    // pinhead when zoomed in and a paint roller when zoomed out.
    val eraserRadiusPx = with(LocalDensity.current) { ERASER_TOUCH_RADIUS.toPx() }
    fun tolerancePoints(): Float = at.toPage(eraserRadiusPx)

    val inputModifier = when {
        // Text only. Drag across words and it snaps to the lines they sit on.
        tool == AnnotationTool.Highlight -> Modifier.pointerInput(pageIndex, tool) {
            var start = Offset.Zero
            /** Distinguishes a sweep that found nothing from a stray tap. */
            var swept = false
            detectDragGestures(
                onDragStart = { position ->
                    start = toPage(position)
                    swept = false
                    wetHighlight = emptyList()
                },
                onDrag = { change, _ ->
                    change.consume()
                    swept = true
                    wetHighlight =
                        TextSelection.rectsBetween(currentSegments, start, toPage(change.position))
                },
                onDragEnd = {
                    if (wetHighlight.isNotEmpty()) {
                        // The span the rects cover is recorded alongside their
                        // count, because a count on its own cannot distinguish
                        // "selected a paragraph" from "selected the page" —
                        // which is what the previous selection actually did.
                        SessionRecorder.record(
                            kind = "HIGHLIGHT_SELECT",
                            detail = "page=$pageIndex rects=${wetHighlight.size} " +
                                "x=${wetHighlight.minOf { it.left }.toInt()}.." +
                                "${wetHighlight.maxOf { it.right }.toInt()} " +
                                "y=${wetHighlight.minOf { it.top }.toInt()}.." +
                                "${wetHighlight.maxOf { it.bottom }.toInt()} " +
                                "runsOnPage=${currentSegments.size} " +
                                "zoomed=${at.origin != Offset.Zero} turns=${at.quarterTurns}",
                        )
                        add(
                            Annotation.Highlight(
                                id = 0L,
                                pageIndex = pageIndex,
                                rects = wetHighlight,
                                color = currentColor,
                            ),
                        )
                    } else if (swept) {
                        // Dragged across the page and selected nothing. On a
                        // scan that is every drag, and without saying so the
                        // tool simply looks broken.
                        SessionRecorder.record(
                            kind = "HIGHLIGHT_MISSED",
                            detail = "page=$pageIndex runsOnPage=${currentSegments.size}",
                        )
                        highlightMissed()
                    }
                    wetHighlight = emptyList()
                },
                onDragCancel = { wetHighlight = emptyList() },
            )
        }

        // Traced: the pen keeps the touch points, thinned only by the sampling
        // rate of the screen; the cloud throws them away and scallops the ring
        // they enclosed. Same gesture, so the same branch captures it.
        tool.tracesPath -> Modifier.pointerInput(pageIndex, tool) {
            detectDragGestures(
                onDragStart = { position -> wetStroke = listOf(toPage(position)) },
                onDrag = { change, _ ->
                    change.consume()
                    wetStroke = wetStroke + toPage(change.position)
                },
                onDragEnd = {
                    // What is drawn is what was traced, for the pen. For the cloud
                    // it is the scalloped ring — built here, by the same call the
                    // preview used, so the mark is the one that was on screen when
                    // the finger came up.
                    val path = if (tool == AnnotationTool.Cloud) {
                        cloudOutline(wetStroke, currentWidth)
                    } else {
                        wetStroke
                    }
                    if (path.size > 1) {
                        // Dashed through the same splitter the shapes use, so a
                        // line type means the same thing whichever tool drew it.
                        val strokes = dashed(path, currentStyle, currentWidth)
                        SessionRecorder.record(
                            kind = "SHAPE_COMMIT",
                            detail = "page=$pageIndex tool=$tool traced=${wetStroke.size} " +
                                "points=${path.size} strokes=${strokes.size}",
                        )
                        add(
                            Annotation.Shape(
                                id = 0L,
                                pageIndex = pageIndex,
                                strokes = strokes,
                                color = currentColor,
                                strokeWidth = currentWidth,
                            ),
                        )
                    }
                    wetStroke = emptyList()
                },
                onDragCancel = { wetStroke = emptyList() },
            )
        }

        // A line, an arrow, a box or a circle: two corners rather than a path, so
        // the shape is rebuilt from the drag on every event and only committed
        // when the finger lifts.
        tool.isDragged -> Modifier.pointerInput(pageIndex, tool) {
            var start = Offset.Zero
            detectDragGestures(
                onDragStart = { position ->
                    start = toPage(position)
                    wetShape = emptyList()
                },
                onDrag = { change, _ ->
                    change.consume()
                    wetShape = shapeStrokes(
                        tool = tool,
                        start = start,
                        end = toPage(change.position),
                        style = currentStyle,
                        widthPoints = currentWidth,
                    )
                },
                onDragEnd = {
                    SessionRecorder.record(
                        kind = "SHAPE_COMMIT",
                        detail = "page=$pageIndex tool=$tool strokes=${wetShape.size}",
                    )
                    if (wetShape.isNotEmpty()) {
                        add(
                            Annotation.Shape(
                                id = 0L,
                                pageIndex = pageIndex,
                                strokes = wetShape,
                                color = currentColor,
                                strokeWidth = currentWidth,
                            ),
                        )
                    }
                    wetShape = emptyList()
                },
                onDragCancel = { wetShape = emptyList() },
            )
        }

        tool == AnnotationTool.Note -> Modifier.pointerInput(pageIndex, tool) {
            detectTapGestures { position ->
                val at = toPage(position)
                // A tap on an existing marker opens it; anywhere else starts a new
                // one. Without this the text a note holds could be typed and never
                // read again — the marker was the only thing the page ever showed,
                // which made the tool look like it added a dot and nothing else.
                //
                // Topmost first: later marks draw over earlier ones, so the last
                // match is the one under the finger.
                val hit = currentAnnotations
                    .filterIsInstance<Annotation.Note>()
                    .lastOrNull { it.isHitBy(at, tolerancePoints()) }

                if (hit != null) openNote(hit) else requestNote(at)
            }
        }

        // A tap rubs out the mark it lands on; a drag sweeps across several. Both
        // are needed: a drag never fires for a tap, because it has to pass touch
        // slop first, and a tap on a single highlight is the common case.
        tool == AnnotationTool.Eraser -> Modifier
            .pointerInput(pageIndex, tool) {
                detectTapGestures { position ->
                    eraseStart()
                    erase(toPage(position), tolerancePoints())
                    eraseEnd()
                }
            }
            .pointerInput(pageIndex, tool) {
                detectDragGestures(
                    onDragStart = { position ->
                        eraseStart()
                        eraserAt = toPage(position)
                        erase(toPage(position), tolerancePoints())
                    },
                    onDrag = { change, _ ->
                        change.consume()
                        val here = toPage(change.position)
                        eraserAt = here
                        erase(here, tolerancePoints())
                    },
                    onDragEnd = {
                        eraserAt = null
                        eraseEnd()
                    },
                    onDragCancel = {
                        eraserAt = null
                        eraseEnd()
                    },
                )
            }

        // Snapshot is not here. A capture is whatever was framed on screen, which
        // routinely spans two pages and the gap between them, so it is dragged on
        // an overlay above the whole reader rather than inside any one page — see
        // `captureOverlay`.

        // Signature is driven from its own surface. With no tool
        // the reader keeps every gesture — except that a note has to be openable
        // without first arming a tool, which is not something anyone would think
        // to try.
        //
        // The detector is added *only* on pages that actually carry a note, so the
        // no-tool path stays exactly as it was everywhere else. That matters: an
        // always-on tap handler here is the kind of thing that quietly interferes
        // with scrolling, and most pages have no note to open.
        else -> Modifier
            .pointerInput(pageIndex, tool, annotations.size) {
                detectTapGestures(
                    onTap = { position ->
                        // A note first, if one is under the finger. Otherwise a
                        // tap dismisses the selection — the same gesture every
                        // reader uses, which is why it must not also start one.
                        val note = currentAnnotations
                            .filterIsInstance<Annotation.Note>()
                            .lastOrNull { it.isHitBy(toPage(position), tolerancePoints()) }

                        if (note != null) openNote(note) else clearSelection()
                    },
                    // Long press, because a plain drag has to go on scrolling the
                    // document. It is also what selecting text means on every
                    // other reader, so it needs no explaining.
                    onLongPress = { position ->
                        selectWord(toPage(position))
                    },
                )
            }
            // Only while a selection exists on this page: an always-on drag
            // handler would race the scroller for every swipe on every page.
            .then(
                if (selection != null) {
                    Modifier.pointerInput(pageIndex, selection.rects.size) {
                        var draggingStart = false
                        detectDragGestures(
                            onDragStart = { position ->
                                val at = toPage(position)
                                val live = currentSelection ?: return@detectDragGestures
                                // Whichever handle is nearer. Grabbing the wrong
                                // one collapses the selection, which reads as the
                                // drag having deleted it.
                                draggingStart =
                                    (at - live.startHandle).getDistance() <=
                                    (at - live.endHandle).getDistance()
                            },
                            onDrag = { change, _ ->
                                change.consume()
                                moveHandle(draggingStart, toPage(change.position))
                            },
                        )
                    }
                } else {
                    Modifier
                },
            )
    }

    return this
        .then(inputModifier)
        .drawWithContent {
            drawContent()
            // Read through the state, not the captured parameter. A plain captured
            // list changes value without the draw node ever hearing about it, so
            // the display list is reused and the lambda never runs again — the
            // page composed with the new mark and kept painting the old picture.
            val marks = currentAnnotations
            // What was actually painted, recorded when it changes.
            //
            // Composition reporting a new list is not the same as the page having
            // drawn it: a draw node that never re-runs leaves the old picture on
            // screen while every counter says the mark is gone. PAGE_MARKS says
            // what the page was told; this says what it did about it, and only the
            // pair of them can tell those two failures apart.
            if (painted[0] != revision) {
                painted[0] = revision
                SessionRecorder.record(
                    kind = "PAGE_PAINT",
                    detail = "page=$pageIndex rev=$revision marks=${marks.size}",
                )
            }
            marks.forEach { drawAnnotation(it, at) }
            if (wetHighlight.isNotEmpty()) {
                drawHighlightRects(wetHighlight, currentColor, at)
            }
            // The shape as it is being dragged, drawn exactly as it will be
            // committed — same builder, same dashes — so what is released is
            // what was aimed at.
            wetShape.forEach { stroke ->
                drawInkStroke(
                    points = stroke,
                    color = currentColor,
                    widthPoints = currentWidth,
                    at = at,
                    smooth = tool == AnnotationTool.Pen,
                )
            }
            if (wetStroke.size > 1) {
                // The cloud is previewed as a cloud, rebuilt on every frame from
                // the same call that will commit it. Watching a plain trace turn
                // into scallops only on lift means aiming at something you cannot
                // see; the scallops do shuffle as the ring grows, but they shuffle
                // into the ones you are going to get.
                if (tool == AnnotationTool.Cloud) {
                    drawInkStroke(
                        points = cloudOutline(wetStroke, currentWidth),
                        color = currentColor,
                        widthPoints = currentWidth,
                        at = at,
                        smooth = false,
                    )
                } else {
                    drawInkStroke(wetStroke, currentColor, currentWidth, at)
                }
            }
            currentSelection?.let { drawSelection(it, at) }
            eraserAt?.let { point ->
                // Shows exactly how far the eraser reaches, so a miss reads as a
                // miss rather than as the tool not working.
                drawCircle(
                    color = Color.Black.copy(alpha = 0.35f),
                    radius = eraserRadiusPx,
                    center = at.toScreen(point),
                    style = Stroke(width = ERASER_RING_PX),
                )
            }
        }
}

private fun DrawScope.drawAnnotation(annotation: Annotation, at: PageMapping) {
    when (annotation) {
        is Annotation.Highlight ->
            drawHighlightRects(annotation.rects, annotation.color, at)
        is Annotation.Ink ->
            drawInkStroke(annotation.points, annotation.color, annotation.strokeWidth, at)
        is Annotation.Shape -> annotation.strokes.forEach { stroke ->
            // Not smoothed: these points are corners, not samples.
            drawInkStroke(
                points = stroke,
                color = annotation.color,
                widthPoints = annotation.strokeWidth,
                at = at,
                smooth = false,
            )
        }
        is Annotation.Signature -> annotation.strokes.forEach { stroke ->
            drawInkStroke(stroke, annotation.color, SIGNATURE_WIDTH_POINTS, at)
        }
        is Annotation.Note -> {
            // An anchored marker; the note's text is shown in a sheet rather than
            // painted onto the page, where it would obscure what it annotates.
            //
            // Outlined, rather than a plain disc in the pen colour. A 7 pt yellow
            // dot on white paper is genuinely hard to find, and the first version
            // of this was reported as the note not being added at all — which is
            // exactly what an invisible marker looks like.
            val centre = at.toScreen(annotation.anchor)
            val radius = at.toScreen(NOTE_MARKER_RADIUS_POINTS)
            drawCircle(Color(annotation.color), radius = radius, center = centre)
            drawCircle(
                Color.Black.copy(alpha = NOTE_OUTLINE_ALPHA),
                radius = radius,
                center = centre,
                style = Stroke(width = at.toScreen(NOTE_OUTLINE_WIDTH_POINTS)),
            )
            // A pip in the middle, so it reads as a marker rather than as a stray
            // blob of highlighter.
            drawCircle(
                Color.Black.copy(alpha = NOTE_OUTLINE_ALPHA),
                radius = radius * NOTE_PIP_FRACTION,
                center = centre,
            )
        }
    }
}

private fun DrawScope.drawHighlightRects(
    rects: List<Rect>,
    color: Long,
    at: PageMapping,
) {
    // Multiply, so the page's own text stays legible through the wash rather than
    // being covered by a flat translucent block.
    rects.forEach { r ->
        val box = at.toScreen(r)
        drawRect(
            color = Color(color).copy(alpha = HIGHLIGHT_ALPHA),
            topLeft = box.topLeft,
            size = Size(box.width, box.height),
        )
    }
}

private fun DrawScope.drawInkStroke(
    points: List<Offset>,
    color: Long,
    widthPoints: Float,
    at: PageMapping,
    /**
     * Whether to round the corners off.
     *
     * True for freehand, where the points are touch samples and the curve is
     * what the hand actually did. False for a shape: its points are its corners,
     * and smoothing them turns a rectangle into an oval — which is precisely what
     * it did the first time this drew one.
     */
    smooth: Boolean = true,
) {
    if (points.size < 2) return
    fun place(p: Offset) = at.toScreen(p)

    val path = Path().apply {
        val start = place(points[0])
        moveTo(start.x, start.y)
        if (smooth) {
            // Quadratic midpoints, so a fast drag reads as a smooth line instead
            // of the visible polygon that joining raw touch samples produces.
            for (i in 1 until points.size) {
                val prev = place(points[i - 1])
                val cur = place(points[i])
                quadraticBezierTo(prev.x, prev.y, (prev.x + cur.x) / 2f, (prev.y + cur.y) / 2f)
            }
            val last = place(points.last())
            lineTo(last.x, last.y)
        } else {
            for (i in 1 until points.size) {
                val cur = place(points[i])
                lineTo(cur.x, cur.y)
            }
        }
    }
    drawPath(
        path = path,
        color = Color(color),
        style = Stroke(
            width = at.toScreen(widthPoints),
            cap = StrokeCap.Round,
            join = StrokeJoin.Round,
        ),
    )
}

/** Highlighter wash. Low enough to read through, high enough to see. */
private const val HIGHLIGHT_ALPHA = 0.35f

/** Marker nib, in page points, so it thickens with the page rather than the screen. */
private const val MARKER_WIDTH_POINTS = 2.4f
private const val SIGNATURE_WIDTH_POINTS = 1.8f
private const val ERASER_RING_PX = 2f

/**
 * How far the eraser reaches from the touch point.
 *
 * A highlight is a few points tall and an ink stroke has no area at all, so an
 * exact hit test would ask for precision no finger has. Roughly half a fingertip.
 */
private val ERASER_TOUCH_RADIUS = 14.dp

/** Ink of the note marker's outline and pip. */
private const val NOTE_OUTLINE_ALPHA = 0.65f

/** Outline thickness, in page points, so it holds up at any zoom. */
private const val NOTE_OUTLINE_WIDTH_POINTS = 1.2f

/** Pip radius as a fraction of the marker's. */
private const val NOTE_PIP_FRACTION = 0.32f

/**
 * The selected text, and the handles that adjust it.
 *
 * A wash under the words rather than an outline around them: the point is to
 * show which text is selected, and an outline around a ragged multi-line span is
 * far harder to read than a band behind it.
 *
 * The handles are teardrops hanging below each end, which is the shape every
 * Android text field uses. Copying that is not laziness — a control someone has
 * used a thousand times needs no discovering.
 */
private fun DrawScope.drawSelection(
    selection: PageTextSelection,
    at: PageMapping,
) {
    if (selection.rects.isEmpty()) return

    selection.rects.forEach { rect ->
        val box = at.toScreen(rect)
        drawRect(
            color = SELECTION_COLOUR.copy(alpha = SELECTION_ALPHA),
            topLeft = box.topLeft,
            size = Size(box.width, box.height),
        )
    }

    listOf(selection.startHandle, selection.endHandle).forEach { handle ->
        val anchor = at.toScreen(handle)
        val centre = anchor + Offset(0f, HANDLE_RADIUS_PX)
        drawCircle(SELECTION_COLOUR, radius = HANDLE_RADIUS_PX, center = centre)
        // The stem, so the circle reads as attached to the text rather than
        // floating below it.
        drawLine(
            color = SELECTION_COLOUR,
            start = anchor,
            end = centre,
            strokeWidth = HANDLE_STEM_PX,
        )
    }
}

/** The selection wash and its handles. Blue, as every reader on the platform. */
private val SELECTION_COLOUR = Color(0xFF4C8DF6)

/** Low enough to read the words through, high enough to see which they are. */
private const val SELECTION_ALPHA = 0.32f

/** Handles in pixels, not page points: a grab target is a finger, not a font. */
private const val HANDLE_RADIUS_PX = 18f
private const val HANDLE_STEM_PX = 3f
