package com.hsilighting.pagify.ui.contacts

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.PhotoCamera
import androidx.compose.material.icons.filled.PhotoLibrary
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FilterChip
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.ContactGroup
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Contacts read off business cards, and the groups they are filed in.
 *
 * The dates carry the same weight as the fields, because the reason this tab
 * exists is to know when a contact was sent on — not only to hold it.
 *
 * ## Two views, and which one opens
 *
 * **Groups** lists what the user made, with Ungrouped at the end; **All** is the
 * flat searchable list. The default is Groups only once a group exists. Somebody
 * who has never made one should not be shown an organisational layer they did not
 * ask for — it would be an empty screen standing between them and their contacts.
 */
@Composable
fun ContactsScreen(
    contacts: List<Contact>,
    groups: List<ContactGroup>,
    /** Contact id to the groups it is in. */
    memberships: Map<Long, List<Long>>,
    importTarget: Long?,
    onSetImportTarget: (Long?) -> Unit,
    onCreateGroup: (String) -> Unit,
    /** What was just scanned and is waiting to be filed, if anything. */
    pendingFilingLabel: String?,
    onFileScanned: (Long) -> Unit,
    onCreateGroupForScan: (String) -> Unit,
    onSkipFiling: () -> Unit,
    onRenameGroup: (ContactGroup, String) -> Unit,
    onDeleteGroup: (ContactGroup) -> Unit,
    onExportGroup: (ContactGroup) -> Unit,
    onRemoveFromGroup: (Contact, Long) -> Unit,
    onScanFromGallery: () -> Unit,
    onScanFromCamera: () -> Unit,
    onExport: (Contact) -> Unit,
    onDelete: (Contact) -> Unit,
    onSaveEdit: (Contact) -> Unit,
    modifier: Modifier = Modifier,
) {
    var query by remember { mutableStateOf("") }
    var open by remember { mutableStateOf<Contact?>(null) }
    var editing by remember { mutableStateOf<Contact?>(null) }
    var openGroup by remember { mutableStateOf<ContactGroup?>(null) }
    var choosingSource by remember { mutableStateOf(false) }
    var pickingTarget by remember { mutableStateOf(false) }
    var creatingGroup by remember { mutableStateOf(false) }
    var namingForScan by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf<ContactGroup?>(null) }

    // Groups only once one exists — see the note on this function.
    var byGroup by remember(groups.isEmpty()) { mutableStateOf(groups.isNotEmpty()) }

    // Drilling into a group is a place the back gesture should leave, not the tab.
    BackHandler(enabled = openGroup != null) { openGroup = null }

    val inGroup = { group: ContactGroup ->
        contacts.filter { memberships[it.id].orEmpty().contains(group.id) }
    }
    val ungrouped = remember(contacts, memberships) {
        contacts.filter { memberships[it.id].isNullOrEmpty() }
    }

    // Search scopes to whatever is on screen: inside a group it searches that
    // group. Matched on `searchable`, which includes the raw recogniser output,
    // so a phone number the parser failed to classify is still findable.
    val pool = openGroup?.let(inGroup) ?: contacts
    val shown = remember(pool, query) {
        if (query.isBlank()) pool
        else pool.filter { it.searchable.contains(query.trim().lowercase()) }
    }

    Box(modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Header(
                group = openGroup,
                onBack = { openGroup = null },
                onExportGroup = { openGroup?.let(onExportGroup) },
                onRenameGroup = { renaming = openGroup },
                onDeleteGroup = {
                    openGroup?.let { group -> openGroup = null; onDeleteGroup(group) }
                },
            )

            if (openGroup == null && groups.isNotEmpty()) {
                Row(
                    Modifier.padding(horizontal = 20.dp, vertical = 4.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    FilterChip(
                        selected = byGroup,
                        onClick = { byGroup = true },
                        label = { Text("Groups") },
                    )
                    FilterChip(
                        selected = !byGroup,
                        onClick = { byGroup = false },
                        label = { Text("All") },
                    )
                }
            }

            // The sticky import target. Set once before a batch, so forty cards
            // after an event do not mean answering the same question forty times.
            if (contacts.isNotEmpty() || groups.isNotEmpty()) {
                AssistChip(
                    onClick = { pickingTarget = true },
                    label = {
                        Text(
                            text = importTarget
                                ?.let { id -> groups.firstOrNull { it.id == id }?.name }
                                ?.let { "Scanning into $it" }
                                ?: "Scanning into no group",
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    },
                    modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp),
                )
            }

            if (pool.isNotEmpty() || query.isNotBlank()) {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    placeholder = {
                        Text(openGroup?.let { "Search ${it.name}…" } ?: "Search contacts…")
                    },
                    singleLine = true,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp, vertical = 4.dp),
                )
            }

            val listPadding = PaddingValues(start = 20.dp, end = 20.dp, top = 8.dp, bottom = 96.dp)

            when {
                contacts.isEmpty() && groups.isEmpty() -> Empty { choosingSource = true }

                // The group list is its own thing rather than a list of contacts,
                // so it gets its own branch.
                openGroup == null && byGroup && query.isBlank() -> LazyColumn(
                    contentPadding = listPadding,
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(groups, key = { it.id }) { group ->
                        GroupRow(
                            name = group.name,
                            count = inGroup(group).size,
                            subtitle = group.lastExportedAt?.let { "sent ${onDay(it)}" },
                            onClick = { openGroup = group },
                        )
                    }
                    if (ungrouped.isNotEmpty()) {
                        item(key = "ungrouped") {
                            GroupRow(
                                name = "Ungrouped",
                                count = ungrouped.size,
                                subtitle = null,
                                onClick = { byGroup = false },
                            )
                        }
                    }
                }

                shown.isEmpty() && query.isNotBlank() ->
                    Message("Nothing matches “${query.trim()}”.")

                shown.isEmpty() -> Message(
                    openGroup?.let { "Nothing in ${it.name} yet." } ?: "No contacts yet.",
                )

                else -> LazyColumn(
                    contentPadding = listPadding,
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(shown, key = { it.id }) { contact ->
                        ContactRow(contact) { open = contact }
                    }
                }
            }
        }

        ExtendedFloatingActionButton(
            onClick = { choosingSource = true },
            icon = { Icon(Icons.Filled.PersonAdd, contentDescription = null) },
            text = { Text("Add a card") },
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .padding(20.dp),
        )
    }

    if (choosingSource) {
        SourceChooser(
            onCamera = { choosingSource = false; onScanFromCamera() },
            onGallery = { choosingSource = false; onScanFromGallery() },
            onDismiss = { choosingSource = false },
        )
    }

    if (pickingTarget) {
        GroupPicker(
            groups = groups,
            selected = importTarget,
            onPick = { onSetImportTarget(it); pickingTarget = false },
            onCreate = { pickingTarget = false; creatingGroup = true },
            onDismiss = { pickingTarget = false },
        )
    }

    if (creatingGroup) {
        GroupNameDialog(
            title = "New group",
            onConfirm = { onCreateGroup(it); creatingGroup = false },
            onDismiss = { creatingGroup = false },
        )
    }

    // Asked after a scan, and only while nothing else is on top of it — naming a
    // new group replaces this rather than stacking a dialog on a dialog.
    if (pendingFilingLabel != null && !namingForScan) {
        FilingPrompt(
            label = pendingFilingLabel,
            groups = groups,
            suggested = importTarget,
            onFile = onFileScanned,
            onCreate = { namingForScan = true },
            onSkip = onSkipFiling,
        )
    }

    if (namingForScan) {
        GroupNameDialog(
            title = "New group",
            onConfirm = { onCreateGroupForScan(it); namingForScan = false },
            // Backing out returns to the filing question rather than dropping it,
            // so a mistaken tap on "New group" does not leave the card unfiled.
            onDismiss = { namingForScan = false },
        )
    }

    renaming?.let { group ->
        GroupNameDialog(
            title = "Rename group",
            initial = group.name,
            confirm = "Rename",
            onConfirm = { name ->
                onRenameGroup(group, name)
                // The header holds its own copy, so re-open it renamed.
                openGroup = group.copy(name = name.trim())
                renaming = null
            },
            onDismiss = { renaming = null },
        )
    }

    open?.let { contact ->
        ContactSheet(
            contact = contact,
            groups = groups.filter { memberships[contact.id].orEmpty().contains(it.id) },
            onEdit = { editing = contact; open = null },
            onExport = { open = null; onExport(contact) },
            onDelete = { open = null; onDelete(contact) },
            onRemoveFromGroup = { onRemoveFromGroup(contact, it) },
            onDismiss = { open = null },
        )
    }

    editing?.let { contact ->
        ContactEditor(
            contact = contact,
            onSave = { onSaveEdit(it); editing = null },
            onDismiss = { editing = null },
        )
    }
}

