package com.hsilighting.pagify.ui.contacts

import android.content.Intent
import android.util.Log
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
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
    /** The group last filed into — a suggestion for the filing question only. */
    suggestedGroup: Long?,
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
    onAddToGroupPicked: (Contact, Long) -> Unit,
    onCreateGroupWith: (Contact, String) -> Unit,
    /** Given the group being viewed, so a scan from inside one files there. */
    onScanFromGallery: (Long?) -> Unit,
    onScanFromCamera: (Long?) -> Unit,
    onExport: (Contact) -> Unit,
    onDelete: (Contact) -> Unit,
    onSaveEdit: (Contact) -> Unit,
    /** Several at once, picked by long press. */
    onDeleteContacts: (List<Contact>) -> Unit,
    onDeleteGroups: (List<ContactGroup>) -> Unit,
    onExportSelected: (List<Contact>) -> Unit,
    modifier: Modifier = Modifier,
) {
    var query by remember { mutableStateOf("") }
    var open by remember { mutableStateOf<Contact?>(null) }
    var editing by remember { mutableStateOf<Contact?>(null) }
    var openGroup by remember { mutableStateOf<ContactGroup?>(null) }
    var choosingSource by remember { mutableStateOf(false) }
    var creatingGroup by remember { mutableStateOf(false) }
    var namingForScan by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf<ContactGroup?>(null) }
    var addingToGroup by remember { mutableStateOf<Contact?>(null) }
    var namingForContact by remember { mutableStateOf<Contact?>(null) }
    var confirmingDelete by remember { mutableStateOf<ContactGroup?>(null) }

    // Picked by long press, for deleting or exporting several at once. Contacts
    // and groups never mix: the two lists are separate views, and a delete that
    // meant different things to different rows in one selection would be a
    // confirmation nobody could read.
    var pickedContacts by remember { mutableStateOf(emptySet<Long>()) }
    var pickedGroups by remember { mutableStateOf(emptySet<Long>()) }
    var confirmingBulk by remember { mutableStateOf(false) }
    val picking = pickedContacts.isNotEmpty() || pickedGroups.isNotEmpty()
    val clearPicked = { pickedContacts = emptySet(); pickedGroups = emptySet() }

    // Groups only once one exists — see the note on this function.
    var byGroup by remember(groups.isEmpty()) { mutableStateOf(groups.isNotEmpty()) }

    // Back gets out of a selection first, then out of a group. Both before it
    // reaches the tab, and in that order — the innermost thing the user is in.
    BackHandler(enabled = picking) { clearPicked() }
    BackHandler(enabled = !picking && openGroup != null) { openGroup = null }

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
            if (picking) {
                SelectionHeader(
                    count = pickedContacts.size + pickedGroups.size,
                    onClear = clearPicked,
                    onExport = if (pickedContacts.isNotEmpty()) {
                        {
                            onExportSelected(contacts.filter { it.id in pickedContacts })
                            clearPicked()
                        }
                    } else {
                        null
                    },
                    onDelete = { confirmingBulk = true },
                )
            } else {
                Header(
                    group = openGroup,
                    onBack = { openGroup = null },
                    onExportGroup = { openGroup?.let(onExportGroup) },
                    onRenameGroup = { renaming = openGroup },
                    onDeleteGroup = { confirmingDelete = openGroup },
                )
            }

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

            // Makes a group. That is all it does.
            //
            // It used to be a picker for a target that new cards would be filed
            // into, chosen in advance — which asked the same question as the
            // prompt after a scan, in a place nobody looked. Two ways to answer
            // one question, so the one that had to be answered early is gone.
            //
            // **Always drawn**, because it is now the only way to make a group,
            // and it used to be hidden until a contact or a group already
            // existed: the control that creates the first group was unreachable
            // until there was one.
            if (openGroup == null) {
                AssistChip(
                    onClick = { creatingGroup = true },
                    leadingIcon = { Icon(Icons.Filled.Add, contentDescription = null) },
                    label = { Text("Group") },
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
                            onRename = { renaming = group },
                            onExport = { onExportGroup(group) },
                            onDelete = { confirmingDelete = group },
                            selected = group.id in pickedGroups,
                            selecting = picking,
                            onLongPress = {
                                pickedContacts = emptySet()
                                pickedGroups = if (group.id in pickedGroups) {
                                    pickedGroups - group.id
                                } else {
                                    pickedGroups + group.id
                                }
                            },
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
                        ContactRow(
                            contact = contact,
                            selected = contact.id in pickedContacts,
                            selecting = picking,
                            onLongPress = {
                                pickedGroups = emptySet()
                                pickedContacts = if (contact.id in pickedContacts) {
                                    pickedContacts - contact.id
                                } else {
                                    pickedContacts + contact.id
                                }
                            },
                            onClick = { open = contact },
                        )
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
            // The group being viewed goes with the scan, so a card taken from
            // inside a group lands there and is not asked about.
            onCamera = { choosingSource = false; onScanFromCamera(openGroup?.id) },
            onGallery = { choosingSource = false; onScanFromGallery(openGroup?.id) },
            onDismiss = { choosingSource = false },
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
            suggested = suggestedGroup,
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

    if (confirmingBulk) {
        val groupsPicked = pickedGroups.size
        val contactsPicked = pickedContacts.size
        AlertDialog(
            onDismissRequest = { confirmingBulk = false },
            title = {
                Text(
                    if (groupsPicked > 0) {
                        "Delete $groupsPicked ${if (groupsPicked == 1) "group" else "groups"}?"
                    } else {
                        "Delete $contactsPicked ${if (contactsPicked == 1) "contact" else "contacts"}?"
                    },
                )
            },
            text = {
                Text(
                    if (groupsPicked > 0) {
                        "The groups are removed. Their contacts stay in Pagify and " +
                            "move to Ungrouped if they were in no other group."
                    } else {
                        "They will be removed from Pagify. If the cards are not " +
                            "still in your pocket, this cannot be undone."
                    },
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmingBulk = false
                        if (pickedGroups.isNotEmpty()) {
                            onDeleteGroups(groups.filter { it.id in pickedGroups })
                        } else {
                            onDeleteContacts(contacts.filter { it.id in pickedContacts })
                        }
                        clearPicked()
                    },
                ) { Text("Delete") }
            },
            dismissButton = {
                TextButton(onClick = { confirmingBulk = false }) { Text("Keep") }
            },
        )
    }

    confirmingDelete?.let { group ->
        ConfirmGroupDelete(
            group = group,
            onConfirm = {
                confirmingDelete = null
                // Leave the group before deleting it, or the header spends a
                // frame naming something that is gone.
                if (openGroup?.id == group.id) openGroup = null
                onDeleteGroup(group)
            },
            onDismiss = { confirmingDelete = null },
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
            onAddToGroup = { addingToGroup = contact },
            onDismiss = { open = null },
        )
    }

    // Filing a contact that already exists — the direction that was missing, so a
    // contact scanned before any group existed could never be put in one.
    addingToGroup?.let { contact ->
        val alreadyIn = memberships[contact.id].orEmpty()
        val available = groups.filterNot { it.id in alreadyIn }

        FilingPrompt(
            label = contact.displayName,
            groups = available,
            suggested = suggestedGroup?.takeIf { last -> available.any { it.id == last } },
            onFile = { groupId ->
                onAddToGroupPicked(contact, groupId)
                addingToGroup = null
            },
            onCreate = { namingForContact = contact; addingToGroup = null },
            onSkip = { addingToGroup = null },
        )
    }

    namingForContact?.let { contact ->
        GroupNameDialog(
            title = "New group",
            onConfirm = { name ->
                onCreateGroupWith(contact, name)
                namingForContact = null
            },
            onDismiss = { namingForContact = null },
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
            IconButton(onClick = onDeleteGroup) {
                Icon(
                    Icons.Filled.Delete,
                    contentDescription = "Delete this group",
                    tint = MaterialTheme.colorScheme.error,
                )
            }
        }
    }
}

