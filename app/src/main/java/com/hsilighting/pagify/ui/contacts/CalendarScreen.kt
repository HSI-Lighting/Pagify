package com.hsilighting.pagify.ui.contacts

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ChevronLeft
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.Contact
import java.util.Calendar
import java.util.Locale

/**
 * The month, and which days have anything on them.
 *
 * **A card is met on a date.** A contact list sorted by name loses that entirely,
 * and the thing somebody actually remembers about a card is not the name on it —
 * it is the day, and often the event. So the calendar is a second way into the
 * same contacts rather than a separate feature: no new storage, no new state,
 * just `capturedAt` read as a date instead of a sort key.
 *
 * Two kinds of mark, because they answer different questions. A **filled** day
 * is one where cards were collected, which is memory. A **ringed** day is one
 * where a reminder falls due, which is obligation. A day can be both, and the
 * two are drawn differently rather than in two colours, so they are still
 * distinguishable to somebody who cannot separate the hues.
 */
@Composable
fun CalendarScreen(
    contacts: List<Contact>,
    onOpenContact: (Contact) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val today = remember { startOfDay(System.currentTimeMillis()) }
    var month by remember { mutableStateOf(startOfMonth(today)) }
    var selected by remember { mutableStateOf<Long?>(null) }

    // Both indexes are built once per change of the contact list rather than per
    // recomposition: a month scroll must not re-bucket every contact.
    val captured = remember(contacts) { contacts.groupBy { startOfDay(it.capturedAt) } }
    val reminders = remember(contacts) {
        contacts.filter { it.reminderAt != null && it.reminderDoneAt == null }
            .groupBy { startOfDay(it.reminderAt!!) }
    }

    Column(modifier.fillMaxSize()) {
        Row(
            Modifier.fillMaxWidth().padding(start = 4.dp, end = 8.dp, top = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back to contacts")
            }
            Text(
                text = monthName(month),
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { month = addMonths(month, -1); selected = null }) {
                Icon(Icons.Filled.ChevronLeft, contentDescription = "The month before")
            }
            IconButton(onClick = { month = addMonths(month, 1); selected = null }) {
                Icon(Icons.Filled.ChevronRight, contentDescription = "The month after")
            }
        }

        MonthGrid(
            month = month,
            today = today,
            selected = selected,
            captured = captured,
            reminders = reminders,
            onPick = { day -> selected = if (selected == day) null else day },
        )

        val chosen = selected
        if (chosen == null) {
            Summary(month, captured, reminders)
        } else {
            DayEntries(
                day = chosen,
                captured = captured[chosen].orEmpty(),
                due = reminders[chosen].orEmpty(),
                onOpenContact = onOpenContact,
            )
        }
    }
}

@Composable
private fun MonthGrid(
    month: Long,
    today: Long,
    selected: Long?,
    captured: Map<Long, List<Contact>>,
    reminders: Map<Long, List<Contact>>,
    onPick: (Long) -> Unit,
) {
    val weekdays = remember { weekdayInitials() }

    Column(Modifier.fillMaxWidth().padding(horizontal = 8.dp)) {
        Row(Modifier.fillMaxWidth()) {
            weekdays.forEach { initial ->
                Text(
                    text = initial,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.weight(1f).padding(vertical = 6.dp),
                )
            }
        }

        // Weeks as rows of seven, with the leading blanks a month needs before
        // its first day lands on the right weekday.
        val days = daysOfMonth(month)
        val blanks = leadingBlanks(month)
        val cells = List(blanks) { null } + days
        cells.chunked(7).forEach { week ->
            Row(Modifier.fillMaxWidth()) {
                week.forEach { day ->
                    if (day == null) {
                        Box(Modifier.weight(1f).aspectRatio(1f))
                    } else {
                        DayCell(
                            day = day,
                            isToday = day == today,
                            isSelected = day == selected,
                            capturedCount = captured[day]?.size ?: 0,
                            hasReminder = reminders.containsKey(day),
                            onPick = onPick,
                            modifier = Modifier.weight(1f),
                        )
                    }
                }
                // A short last week keeps its cells the right size rather than
                // stretching them across the row.
                repeat(7 - week.size) { Box(Modifier.weight(1f).aspectRatio(1f)) }
            }
        }
    }
}

