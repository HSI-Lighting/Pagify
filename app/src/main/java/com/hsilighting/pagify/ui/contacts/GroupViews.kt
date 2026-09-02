package com.hsilighting.pagify.ui.contacts

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.ContactGroup

/**
 * One group in the list, with what is in it.
 *
 * The count is on the row because a group's size is the first thing anybody wants
 * to know about it, and the export date is there for the same reason it is on a
 * contact row: knowing when a set of details was sent on is what this feature is
 * for.
 */
@Composable
fun GroupRow(
    name: String,
    count: Int,
    subtitle: String?,
    onClick: () -> Unit,
) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = MaterialTheme.shapes.medium,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
    ) {
        Row(
            Modifier.padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    text = name,
                    style = MaterialTheme.typography.titleSmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = buildString {
                        append(if (count == 1) "1 contact" else "$count contacts")
                        subtitle?.let { append(" · $it") }
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Icon(
                Icons.AutoMirrored.Filled.KeyboardArrowRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * Name a new group, or rename one.
 *
 * No date picker. The plan allows a group to carry an event date, and the schema
 * holds one, but a date field on the create dialog is one more thing to answer
 * before the first card can be scanned — and the group is nearly always being
 * made *at* the event it is named after. It can be added where it costs nothing:
 * on the group itself, later.
 */
@Composable
fun GroupNameDialog(
    title: String,
    initial: String = "",
    confirm: String = "Create",
    onConfirm: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var name by remember { mutableStateOf(initial) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                label = { Text("Name") },
                placeholder = { Text("Light + Building 2026") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
        },
        confirmButton = {
            TextButton(
                onClick = { onConfirm(name) },
                enabled = name.isNotBlank(),
            ) { Text(confirm) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/**
 * Choosing which group cards are filed into, or none.
 *
 * "None" is a first-class option rather than a way of cancelling: filing is an
 * aid, and a contact in no group is an ordinary state, not an unfinished one.
 */
@Composable
fun GroupPicker(
    groups: List<ContactGroup>,
    selected: Long?,
    onPick: (Long?) -> Unit,
    onCreate: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Scan into") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                PickerRow("None", selected == null) { onPick(null) }
                groups.forEach { group ->
                    PickerRow(group.name, selected == group.id) { onPick(group.id) }
                }
            }
        },
        confirmButton = { TextButton(onClick = onCreate) { Text("New group") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Close") } },
    )
}

@Composable
private fun PickerRow(label: String, chosen: Boolean, onClick: () -> Unit) {
    Surface(
        color = if (chosen) {
            MaterialTheme.colorScheme.primaryContainer
        } else {
            MaterialTheme.colorScheme.surface
        },
        shape = MaterialTheme.shapes.small,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
    ) {
        Box(Modifier.padding(horizontal = 12.dp, vertical = 12.dp)) {
            Text(
                text = label,
                style = MaterialTheme.typography.bodyLarge,
                color = if (chosen) {
                    MaterialTheme.colorScheme.onPrimaryContainer
                } else {
                    MaterialTheme.colorScheme.onSurface
                },
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}
