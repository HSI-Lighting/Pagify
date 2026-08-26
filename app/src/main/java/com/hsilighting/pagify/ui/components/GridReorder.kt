package com.hsilighting.pagify.ui.components

import com.hsilighting.pagify.core.SessionRecorder
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.lazy.grid.LazyGridItemInfo
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.IntOffset
import androidx.compose.runtime.withFrameNanos
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Dragging items around a grid to reorder them.
 *
 * Replaces a pair of nudge arrows on each cell. Two things were wrong with those,
 * and this fixes both. Moving a page five places meant five taps — and each tap
 * was a *document edit*, so each one re-rendered every thumbnail in the grid. The
 * sheet spent most of a reorder redrawing.
 *
 * A drag moves the item in a list held here, on screen only. Nothing reaches the
 * document until the finger lifts, and then it is one move: one edit, one
 * re-render, one entry in the undo history to reverse it with.
 *
 * ## Why the position is absolute
 *
 * The obvious way to draw a dragged item is to accumulate the drag deltas and
 * translate by them. It does not work here, and the way it fails is exactly what
 * it looks like: **the page snaps a whole cell sideways the moment it passes a
 * neighbour.**
 *
 * The reason is that the item is also moving in the *list*. When the order
 * changes, the grid lays the item out at its new slot, so the position the
 * translation is applied to has itself jumped a cell — and the drawn result jumps
 * with it. Compensating for that after the fact means reading the new layout
 * before it exists, which is a frame late and visibly wrong.
 *
 * So nothing here is relative. The item's absolute position at the start of the
 * drag is remembered, and where it should be drawn now is
 *
 *     startPosition + howFarTheFingerHasMoved
 *
 * which does not depend on slots at all. [displacement] turns that into a
 * translation by subtracting wherever the grid has *currently* laid the item out.
 * Whatever the layout does, the two cancel and the page stays under the finger.
 * It settles into place when the finger lifts, and not before.
 */
