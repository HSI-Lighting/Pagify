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
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
import androidx.compose.ui.platform.testTag
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
    /**
     * Rename, export and delete, if this row is a real group.
     *
     * Here **as well as** inside the group. They were only in the open group's
     * header, which meant the answer to "how do I delete this group" was to open
     * it first and look at the top — reasonable once you know, and indis-
     * tinguishable from the feature being broken until you do. Ungrouped is not
     * a group and passes none of these, so it shows no menu.
     */
    onRename: (() -> Unit)? = null,
    onExport: (() -> Unit)? = null,
    onDelete: (() -> Unit)? = null,
) {
    var menuOpen by remember { mutableStateOf(false) }

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
            if (onDelete != null || onRename != null || onExport != null) {
                Box {
                    IconButton(onClick = { menuOpen = true }) {
                        Icon(
                            Icons.Filled.MoreVert,
                            contentDescription = "What to do with $name",
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                        onRename?.let { rename ->
                            DropdownMenuItem(
                                text = { Text("Rename") },
                                onClick = { menuOpen = false; rename() },
                            )
                        }
                        onExport?.let { export ->
                            DropdownMenuItem(
                                text = { Text("Export") },
                                onClick = { menuOpen = false; export() },
                            )
                        }
                        onDelete?.let { delete ->
                            DropdownMenuItem(
                                text = {
                                    Text("Delete", color = MaterialTheme.colorScheme.error)
                                },
                                onClick = { menuOpen = false; delete() },
                            )
                        }
                    }
                }
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
 * Where the card that was just read should be filed.
 *
 * Asked **after** the contact is saved, never before. Filing is an aid, so it can
 * never be the thing standing between somebody and a card they photographed —
 * dismissing this, or the app dying while it is on screen, loses nothing.
 *
 * The group used last is offered first and already selected, because at an event
 * the answer is the same forty times running and the second card onwards should
 * cost one tap.
 */
@Composable
fun FilingPrompt(
    label: String,
    groups: List<ContactGroup>,
    suggested: Long?,
    onFile: (Long) -> Unit,
    onCreate: () -> Unit,
    onSkip: () -> Unit,
) {
    // The suggested group first, then the rest in their usual order.
    val ordered = remember(groups, suggested) {
        groups.sortedByDescending { it.id == suggested }
    }

    AlertDialog(
        onDismissRequest = onSkip,
        title = { Text("Add $label to a group?") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (groups.isEmpty()) {
                    Text(
                        "You have no groups yet. One is worth making for an event " +
                            "or a client — it is what makes forty cards findable " +
                            "afterwards.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                } else {
                    ordered.forEach { group ->
                        PickerRow(group.name, group.id == suggested) { onFile(group.id) }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onCreate) { Text("New group") } },
        dismissButton = { TextButton(onClick = onSkip) { Text("Not now") } },
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
            // Tagged because the same group name is on screen twice while this is
            // open — once in the list behind it, once here — and a test that
            // cannot tell them apart cannot test this at all.
            .testTag("pick:$label")
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
