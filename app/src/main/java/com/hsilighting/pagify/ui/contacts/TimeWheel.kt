package com.hsilighting.pagify.ui.contacts

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.snapping.rememberSnapFlingBehavior
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import java.util.Calendar
import java.util.Locale
import java.util.TimeZone

/**
 * How far apart the offered minutes are.
 *
 * Five, not one. A wheel of sixty minutes is a long scroll to land on a number
 * nobody chose deliberately: meetings happen at half past and at quarter to, not
 * at 14:37. Twelve stops fit in two flicks, and every time the app can already
 * hold — the nine o'clock offsets, anything picked here before — sits on one of
 * them, so nothing existing becomes unrepresentable.
 */
internal const val MINUTE_STEP = 5

/**
 * Midnight UTC on the local date of a moment — the form a date picker speaks.
 *
 * Material's date picker reports and accepts midnight *UTC*, while the person
 * using it lives in a local day. East of Greenwich a local millis handed
 * straight to the picker lands on the right day by luck; west of it, it lands on
 * the day before. Converting deliberately is the only way this is right in both
 * halves of the world.
 */
internal fun pickerDayOf(millis: Long): Long {
    val local = Calendar.getInstance().apply { timeInMillis = millis }
    return Calendar.getInstance(TimeZone.getTimeZone("UTC")).apply {
        clear()
        set(local.get(Calendar.YEAR), local.get(Calendar.MONTH), local.get(Calendar.DAY_OF_MONTH))
    }.timeInMillis
}

/**
 * Whether a day and time have already gone.
 *
 * A reminder set for a moment that has passed is not a reminder — either it
 * fires the instant it is saved, or it never fires at all, and both look to the
 * person who set it like the app quietly lost it.
 */
internal fun hasPassed(
    dayUtcMillis: Long,
    hour: Int,
    minute: Int,
    now: Long = System.currentTimeMillis(),
): Boolean = atLocalTime(dayUtcMillis, hour, minute) <= now

/**
 * Where the wheels should be standing when they appear.
 *
 * On a later day, nine in the morning — the same default the offsets use.
 *
 * On **today**, the next five-minute mark that has not yet arrived. This is what
 * removes the morning/afternoon trap: at half past three the wheel opens on 3:35
 * PM, so PM is already the answer rather than something to notice and change,
 * and nine o'clock — which at half past three means nine tomorrow, or nothing —
 * cannot be accepted by mistake.
 *
 * Late enough at night there is no mark left before midnight. Rather than roll
 * silently into tomorrow, this stops at the last one and lets the caller refuse
 * it: a reminder landing on a different day than the one on screen is worse than
 * one that says plainly it cannot be set.
 */
internal fun firstOfferableTime(
    dayUtcMillis: Long,
    now: Long = System.currentTimeMillis(),
): Pair<Int, Int> {
    if (pickerDayOf(now) != dayUtcMillis) return 9 to 0

    val local = Calendar.getInstance().apply { timeInMillis = now }
    val minutesSoFar = local.get(Calendar.HOUR_OF_DAY) * 60 + local.get(Calendar.MINUTE)
    // Strictly after now, so the wheel never opens on a moment that has gone by
    // the time somebody reaches the Set button.
    val next = (minutesSoFar / MINUTE_STEP + 1) * MINUTE_STEP
    if (next >= 24 * 60) return 23 to (60 - MINUTE_STEP)
    return (next / 60) to (next % 60)
}

/** A 24-hour hour as it is spoken: one to twelve, and whether it is afternoon. */
internal fun to12Hour(hour24: Int): Pair<Int, Boolean> {
    val pm = hour24 >= 12
    val h = hour24 % 12
    return (if (h == 0) 12 else h) to pm
}

/** The inverse: midnight is 12 AM and noon is 12 PM, which is the awkward part. */
internal fun to24Hour(hour12: Int, pm: Boolean): Int {
    val base = if (hour12 == 12) 0 else hour12
    return if (pm) base + 12 else base
}

/**
 * Where a given minute sits among the ones offered.
 *
 * Rounds down, so an existing nine o'clock stays nine o'clock. A value off the
 * step could only come from a version that offered one, and losing four minutes
 * from a reminder is not worth a special case.
 */
internal fun minuteIndexOf(minute: Int): Int =
    (minute / MINUTE_STEP).coerceIn(0, 60 / MINUTE_STEP - 1)

/**
 * Three columns you scroll, instead of a clock face you aim at.
 *
 * The dial this replaces asks two questions in one gesture — which number, and
 * which half of the day — and answers the second itself, with AM, before anybody
 * has looked at it. A wheel puts AM and PM on the screen as a column with the
 * chosen one in the middle, so the answer is visible rather than assumed.
 *
 * The wheels own their position. Driving them from outside while a finger is on
 * them makes the list fight the scroll, so this reports upward and is never
 * pushed back.
 */
