package com.hsilighting.pagify.ui.contacts

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Event
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.TimePicker
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.data.db.DealStage
import java.util.Calendar
import java.util.Locale

/**
 * Where a contact has got to, and when to chase it.
 *
 * Deliberately small. A card scanned at a stand is worth thirty seconds of
 * attention, standing up, and a form that asks for more than that gets skipped —
 * at which point the CRM holds nothing and is worse than no CRM, because the
 * empty stages look like facts.
 *
 * So: one row of stages, one switch, and three buttons for a reminder. No free
 * text — the notes field already exists and this is not another one.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ProgressSheet(
    contact: Contact,
    onSave: (Contact) -> Unit,
    onDismiss: () -> Unit,
) {
    var stage by remember(contact.id) { mutableStateOf(contact.stage) }
    var met by remember(contact.id) { mutableStateOf(contact.met) }
    var meetingAt by remember(contact.id) { mutableStateOf(contact.meetingAt) }
    var followUpAt by remember(contact.id) { mutableStateOf(contact.followUpAt) }

    // **Asked for at the moment a reminder is first set, not on launch.**
    //
    // A permission prompt on first run is a prompt with no context, and the
    // honest answer to it is "no". Here the user has just chosen to be reminded
    // about somebody, so the request answers a question they have already asked.
    // Refusing it still stores the reminder — it shows on the calendar and in
    // the day list — so the feature degrades rather than disappears.
    val askForNotifications = rememberNotificationPermission()

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Column {
                Text(contact.displayName, fontWeight = FontWeight.SemiBold)
                if (contact.company.isNotBlank()) {
                    Text(
                        contact.company,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        },
        text = {
            Column {
                Label("Stage")
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    DealStage.entries.forEach { option ->
                        // The picked chip wears the colour its badge wears in
                        // the calendar. This is the only screen where a stage is
                        // chosen, so it is the only place the scheme can be
                        // learnt — six colours that appear fully formed in a
                        // list and nowhere else are just six colours.
                        val palette = stagePalette(option)
                        FilterChip(
                            selected = stage == option,
                            onClick = { stage = option },
                            label = { Text(option.label) },
                            colors = FilterChipDefaults.filterChipColors(
                                selectedContainerColor = palette.container,
                                selectedLabelColor = palette.ink,
                            ),
                        )
                    }
                }

                Row(
                    Modifier.fillMaxWidth().padding(top = 18.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f)) {
                        Text("Met in person", style = MaterialTheme.typography.bodyLarge)
                        Text(
                            "Rather than a card passed on or taken from a stand.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Switch(checked = met, onCheckedChange = { met = it })
                }

                // **Two reminders, not one with a type.** A contact usually has
                // both at once — a meeting on Thursday and a chase the week
                // after if it does not happen — and a single field would make
                // setting the second one delete the first.
                ReminderRow(
                    heading = "Meeting",
                    caption = "Announces itself when it comes round.",
                    at = meetingAt,
                    offsets = listOf("Tomorrow" to 1, "In 2 days" to 2, "Next week" to 7),
                    onPick = { meetingAt = it; if (it != null) askForNotifications() },
                )
                ReminderRow(
                    heading = "Follow up",
                    caption = "Waits quietly in the notification shade.",
                    at = followUpAt,
                    offsets = listOf("In 3 days" to 3, "Next week" to 7, "In a month" to 30),
                    onPick = { followUpAt = it; if (it != null) askForNotifications() },
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    onSave(
                        contact.copy(
                            stage = stage,
                            met = met,
                            meetingAt = meetingAt,
                            followUpAt = followUpAt,
                            // A reminder that is moved or cleared is no longer
                            // one that was dealt with. Leaving the old "done"
                            // stamp would stop the new date ever coming due —
                            // silently, because the date would look right.
                            meetingDoneAt = if (meetingAt == contact.meetingAt) {
                                contact.meetingDoneAt
                            } else {
                                null
                            },
                            followUpDoneAt = if (followUpAt == contact.followUpAt) {
                                contact.followUpDoneAt
                            } else {
                                null
                            },
                        ),
                    )
                },
            ) { Text("Save") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
private fun Label(text: String, top: androidx.compose.ui.unit.Dp = 0.dp) {
    Text(
        text = text.uppercase(Locale.getDefault()),
        style = MaterialTheme.typography.labelSmall,
        fontWeight = FontWeight.SemiBold,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = top, bottom = 6.dp),
    )
}

/**
 * Nine in the morning, the given number of days from now.
 *
 * A time of day rather than "now plus N × 24 hours", because a reminder set at
 * half past eleven at night should not fire at half past eleven at night.
 */
internal fun morningIn(days: Int, from: Long = System.currentTimeMillis()): Long =
    Calendar.getInstance().apply {
        timeInMillis = from
        add(Calendar.DAY_OF_YEAR, days)
        set(Calendar.HOUR_OF_DAY, 9)
        set(Calendar.MINUTE, 0)
        set(Calendar.SECOND, 0)
        set(Calendar.MILLISECOND, 0)
    }.timeInMillis

internal fun sameDay(a: Long, b: Long): Boolean = startOfDay(a) == startOfDay(b)

private fun onDate(millis: Long): String =
    java.text.SimpleDateFormat("EEE d MMM", Locale.getDefault()).format(millis)

/**
 * A way to ask for the notification permission, or a no-op where none is needed.
 *
 * Android 12 and earlier post notifications without asking, so the launcher is
 * still registered — a composable cannot conditionally call a remember — but the
 * returned function does nothing on those versions rather than opening a dialog
 * the system would immediately dismiss.
 */