/**
 * What replaces the title while several things are picked.
 *
 * A count rather than a list, and a close that clears rather than a back arrow
 * that navigates: the selection is a mode, and the way out of a mode should undo
 * it rather than move somewhere else.
 */
@Composable
private fun SelectionHeader(
    count: Int,
    onClear: () -> Unit,
    onExport: (() -> Unit)?,
    onDelete: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(start = 8.dp, end = 8.dp, top = 12.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = onClear) {
            Icon(Icons.Filled.Close, contentDescription = "Stop selecting")
        }
        Text(
            text = "$count selected",
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.weight(1f),
        )
        // Only for contacts: a group export is a different thing, done from the
        // group itself, and offering it here would mean two exports with the
        // same icon doing different things.
        onExport?.let { export ->
            IconButton(onClick = export) {
                Icon(Icons.Filled.Share, contentDescription = "Export the selected contacts")
            }
        }
        IconButton(onClick = onDelete) {
            Icon(
                Icons.Filled.Delete,
                contentDescription = "Delete what is selected",
                tint = MaterialTheme.colorScheme.error,
            )
        }
    }
}

/**
 * Confirming a group deletion, wherever it was asked for.
 *
 * One dialog for both routes — the open group's header and the list row's menu —
 * rather than a copy in each. Two confirmations for the same act drift apart,
 * and the wording here is the part that matters most.
 */