@Composable
private fun DayCell(
    day: Long,
    isToday: Boolean,
    isSelected: Boolean,
    capturedCount: Int,
    hasReminder: Boolean,
    onPick: (Long) -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val hasCards = capturedCount > 0

    // Selection wins over "has cards" wins over plain, so the day being looked
    // at is never ambiguous.
    val background = when {
        isSelected -> scheme.primary
        hasCards -> scheme.primaryContainer
        else -> Color.Transparent
    }
    val foreground = when {
        isSelected -> scheme.onPrimary
        hasCards -> scheme.onPrimaryContainer
        isToday -> scheme.primary
        else -> scheme.onSurface
    }

    Box(
        modifier
            .aspectRatio(1f)
            .padding(2.dp)
            .clip(CircleShape)
            .background(background)
            .clickable { onPick(day) },
        contentAlignment = Alignment.Center,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text(
                text = dayOfMonth(day).toString(),
                style = MaterialTheme.typography.bodyMedium,
                color = foreground,
                fontWeight = if (isToday || hasCards) FontWeight.Bold else FontWeight.Normal,
            )
            // The count, when there is more than one, so a busy day reads as busy
            // without opening it.
            if (capturedCount > 1) {
                Text(
                    text = capturedCount.toString(),
                    style = MaterialTheme.typography.labelSmall,
                    color = foreground.copy(alpha = 0.75f),
                )
            }
        }
        // A ring rather than another fill: a day can be both collected-on and
        // due, and two fills cannot show that.
        if (hasReminder) {
            Box(
                Modifier
                    .size(6.dp)
                    .align(Alignment.BottomCenter)
                    .padding(bottom = 0.dp)
                    .clip(CircleShape)
                    .background(if (isSelected) scheme.onPrimary else scheme.error),
            )
        }
    }
}