@Composable
internal fun TimeWheels(
    initialHour: Int,
    initialMinute: Int,
    is24Hour: Boolean,
    onChange: (hour: Int, minute: Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val hours = remember(is24Hour) { if (is24Hour) (0..23).toList() else (1..12).toList() }
    val minutes = remember { (0 until 60 step MINUTE_STEP).toList() }

    val spoken = remember(initialHour) { to12Hour(initialHour) }
    var hourIndex by remember {
        mutableIntStateOf(if (is24Hour) initialHour else hours.indexOf(spoken.first))
    }
    var minuteIndex by remember { mutableIntStateOf(minuteIndexOf(initialMinute)) }
    var pmIndex by remember { mutableIntStateOf(if (spoken.second) 1 else 0) }

    LaunchedEffect(hourIndex, minuteIndex, pmIndex, is24Hour) {
        val hour = if (is24Hour) hours[hourIndex] else to24Hour(hours[hourIndex], pmIndex == 1)
        onChange(hour, minutes[minuteIndex])
    }

    Box(
        modifier = modifier.height(ROW_HEIGHT * VISIBLE_ROWS),
        contentAlignment = Alignment.Center,
    ) {
        // The band that says which row counts. Behind the wheels, so the digits
        // sitting in it stay legible.
        Box(
            Modifier
                .fillMaxWidth()
                .height(ROW_HEIGHT)
                .clip(RoundedCornerShape(12.dp))
                .background(MaterialTheme.colorScheme.secondaryContainer),
        )

        Row(
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Wheel(
                labels = hours.map {
                    if (is24Hour) String.format(Locale.getDefault(), "%02d", it) else it.toString()
                },
                index = hourIndex,
                onIndex = { hourIndex = it },
                label = "Hour",
            )
            Text(":", style = MaterialTheme.typography.headlineSmall)
            Wheel(
                labels = minutes.map { String.format(Locale.getDefault(), "%02d", it) },
                index = minuteIndex,
                onIndex = { minuteIndex = it },
                label = "Minute",
            )
            if (!is24Hour) {
                Wheel(
                    labels = listOf("AM", "PM"),
                    index = pmIndex,
                    onIndex = { pmIndex = it },
                    label = "Morning or afternoon",
                )
            }
        }
    }
}

/**
 * One scrolling column, with the middle row as the answer.
 *
 * The padding above and below is what lets the first and last entries reach the
 * middle; without it a wheel can never select its own ends. Because of it the
 * centred item is exactly the first visible one, which is why the index needs no
 * arithmetic.
 */
@Composable
private fun Wheel(
    labels: List<String>,
    index: Int,
    onIndex: (Int) -> Unit,
    label: String,
) {
    val state = rememberLazyListState(initialFirstVisibleItemIndex = index)

    // Report on settling rather than during the scroll: a wheel dragged past
    // four numbers should not set four times, and the value that matters is the
    // one it stops on.
    LaunchedEffect(state) {
        snapshotFlow { state.isScrollInProgress }.collect { scrolling ->
            if (!scrolling) onIndex(state.firstVisibleItemIndex)
        }
    }

    LazyColumn(
        state = state,
        flingBehavior = rememberSnapFlingBehavior(state),
        contentPadding = PaddingValues(vertical = ROW_HEIGHT * ((VISIBLE_ROWS - 1) / 2)),
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier
            .width(if (labels.size == 2) 68.dp else 62.dp)
            .height(ROW_HEIGHT * VISIBLE_ROWS)
            .semantics { contentDescription = label },
    ) {
        itemsIndexed(labels) { at, text ->
            val centred = at == state.firstVisibleItemIndex
            Box(
                modifier = Modifier
                    .height(ROW_HEIGHT)
                    .fillMaxWidth(),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = text,
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = if (centred) FontWeight.SemiBold else FontWeight.Normal,
                    color = if (centred) {
                        MaterialTheme.colorScheme.onSecondaryContainer
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.55f)
                    },
                )
            }
        }
    }
}

private val ROW_HEIGHT = 46.dp

/** Odd, so there is a middle row for the selection to sit in. */
private const val VISIBLE_ROWS = 5

/**
 * Whether a day is one a reminder can still be set for.
 *
 * Today counts: most of it is usually still ahead, and the time step refuses
 * whatever is left over. Yesterday does not, at any hour — a reminder pointing
 * backwards can only ever be a mistake, and greying those days out says so
 * before the mistake is made rather than after.
 */
internal fun isOfferableDay(utcTimeMillis: Long, now: Long = System.currentTimeMillis()): Boolean =
    utcTimeMillis >= pickerDayOf(now)
