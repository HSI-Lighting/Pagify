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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.FilterChip
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
    var reminderAt by remember(contact.id) { mutableStateOf(contact.reminderAt) }

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
                        FilterChip(
                            selected = stage == option,
                            onClick = { stage = option },
                            label = { Text(option.label) },
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

                Label("Remind me", top = 18.dp)
                // Offsets rather than a date picker. Standing at a stand, "next
                // week" is a decision and a calendar is a chore — and the date
                // is editable afterwards from the calendar itself.
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    listOf(
                        "Tomorrow" to 1,
                        "In 3 days" to 3,
                        "Next week" to 7,
                        "In a month" to 30,
                    ).forEach { (label, days) ->
                        val at = remember(days) { morningIn(days) }
                        FilterChip(
                            selected = reminderAt?.let { sameDay(it, at) } == true,
                            onClick = {
                                val already = reminderAt?.let { sameDay(it, at) } == true
                                reminderAt = if (already) null else at
                                if (!already) askForNotifications()
                            },
                            label = { Text(label) },
                        )
                    }
                }
                val set = reminderAt
                if (set != null) {
                    AssistChip(
                        onClick = { reminderAt = null },
                        label = { Text("Clear ${onDate(set)}") },
                        colors = AssistChipDefaults.assistChipColors(
                            labelColor = MaterialTheme.colorScheme.onSurfaceVariant,
                        ),
                        modifier = Modifier.padding(top = 8.dp),
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    onSave(
                        contact.copy(
                            stage = stage,
                            met = met,
                            reminderAt = reminderAt,
                            // A reminder that is moved or cleared is no longer
                            // one that was dealt with; leaving the old
                            // "done" stamp would keep the new date from ever
                            // coming due.
                            reminderDoneAt = if (reminderAt == contact.reminderAt) {
                                contact.reminderDoneAt
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