@Composable
private fun Header(
    group: ContactGroup?,
    onBack: () -> Unit,
    onExportGroup: () -> Unit,
    onRenameGroup: () -> Unit,
    onDeleteGroup: () -> Unit,
) {
    var confirmingDelete by remember { mutableStateOf(false) }

    Row(
        Modifier
            .fillMaxWidth()
            .padding(start = 8.dp, end = 8.dp, top = 12.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (group != null) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "All groups")
            }
        }
        Text(
            text = group?.name ?: "Contacts",
            style = MaterialTheme.typography.headlineMedium,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier
                .weight(1f)
                .padding(start = if (group == null) 12.dp else 0.dp),
        )
        if (group != null) {
            IconButton(onClick = onRenameGroup) {
                Icon(Icons.Filled.Edit, contentDescription = "Rename this group")
            }
            IconButton(onClick = onExportGroup) {
                Icon(Icons.Filled.Share, contentDescription = "Export this group")
            }
            IconButton(onClick = { confirmingDelete = true }) {
                Icon(
                    Icons.Filled.Delete,
                    contentDescription = "Delete this group",
                    tint = MaterialTheme.colorScheme.error,
                )
            }
        }
    }

    if (confirmingDelete && group != null) {
        AlertDialog(
            onDismissRequest = { confirmingDelete = false },
            title = { Text("Delete ${group.name}?") },
            // Said plainly, because "delete the folder" reads like "delete what is
            // in it", and this is the moment somebody decides.
            text = {
                Text(
                    "The group is removed. Its contacts stay in Pagify — they keep " +
                        "any other groups they are in, and move to Ungrouped if " +
                        "this was their only one.",
                )
            },
            confirmButton = {
                TextButton(onClick = { confirmingDelete = false; onDeleteGroup() }) {
                    Text("Delete the group")
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmingDelete = false }) { Text("Keep") }
            },
        )
    }
}