@Composable
private fun ConfirmGroupDelete(
    group: ContactGroup,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Delete ${group.name}?") },
        // Said plainly, because "delete the folder" reads like "delete what is in
        // it", and this is the moment somebody decides.
        text = {
            Text(
                "The group is removed. Its contacts stay in Pagify — they keep any " +
                    "other groups they are in, and move to Ungrouped if this was " +
                    "their only one.",
            )
        },
        confirmButton = { TextButton(onClick = onConfirm) { Text("Delete the group") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Keep") } },
    )
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
private fun ContactRow(
    contact: Contact,
    selected: Boolean,
    selecting: Boolean,
    onLongPress: () -> Unit,
    onClick: () -> Unit,
) {
    Surface(
        color = if (selected) {
            MaterialTheme.colorScheme.primaryContainer
        } else {
            MaterialTheme.colorScheme.surfaceVariant
        },
        shape = MaterialTheme.shapes.medium,
        modifier = Modifier
            .fillMaxWidth()
            .combinedClickable(
                onClick = { if (selecting) onLongPress() else onClick() },
                onLongClick = onLongPress,
            ),
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
    onAddToGroup: () -> Unit,
    onDismiss: () -> Unit,
) {
    var confirmingDelete by remember { mutableStateOf(false) }
    val context = LocalContext.current

    // Nothing here is worth crashing over: a device with no dialer, no mail app
    // or no browser is unusual but not broken, and the contact is still on screen
    // to be read or copied.
    val launch: (Intent) -> Unit = { intent ->
        runCatching { context.startActivity(intent) }
            .onFailure { Log.w("ContactsScreen", "nothing could open that", it) }
    }

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

                    contact.phones.forEach { phone ->
                        Detail(phone.kind.replaceFirstChar(Char::uppercase), phone.raw) {
                            // ACTION_DIAL, not ACTION_CALL. Dial opens the keypad
                            // with the number in it and needs no permission;
                            // CALL_PHONE would place the call outright, which is
                            // both a prompt to ask for and a worse answer to a tap.
                            // The dialled form is `normalised` where the engine
                            // could produce one — that is the version with the
                            // spaces and brackets taken out.
                            launch(
                                Intent(
                                    Intent.ACTION_DIAL,
                                    "tel:${phone.normalised.ifBlank { phone.raw }}".toUri(),
                                ),
                            )
                        }
                    }

                    contact.emails.forEach { email ->
                        Detail("Email", email) {
                            launch(Intent(Intent.ACTION_SENDTO, "mailto:$email".toUri()))
                        }
                    }

                    contact.urls.forEach { url ->
                        Detail("Web", url) {
                            // Cards print "www.example.com" far more often than
                            // they print a scheme, and a URL without one opens
                            // nothing at all.
                            val address = if ("://" in url) url else "https://$url"
                            launch(Intent(Intent.ACTION_VIEW, address.toUri()))
                        }
                    }

                    Detail("Address", contact.address)
                    Detail("Notes", contact.notes)

                    HorizontalDivider(Modifier.padding(vertical = 4.dp))
                    Text(
                        text = "Groups",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    // Both directions. Showing only the groups a contact is
                    // already in — with a tap that removes — gave no way to add
                    // one at all, so a contact filed nowhere could never be
                    // filed.
                    groups.forEach { group ->
                        AssistChip(
                            onClick = { onRemoveFromGroup(group.id) },
                            label = { Text("${group.name}  ✕") },
                        )
                    }
                    AssistChip(
                        onClick = onAddToGroup,
                        label = { Text("+ Add to a group") },
                    )

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

/**
 * One labelled value. Absent fields draw nothing rather than an empty row.
 *
 * A value with somewhere to go — a number, an email, a website — is tappable and
 * drawn as a link. The point of having somebody's number in your pocket is
 * ringing them, and making that a copy-and-paste into another app is the kind of
 * small friction that decides whether a feature gets used.
 *
 * Selection still works: the text sits in a `SelectionContainer`, and a long
 * press selects while a tap opens.
 */
@Composable
private fun Detail(label: String, value: String, onOpen: (() -> Unit)? = null) {
    if (value.isBlank()) return
    Column {
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            text = value,
            style = MaterialTheme.typography.bodyMedium,
            color = if (onOpen != null) {
                MaterialTheme.colorScheme.primary
            } else {
                MaterialTheme.colorScheme.onSurface
            },
            textDecoration = if (onOpen != null) TextDecoration.Underline else null,
            modifier = if (onOpen != null) Modifier.clickable(onClick = onOpen) else Modifier,
        )
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
