package com.hsilighting.pagify.ui.contacts

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.SnapshotStateList
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.Phone

/**
 * Correcting a contact.
 *
 * The reason this exists is the printed path. A card read from a QR is exact, but
 * one read from its printed text is recognition plus inference — which line is the
 * name, which is the company — and it will sometimes be wrong. Without this the
 * only remedy for a misread digit is to delete the contact and photograph the card
 * again, assuming the card is still in the room.
 *
 * ## What it deliberately does not do
 *
 * **Nothing is validated away.** A phone field accepts whatever is typed, because
 * the person holding the card knows more about it than any pattern does —
 * extensions, in-country formats, "call the office first". A rule that refuses
 * what somebody can see printed in front of them is worse than a wrong guess.
 *
 * **Raw text is never edited.** [Contact.rawText] is the evidence of what the
 * recogniser actually saw and the reason search finds fields the parser missed.
 * Letting it be rewritten to match the corrections would destroy the only record
 * of what the card said.
 *
 * The capture and export dates are not editable either. They are facts about what
 * happened, not fields on a card.
 */
@Composable
fun ContactEditor(
    contact: Contact,
    onSave: (Contact) -> Unit,
    onDismiss: () -> Unit,
) {
    var name by remember { mutableStateOf(contact.name) }
    var title by remember { mutableStateOf(contact.title) }
    var company by remember { mutableStateOf(contact.company) }
    var address by remember { mutableStateOf(contact.address) }
    var notes by remember { mutableStateOf(contact.notes) }

    val phones = remember { contact.phones.toMutableStateList() }
    val emails = remember { contact.emails.toMutableStateList() }
    val urls = remember { contact.urls.toMutableStateList() }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (contact.name.isBlank()) "New contact" else "Edit contact") },
        text = {
            Column(
                Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Field("Name", name) { name = it }
                Field("Job title", title) { title = it }
                Field("Company", company) { company = it }

                HorizontalDivider(Modifier.padding(vertical = 4.dp))

                phones.forEachIndexed { index, phone ->
                    PhoneField(
                        phone = phone,
                        onChange = { phones[index] = it },
                        onRemove = { phones.removeAt(index) },
                    )
                }
                AddButton("Add a phone number") {
                    phones.add(Phone(raw = "", normalised = "", kind = "work"))
                }

                HorizontalDivider(Modifier.padding(vertical = 4.dp))

                RepeatedFields("Email", emails)
                AddButton("Add an email") { emails.add("") }

                RepeatedFields("Website", urls)
                AddButton("Add a website") { urls.add("") }

                HorizontalDivider(Modifier.padding(vertical = 4.dp))

                Field("Address", address, singleLine = false) { address = it }
                Field("Notes", notes, singleLine = false) { notes = it }
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    onSave(
                        contact.copy(
                            name = name.trim(),
                            title = title.trim(),
                            company = company.trim(),
                            address = address.trim(),
                            notes = notes.trim(),
                            // Blank rows are dropped rather than stored: an "Add"
                            // tapped by mistake should not become an empty line in
                            // the exported vCard.
                            phones = phones.filter { it.raw.isNotBlank() }
                                .map { it.copy(raw = it.raw.trim()) },
                            emails = emails.map(String::trim).filter(String::isNotBlank),
                            urls = urls.map(String::trim).filter(String::isNotBlank),
                        ),
                    )
                },
            ) { Text("Save") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
private fun Field(
    label: String,
    value: String,
    singleLine: Boolean = true,
    onChange: (String) -> Unit,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        singleLine = singleLine,
        modifier = Modifier.fillMaxWidth(),
    )
}

/**
 * A phone number and what kind it is.
 *
 * The kind is a menu rather than free text because it is the one part of a phone
 * number with a fixed set of answers — and getting it wrong is what sends
 * somebody to a fax machine.
 *
 * Editing the number clears `normalised`. That field is the engine's E.164 form
 * of what was printed, and once a human has changed the number, a normalisation
 * derived from the old one is worse than none: it would export a `TEL` that
 * disagrees with the number on screen.
 */
@Composable
private fun PhoneField(
    phone: Phone,
    onChange: (Phone) -> Unit,
    onRemove: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }

    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OutlinedTextField(
            value = phone.raw,
            onValueChange = { onChange(phone.copy(raw = it, normalised = "")) },
            label = { Text("Phone") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Phone),
            modifier = Modifier.weight(1f),
        )

        Column {
            AssistChip(
                onClick = { menuOpen = true },
                label = { Text(phone.kind.replaceFirstChar(Char::uppercase)) },
            )
            DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                // The four the engine's PhoneKind can encode. Anything else would
                // be dropped silently on export.
                listOf("work", "cell", "home", "fax").forEach { kind ->
                    DropdownMenuItem(
                        text = { Text(kind.replaceFirstChar(Char::uppercase)) },
                        onClick = {
                            onChange(phone.copy(kind = kind))
                            menuOpen = false
                        },
                    )
                }
            }
        }

        IconButton(onClick = onRemove) {
            Icon(Icons.Filled.Close, contentDescription = "Remove this phone number")
        }
    }
}

@Composable
private fun RepeatedFields(label: String, values: SnapshotStateList<String>) {
    values.forEachIndexed { index, value ->
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OutlinedTextField(
                value = value,
                onValueChange = { values[index] = it },
                label = { Text(label) },
                singleLine = true,
                keyboardOptions = KeyboardOptions(
                    keyboardType = if (label == "Email") KeyboardType.Email else KeyboardType.Uri,
                ),
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { values.removeAt(index) }) {
                Icon(Icons.Filled.Close, contentDescription = "Remove this $label")
            }
        }
    }
}

@Composable
private fun AddButton(label: String, onClick: () -> Unit) {
    TextButton(onClick = onClick) {
        Icon(Icons.Filled.Add, contentDescription = null)
        Text(label, Modifier.padding(start = 8.dp))
    }
}

/** `toMutableStateList` for a list that starts empty as often as not. */
private fun <T> List<T>.toMutableStateList(): SnapshotStateList<T> =
    mutableStateListOf<T>().also { it.addAll(this) }
