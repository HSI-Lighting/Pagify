package com.hsilighting.pagify.ui.contacts

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.HorizontalDivider
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.Contact
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Contacts read off business cards.
 *
 * The dates are given the same weight as the fields, because the reason this tab
 * exists is to know when a contact was sent on — not only to hold it.
 */
@Composable
fun ContactsScreen(
    contacts: List<Contact>,
    onScan: () -> Unit,
    onExport: (Contact) -> Unit,
    onDelete: (Contact) -> Unit,
    modifier: Modifier = Modifier,
) {
    var query by remember { mutableStateOf("") }
    var open by remember { mutableStateOf<Contact?>(null) }

    // Filtered on `searchable`, which includes the raw recogniser output — so a
    // phone number the parser failed to classify is still findable. That is the
    // whole reason the raw text is kept.
    val shown = remember(contacts, query) {
        if (query.isBlank()) {
            contacts
        } else {
            val needle = query.trim().lowercase()
            contacts.filter { it.searchable.contains(needle) }
        }
    }

    Box(modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Text(
                text = "Contacts",
                style = MaterialTheme.typography.headlineMedium,
                modifier = Modifier.padding(start = 20.dp, top = 16.dp, bottom = 8.dp),
            )

            if (contacts.isNotEmpty()) {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    placeholder = { Text("Search contacts…") },
                    singleLine = true,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp, vertical = 4.dp),
                )
            }

            when {
                contacts.isEmpty() -> Empty(onScan)
                shown.isEmpty() -> Message("Nothing matches “${query.trim()}”.")
                else -> LazyColumn(
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(
                        start = 20.dp,
                        end = 20.dp,
                        top = 8.dp,
                        // Room for the scan button, which floats over the list.
                        bottom = 96.dp,
                    ),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(shown, key = { it.id }) { contact ->
                        ContactRow(contact) { open = contact }
                    }
                }
            }
        }

        ExtendedFloatingActionButton(
            onClick = onScan,
            icon = { Icon(Icons.Filled.PersonAdd, contentDescription = null) },
            text = { Text("Scan a card") },
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .padding(20.dp),
        )
    }

    open?.let { contact ->
        ContactSheet(
            contact = contact,
            onExport = {
                open = null
                onExport(contact)
            },
            onDelete = {
                open = null
                onDelete(contact)
            },
            onDismiss = { open = null },
        )
    }
}

@Composable
private fun ContactRow(contact: Contact, onClick: () -> Unit) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = MaterialTheme.shapes.medium,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
    ) {
        Row(
            Modifier.padding(14.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                Modifier
                    .size(44.dp)
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = contact.displayName.take(1).uppercase(),
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                )
            }

            Column(Modifier.weight(1f)) {
                Text(
                    text = contact.displayName,
                    style = MaterialTheme.typography.titleSmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (contact.company.isNotBlank() && contact.company != contact.displayName) {
                    Text(
                        text = contact.company,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                // The export date, on the row rather than hidden in the detail:
                // it is the thing this feature is for.
                Text(
                    text = contact.exportedAt
                        ?.let { "Exported ${onDay(it)}" }
                        ?: "Not exported",
                    style = MaterialTheme.typography.labelSmall,
                    color = if (contact.exportedAt == null) {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    } else {
                        MaterialTheme.colorScheme.primary
                    },
                )
            }
        }
    }
}

@Composable
private fun ContactSheet(
    contact: Contact,
    onExport: () -> Unit,
    onDelete: () -> Unit,
    onDismiss: () -> Unit,
) {
    var confirmingDelete by remember { mutableStateOf(false) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(contact.displayName, maxLines = 2, overflow = TextOverflow.Ellipsis)
                IconButton(onClick = { confirmingDelete = true }) {
                    Icon(
                        Icons.Filled.Delete,
                        contentDescription = "Delete this contact",
                        tint = MaterialTheme.colorScheme.error,
                    )
                }
            }
        },
        text = {
            // Selectable, so a number or an address can be copied out without
            // exporting the whole contact.
            SelectionContainer {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Detail("Title", contact.title)
                    Detail("Company", contact.company)
                    contact.phones.forEach { Detail(it.kind.replaceFirstChar(Char::uppercase), it.raw) }
                    contact.emails.forEach { Detail("Email", it) }
                    contact.urls.forEach { Detail("Web", it) }
                    Detail("Address", contact.address)
                    Detail("Notes", contact.notes)

                    HorizontalDivider(Modifier.padding(vertical = 4.dp))
                    Detail("Added", onDay(contact.capturedAt))
                    Detail(
                        "Exported",
                        contact.exportedAt?.let {
                            val times = if (contact.exportCount == 1) "once" else "${contact.exportCount} times"
                            "${onDay(it)} · $times"
                        } ?: "never",
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onExport) {
                Icon(Icons.Filled.Share, contentDescription = null)
                Text("Export", Modifier.padding(start = 8.dp))
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Close") } },
    )

    if (confirmingDelete) {
        AlertDialog(
            onDismissRequest = { confirmingDelete = false },
            title = { Text("Delete this contact?") },
            text = {
                Text(
                    "${contact.displayName} will be removed from Pagify. " +
                        "If the card is not still in your pocket, this cannot be undone.",
                )
            },
            confirmButton = {
                TextButton(onClick = { confirmingDelete = false; onDelete() }) {
                    Text("Delete")
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmingDelete = false }) { Text("Keep") }
            },
        )
    }
}

/** One labelled value. Absent fields draw nothing rather than an empty row. */
@Composable
private fun Detail(label: String, value: String) {
    if (value.isBlank()) return
    Column {
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(text = value, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun Empty(onScan: () -> Unit) {
    Column(
        Modifier
            .fillMaxSize()
            .padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("No contacts yet", style = MaterialTheme.typography.titleMedium)
        Text(
            text = "Scan a business card and Pagify reads it into a contact you " +
                "can export as a vCard — stamped with the date you sent it.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 8.dp, bottom = 20.dp),
        )
        TextButton(onClick = onScan) { Text("Scan a business card") }
    }
}

@Composable
private fun Message(text: String) {
    Box(
        Modifier
            .fillMaxSize()
            .padding(32.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = text,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** A date somebody can read, in their own locale. */
private fun onDay(millis: Long): String =
    SimpleDateFormat("d MMM yyyy", Locale.getDefault()).format(Date(millis))