class GridReorderState(
    private val gridState: LazyGridState,
    private val scope: CoroutineScope,
    /** Called once, on drop, with where the item started and where it ended. */
    private val onMove: (from: Int, to: Int) -> Unit,
) {
    /** Where the dragged item started, or null when nothing is being dragged. */
    var origin by mutableStateOf<Int?>(null)
        private set

    /** Which slot it currently occupies. Its neighbours have made room. */
    var slot by mutableStateOf<Int?>(null)
        private set

    /** Where the item sat when the drag began, in the grid's own coordinates. */
    private var startPosition = Offset.Zero

    /** How far the finger has travelled since. */
    private var travelled by mutableStateOf(Offset.Zero)

    /** The edge scroll, if one is running. */
    private var scrolling: Job? = null

/**
     * How fast it is going, in pixels per frame. Zero when it is not.
     *
     * Written by the drag and read by the running job every frame, which is what
     * lets one job serve a speed that changes continuously.
     */
    private var speed by mutableStateOf(0f)

    /**
     * The order the grid should draw, as page indices.
     *
     * Empty when nothing is being dragged, which is most of the time.
     */
    var order by mutableStateOf<List<Int>>(emptyList())
        private set

    /** Whether [index] is the page under the finger. */
    fun isDragging(index: Int): Boolean {
        val at = slot ?: return false
        return order.getOrNull(at) == index
    }

    fun start(count: Int, from: Int) {
        val item = itemAt(from) ?: return
        order = (0 until count).toList()
        origin = from
        slot = from
        startPosition = Offset(item.offset.x.toFloat(), item.offset.y.toFloat())
        travelled = Offset.Zero
    }

    /**
     * Follow the finger, and shuffle the order when the item's own centre crosses
     * into another cell.
     *
     * The item's centre rather than the finger: a page picked up by its corner
     * would otherwise swap far too eagerly on one side and not at all on the
     * other.
     */
    fun moveBy(delta: Offset) {
        val currentSlot = slot ?: return
        travelled += delta

        val size = itemAt(currentSlot)?.size ?: return
        val centre = startPosition + travelled +
            Offset(size.width / 2f, size.height / 2f)

        scrollIfNearAnEdge(centre.y)

        val target = gridState.layoutInfo.visibleItemsInfo
            .firstOrNull { item ->
                centre.x >= item.offset.x &&
                    centre.x <= item.offset.x + item.size.width &&
                    centre.y >= item.offset.y &&
                    centre.y <= item.offset.y + item.size.height
            }
            ?.index
            ?: return

        if (target == currentSlot || target !in order.indices) return

        // Moved, not swapped. Swapping sends the displaced page all the way back
        // to where the dragged one came from, which across four pages is not what
        // anybody meant.
        order = order.toMutableList().apply { add(target, removeAt(currentSlot)) }
        slot = target
        // Deliberately no adjustment to `travelled`. The drawn position is
        // absolute, so the layout moving underneath changes nothing about where
        // this page appears — see the note on the class.
    }

    fun drop() {
        val from = origin
        val to = slot
        clear()
        if (from != null && to != null && from != to) onMove(from, to)
    }

    fun cancel() = clear()

    private fun clear() {
        scrolling?.cancel()
        scrolling = null
        speed = 0f
        origin = null
        slot = null
        travelled = Offset.Zero
        startPosition = Offset.Zero
        order = emptyList()
    }

    /**
     * How far to translate the page at [index] from wherever the grid put it.
     *
     * Zero for every page except the one being dragged.
     */
    fun displacement(index: Int): IntOffset {
        val at = slot ?: return IntOffset.Zero
        if (order.getOrNull(at) != index) return IntOffset.Zero
        val laidOut = itemAt(at) ?: return IntOffset.Zero

        val wanted = startPosition + travelled
        return IntOffset(
            (wanted.x - laidOut.offset.x).toInt(),
            (wanted.y - laidOut.offset.y).toInt(),
        )
    }

    private fun itemAt(index: Int): LazyGridItemInfo? =
        gridState.layoutInfo.visibleItemsInfo.firstOrNull { it.index == index }

    /**
     * Scroll when the drag reaches the top or bottom of the grid.
     *
     * Without it a page can only be moved as far as the screen, and the last page
     * of a long document could never be dragged to the front at all.
     *
     * ## Why this is a job and not a scroll
     *
     * The first version scrolled a fixed step on every drag event. Drag events
     * arrive with every frame the finger moves, and each one launched its own
     * coroutine — so the grid took dozens of scrolls a second, all at once, and
     * a page nudged towards the bottom shot to the end of the document before
     * anybody could let go.
     *
     * One job instead, started when the drag enters the edge and cancelled when
     * it leaves. Its speed is set by how far into the edge the page has reached,
     * so it eases in rather than snapping to full pelt, and it is measured per
     * *frame* rather than per event — which is what makes it a speed at all
     * instead of a function of how fast a finger happens to be moving.
     */
    private fun scrollIfNearAnEdge(y: Float) {
        val info = gridState.layoutInfo
        val top = info.viewportStartOffset.toFloat()
        val bottom = info.viewportEndOffset.toFloat()

        speed = when {
            y < top + EDGE -> -depth(top + EDGE - y) * TOP_SPEED
            y > bottom - EDGE -> depth(y - (bottom - EDGE)) * TOP_SPEED
            else -> 0f
        }

        if (speed == 0f) {
            scrolling?.cancel()
            scrolling = null
            return
        }

        // Started only when there is no job already running. The speed varies
        // continuously with how deep into the edge the page has reached, so a
        // job restarted whenever it *changed* was a job restarted on nearly
        // every drag event — and a cancel is not instant, so the old ones each
        // got another scroll in before dying. What was meant to be ten pixels a
        // frame measured twenty-five thousand a second: thirty-two pages in a
        // seventh of a second, off the top of the document. The running job
        // reads `speed` each frame instead, so it needs no restarting.
        if (scrolling?.isActive == true) return

        scrolling = scope.launch {
            while (isActive && speed != 0f) {
                // The grid scrolling moves every item, the dragged one
                // included — so the position it started from has to move with
                // it, or the page slides away from the finger by however far
                // the grid travelled.
                val moved = gridState.scrollBy(speed)
                startPosition -= Offset(0f, moved)
                SessionRecorder.record(
                    kind = "DRAG_SCROLL",
                    detail = "speed=$speed moved=$moved",
                )
                if (moved == 0f) break
                withFrameNanos { }
            }
            scrolling = null
        }
    }

    /** How far into the edge zone, from 0 at its outer limit to 1 at the very edge. */
    private fun depth(into: Float) = (into / EDGE).coerceIn(0f, 1f)

    private companion object {
        /** How close to an edge starts a scroll, in pixels. */
        const val EDGE = 140f

        /**
         * The fastest the grid scrolls, in pixels per frame.
         *
         * About 600 a second at sixty frames — a row and a half, which is quick
         * enough to cross a long document without being quick enough to lose
         * track of where the page is going.
         */
        const val TOP_SPEED = 10f
    }
}

@Composable
fun rememberGridReorderState(
    gridState: LazyGridState,
    onMove: (from: Int, to: Int) -> Unit,
): GridReorderState {
    val scope = rememberCoroutineScope()
    return remember(gridState) { GridReorderState(gridState, scope, onMove) }
}

/**
 * Makes a cell draggable after a long press.
 *
 * After a long press, not immediately: the grid scrolls, and a cell that moved
 * the moment a finger touched it would make scrolling impossible.
 *
 * ## Why the slot is not a key
 *
 * The obvious spelling is `pointerInput(slot) { ... }`, and it is why the first
 * version of this did nothing: a page would lift, and snap straight back as soon
 * as it moved.
 *
 * A drag *changes the slot* — that is the whole point of it. Keying the pointer
 * input on the slot therefore tears the gesture handler down and builds a new one
 * the instant the page passes a neighbour, which arrives as `onDragCancel` and
 * puts everything back. The gesture could never survive its own success.
 *
 * So the key is the page count, which a drag does not change, and the current
 * slot is read through [rememberUpdatedState] — the same fix this codebase has
 * needed twice before for a value captured once inside a `pointerInput`.
 */
@Composable
fun Modifier.reorderable(
    state: GridReorderState,
    slot: Int,
    count: Int,
    enabled: Boolean,
): Modifier {
    val currentSlot by rememberUpdatedState(slot)
    if (!enabled) return this
    return pointerInput(count) {
        detectDragGesturesAfterLongPress(
            onDragStart = { state.start(count, currentSlot) },
            onDrag = { change, delta ->
                change.consume()
                state.moveBy(delta)
            },
            onDragEnd = { state.drop() },
            onDragCancel = { state.cancel() },
        )
    }
}