@Composable
private fun rememberNotificationPermission(): () -> Unit {
    val launcher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { /* Refused is survivable: the reminder is still stored and still shown. */ }

    val context = LocalContext.current
    return remember(context) {
        {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                val granted = ContextCompat.checkSelfPermission(
                    context,
                    Manifest.permission.POST_NOTIFICATIONS,
                ) == PackageManager.PERMISSION_GRANTED
                if (!granted) launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
        }
    }
}

/**
 * One reminder: a heading, a row of offsets, and a way to clear it.
 *
 * Offsets rather than a date picker. Standing at a stand, "next week" is a
 * decision and a calendar is a chore — and tapping the chosen one again clears
 * it, so setting a reminder by mistake costs one tap to undo rather than a
 * hunt for a Clear button.
 */
@OptIn(ExperimentalLayoutApi::class, ExperimentalMaterial3Api::class)
@Composable
private fun ReminderRow(
    heading: String,
    caption: String,
    at: Long?,
    offsets: List<Pair<String, Int>>,
    onPick: (Long?) -> Unit,
) {
    Label(heading, top = 18.dp)
    Text(
        caption,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(bottom = 6.dp),
    )
    var picking by remember { mutableStateOf(false) }

    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        offsets.forEach { (label, days) ->
            val on = remember(days) { morningIn(days) }
            val chosen = at?.let { sameDay(it, on) } == true
            FilterChip(
                selected = chosen,
                onClick = { onPick(if (chosen) null else on) },
                label = { Text(label) },
            )
        }
        // **A way out of the offsets.** They cover the common cases and none of
        // the real ones — a meeting is at a time somebody else chose, and no
        // list of "in three days" will ever contain it. Selected when the date
        // set is not one of the offsets, so a custom date is visibly a state
        // rather than a button that does nothing the second time.
        val isCustom = at != null && offsets.none { (_, days) -> sameDay(at, morningIn(days)) }
        FilterChip(
            selected = isCustom,
            onClick = { picking = true },
            leadingIcon = { Icon(Icons.Filled.Event, contentDescription = null) },
            label = { Text(if (isCustom) onDate(at) else "Pick a date") },
        )
    }

    if (picking) {
        DateAndTimePicker(
            initial = at,
            onPicked = { picking = false; onPick(it) },
            onDismiss = { picking = false },
        )
    }
    if (at != null) {
        AssistChip(
            onClick = { onPick(null) },
            label = { Text("Clear ${onDate(at)}") },
            colors = AssistChipDefaults.assistChipColors(
                labelColor = MaterialTheme.colorScheme.onSurfaceVariant,
            ),
            modifier = Modifier.padding(top = 8.dp),
        )
    }
}

/**
 * A date and then a time, for a reminder the offsets do not cover.
 *
 * **Both, not just the date.** A meeting alert set for nine in the morning when
 * the meeting is at three is worse than no alert: it arrives, gets dismissed as
 * noise, and is gone by the time it mattered. The time defaults to nine so the
 * second step can be accepted without thought when it genuinely does not matter.
 *
 * Two dialogs in sequence rather than one combined control, because Material
 * ships the two separately and a hand-rolled combination of them is a great deal
 * of surface for a screen somebody sees once a fortnight.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DateAndTimePicker(
    initial: Long?,
    onPicked: (Long) -> Unit,
    onDismiss: () -> Unit,
) {
    var chosenDate by remember { mutableStateOf<Long?>(null) }

    val start = initial ?: morningIn(1)
    val dateState = rememberDatePickerState(initialSelectedDateMillis = start)

    if (chosenDate == null) {
        DatePickerDialog(
            onDismissRequest = onDismiss,
            confirmButton = {
                TextButton(
                    onClick = { chosenDate = dateState.selectedDateMillis ?: start },
                    enabled = dateState.selectedDateMillis != null,
                ) { Text("Next") }
            },
            dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
        ) {
            DatePicker(state = dateState)
        }
        return
    }

    val existing = Calendar.getInstance().apply { timeInMillis = start }
    val timeState = rememberTimePickerState(
        initialHour = if (initial != null) existing.get(Calendar.HOUR_OF_DAY) else 9,
        initialMinute = if (initial != null) existing.get(Calendar.MINUTE) else 0,
        is24Hour = android.text.format.DateFormat.is24HourFormat(LocalContext.current),
    )

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("At what time?") },
        text = { TimePicker(state = timeState) },
        confirmButton = {
            TextButton(
                onClick = {
                    onPicked(atLocalTime(chosenDate!!, timeState.hour, timeState.minute))
                },
            ) { Text("Set") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/**
 * The day a date picker returned, at a time in the device's own zone.
 *
 * **The picker works in UTC and a reminder does not.** `selectedDateMillis` is
 * midnight *UTC* on the chosen day, so adding a local hour to it directly puts
 * the reminder on the wrong day for anybody far enough from Greenwich — a day
 * early to the west, and the mistake is invisible because the date shown back is
 * computed the same wrong way.
 *
 * So the day is read out in UTC, where it is correct, and rebuilt in the local
 * zone, where it will be read.
 */
internal fun atLocalTime(utcDateMillis: Long, hour: Int, minute: Int): Long {
    val utc = Calendar.getInstance(java.util.TimeZone.getTimeZone("UTC")).apply {
        timeInMillis = utcDateMillis
    }
    return Calendar.getInstance().apply {
        set(Calendar.YEAR, utc.get(Calendar.YEAR))
        set(Calendar.MONTH, utc.get(Calendar.MONTH))
        set(Calendar.DAY_OF_MONTH, utc.get(Calendar.DAY_OF_MONTH))
        set(Calendar.HOUR_OF_DAY, hour)
        set(Calendar.MINUTE, minute)
        set(Calendar.SECOND, 0)
        set(Calendar.MILLISECOND, 0)
    }.timeInMillis
}