/** What the month holds, when no day is chosen. */
@Composable
private fun Summary(
    month: Long,
    captured: Map<Long, List<Contact>>,
    reminders: Map<Long, List<Contact>>,
) {
    val cards = captured.filterKeys { inMonth(it, month) }.values.sumOf { it.size }
    val days = captured.keys.count { inMonth(it, month) }
    val due = reminders.filterKeys { inMonth(it, month) }.values.sumOf { it.size }

    Column(Modifier.fillMaxWidth().padding(24.dp)) {
        Text(
            text = when (cards) {
                0 -> "No cards this month"
                1 -> "One card, on one day"
                else -> "$cards cards across $days ${if (days == 1) "day" else "days"}"
            },
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (due > 0) {
            Text(
                text = "$due ${if (due == 1) "reminder" else "reminders"} due",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(top = 6.dp),
            )
        }
        Text(
            text = "Tap a day to see who was met.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 14.dp),
        )
    }
}

/** Who was met on the chosen day, and who is due that day. */
@Composable
private fun DayEntries(
    day: Long,
    captured: List<Contact>,
    due: List<Contact>,
    onOpenContact: (Contact) -> Unit,
) {
    Column(Modifier.fillMaxSize()) {
        Text(
            text = fullDate(day),
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.padding(start = 20.dp, end = 20.dp, top = 12.dp, bottom = 6.dp),
        )

        if (captured.isEmpty() && due.isEmpty()) {
            Text(
                text = "Nothing on this day.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
            )
            return@Column
        }

        LazyColumn(Modifier.fillMaxSize()) {
            if (due.isNotEmpty()) {
                item {
                    SectionHeading("Reminders due", MaterialTheme.colorScheme.error)
                }
                items(due, key = { "due-${it.id}" }) { contact ->
                    DayRow(contact, isDue = true, onOpenContact = onOpenContact)
                }
            }
            if (captured.isNotEmpty()) {
                item { SectionHeading("Met this day", MaterialTheme.colorScheme.onSurfaceVariant) }
                items(captured, key = { "met-${it.id}" }) { contact ->
                    DayRow(contact, isDue = false, onOpenContact = onOpenContact)
                }
            }
        }
    }
}

@Composable
private fun SectionHeading(text: String, colour: Color) {
    Text(
        text = text.uppercase(Locale.getDefault()),
        style = MaterialTheme.typography.labelSmall,
        color = colour,
        fontWeight = FontWeight.SemiBold,
        modifier = Modifier.padding(start = 20.dp, end = 20.dp, top = 12.dp, bottom = 4.dp),
    )
}

@Composable
private fun DayRow(contact: Contact, isDue: Boolean, onOpenContact: (Contact) -> Unit) {
    Surface(
        color = MaterialTheme.colorScheme.surface,
        shape = RoundedCornerShape(12.dp),
        tonalElevation = 1.dp,
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 3.dp)
            .clickable { onOpenContact(contact) },
    ) {
        Row(
            Modifier.padding(horizontal = 14.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    text = contact.displayName,
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                val beneath = listOfNotNull(
                    contact.company.takeIf { it.isNotBlank() },
                    contact.stage.label.takeIf { contact.stage != com.hsilighting.pagify.data.db.DealStage.New },
                ).joinToString(" · ")
                if (beneath.isNotBlank()) {
                    Text(
                        text = beneath,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            if (contact.met) {
                Text(
                    text = "Met",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 8.dp),
                )
            }
            if (isDue) {
                Box(
                    Modifier
                        .padding(start = 10.dp)
                        .size(8.dp)
                        .clip(CircleShape)
                        .background(MaterialTheme.colorScheme.error),
                )
            }
        }
    }
}

// ------------------------------------------------------------------ dates --
//
// `java.util.Calendar` rather than `java.time`, because minSdk is 24 and
// `java.time` needs desugaring turned on. It is a worse API and it is the one
// that runs on every phone this app supports without adding a build flag.
//
// Every function here works in the device's own time zone deliberately: a card
// collected at an evening reception belongs to that evening's date as the person
// remembers it, not to the UTC day it happened to fall in.

private fun calendarAt(millis: Long): Calendar =
    Calendar.getInstance().apply { timeInMillis = millis }

/** Midnight at the start of the day this instant falls in. */
internal fun startOfDay(millis: Long): Long = calendarAt(millis).apply {
    set(Calendar.HOUR_OF_DAY, 0)
    set(Calendar.MINUTE, 0)
    set(Calendar.SECOND, 0)
    set(Calendar.MILLISECOND, 0)
}.timeInMillis

/** Midnight on the first of the month this instant falls in. */
internal fun startOfMonth(millis: Long): Long = calendarAt(startOfDay(millis)).apply {
    set(Calendar.DAY_OF_MONTH, 1)
}.timeInMillis

internal fun addMonths(monthStart: Long, by: Int): Long =
    calendarAt(monthStart).apply { add(Calendar.MONTH, by) }.timeInMillis

internal fun dayOfMonth(millis: Long): Int = calendarAt(millis).get(Calendar.DAY_OF_MONTH)

internal fun inMonth(day: Long, monthStart: Long): Boolean {
    val a = calendarAt(day)
    val b = calendarAt(monthStart)
    return a.get(Calendar.YEAR) == b.get(Calendar.YEAR) &&
        a.get(Calendar.MONTH) == b.get(Calendar.MONTH)
}

/** Every day of the month, as midnights. */
internal fun daysOfMonth(monthStart: Long): List<Long> {
    val calendar = calendarAt(monthStart)
    val count = calendar.getActualMaximum(Calendar.DAY_OF_MONTH)
    return (0 until count).map { offset ->
        calendarAt(monthStart).apply { add(Calendar.DAY_OF_MONTH, offset) }.timeInMillis
    }
}

/**
 * How many blank cells come before the first of the month.
 *
 * Counted from the locale's own first day of the week, so a grid that starts on
 * Monday in Europe starts on Sunday where that is the convention. Getting this
 * wrong shifts every date in the month by a column, which looks like data loss
 * rather than a layout bug.
 */
internal fun leadingBlanks(monthStart: Long): Int {
    val calendar = calendarAt(monthStart)
    val firstDayOfWeek = calendar.firstDayOfWeek
    val dayOfWeek = calendar.get(Calendar.DAY_OF_WEEK)
    return (dayOfWeek - firstDayOfWeek + 7) % 7
}

private fun weekdayInitials(): List<String> {
    val calendar = Calendar.getInstance()
    val first = calendar.firstDayOfWeek
    val symbols = java.text.DateFormatSymbols.getInstance().shortWeekdays
    return (0 until 7).map { offset ->
        val index = ((first - 1 + offset) % 7) + 1
        symbols[index].take(1)
    }
}

private fun monthName(monthStart: Long): String =
    java.text.SimpleDateFormat("LLLL yyyy", Locale.getDefault()).format(monthStart)

private fun fullDate(day: Long): String =
    java.text.SimpleDateFormat("EEEE d MMMM yyyy", Locale.getDefault()).format(day)