/**
 * Where the photograph comes from.
 *
 * The camera first because at an event that is the one being used, and the gallery
 * for a card photographed earlier or sent by somebody else. Neither route asks for
 * a permission: the system camera app takes the picture, and the photo picker
 * needs no grant.
 */
@Composable
private fun SourceChooser(
    onCamera: () -> Unit,
    onGallery: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add a business card") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                SourceRow(Icons.Filled.PhotoCamera, "Take a photo", onCamera)
                SourceRow(Icons.Filled.PhotoLibrary, "Choose an image", onGallery)
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
private fun SourceRow(icon: ImageVector, label: String, onClick: () -> Unit) {
    Surface(
        color = MaterialTheme.colorScheme.surface,
        shape = MaterialTheme.shapes.small,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
    ) {
        Row(
            Modifier.padding(14.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
            Text(label, style = MaterialTheme.typography.bodyLarge)
        }
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
    groups: List<ContactGroup>,
    onEdit: () -> Unit,
    onExport: () -> Unit,
    onDelete: () -> Unit,
    onRemoveFromGroup: (Long) -> Unit,
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
                Text(
                    contact.displayName,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                IconButton(onClick = onEdit) {
                    Icon(Icons.Filled.Edit, contentDescription = "Edit this contact")
                }
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
                    contact.phones.forEach {
                        Detail(it.kind.replaceFirstChar(Char::uppercase), it.raw)
                    }
                    contact.emails.forEach { Detail("Email", it) }
                    contact.urls.forEach { Detail("Web", it) }
                    Detail("Address", contact.address)
                    Detail("Notes", contact.notes)

                    if (groups.isNotEmpty()) {
                        HorizontalDivider(Modifier.padding(vertical = 4.dp))
                        Text(
                            text = "Groups",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        groups.forEach { group ->
                            AssistChip(
                                onClick = { onRemoveFromGroup(group.id) },
                                label = { Text("${group.name}  ✕") },
                            )
                        }
                    }

                    HorizontalDivider(Modifier.padding(vertical = 4.dp))
                    Detail("Added", onDay(contact.capturedAt))
                    Detail(
                        "Exported",
                        contact.exportedAt?.let {
                            val times = if (contact.exportCount == 1) {
                                "once"
                            } else {
                                "${contact.exportCount} times"
                            }
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
private fun Empty(onAdd: () -> Unit) {
    Column(
        Modifier
            .fillMaxSize()
            .padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("No contacts yet", style = MaterialTheme.typography.titleMedium)
        Text(
            text = "Photograph a business card and Pagify reads it into a contact " +
                "you can export as a vCard — stamped with the date you sent it.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 8.dp, bottom = 20.dp),
        )
        TextButton(onClick = onAdd) { Text("Add a business card") }
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
