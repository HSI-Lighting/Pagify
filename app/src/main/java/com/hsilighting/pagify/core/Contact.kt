package com.hsilighting.pagify.core

import org.json.JSONArray
import org.json.JSONObject

/**
 * A contact read off a business card.
 *
 * The dates are the point of the feature. `capturedAt` answers "when did I meet
 * this person"; **`exportedAt` answers "when did I send their details on"**, and
 * it is why this exists at all. It is also written into the exported file itself
 * as the vCard `REV` property, so a contact still says when it left the app after
 * it has left the app.
 *
 * Every recognised value carries a confidence. Nothing is dropped for being
 * uncertain — a field thrown away cannot be corrected, and a field flagged as
 * doubtful can. [rawText] holds everything the recogniser produced, including
 * whatever the parser could not classify.
 */
data class Contact(
    val id: Long,
    val name: String = "",
    val title: String = "",
    val company: String = "",
    val address: String = "",
    val notes: String = "",
    val phones: List<Phone> = emptyList(),
    val emails: List<String> = emptyList(),
    val urls: List<String> = emptyList(),
    /** Everything the recogniser saw, never discarded. */
    val rawText: String = "",
    /** The rectified card, if one was kept. */
    val cardImagePath: String? = null,
    val capturedAt: Long = System.currentTimeMillis(),
    /** Null until it has been exported once. */
    val exportedAt: Long? = null,
    val exportCount: Int = 0,
) {
    /** What the list shows when there is no name — a shop or a hotline has none. */
    val displayName: String
        get() = name.ifBlank { company }.ifBlank { "Untitled contact" }

    /**
     * Everything worth searching, as one string.
     *
     * Includes [rawText], so a phone number the parser failed to classify is
     * still findable. That is the whole reason the raw text is kept.
     */
    val searchable: String
        get() = listOf(
            name, title, company, address, notes, rawText,
            phones.joinToString(" ") { it.raw },
            emails.joinToString(" "),
            urls.joinToString(" "),
        ).joinToString(" ").lowercase()
}

data class Phone(
    val raw: String,
    val normalised: String = raw,
    val kind: String = "work",
)

// ------------------------------------------------------------- the engine --

/**
 * The shape `pdf_core::contacts::BusinessCard` serialises to.
 *
 * Written by hand rather than with a serialisation library because it is one
 * object crossing one boundary, and the app already talks to the engine in
 * hand-built JSON everywhere else.
 */
fun Contact.toCardJson(): JSONObject = JSONObject().apply {
    if (name.isNotBlank()) put("name", field(name))
    if (title.isNotBlank()) put("title", field(title))
    if (company.isNotBlank()) put("company", field(company))
    if (address.isNotBlank()) put("address", field(address))
    if (notes.isNotBlank()) put("notes", notes)
    put("rawText", rawText)
    put(
        "phones",
        JSONArray(
            phones.map {
                JSONObject().apply {
                    put("raw", it.raw)
                    put("normalised", it.normalised)
                    put("kind", it.kind)
                    put("confidence", 1.0)
                }
            },
        ),
    )
    put("emails", JSONArray(emails.map { field(it) }))
    put("urls", JSONArray(urls.map { field(it) }))
}

private fun field(value: String) = JSONObject().apply {
    put("value", value)
    // Edited by hand, or read from a QR: either way it is not a guess.
    put("confidence", 1.0)
}

/** Read a card the engine produced — from a QR, or later from OCR. */
fun contactFromCardJson(json: String, id: Long): Contact {
    val card = JSONObject(json)
    return Contact(
        id = id,
        name = card.optJSONObject("name")?.optString("value").orEmpty(),
        title = card.optJSONObject("title")?.optString("value").orEmpty(),
        company = card.optJSONObject("company")?.optString("value").orEmpty(),
        address = card.optJSONObject("address")?.optString("value").orEmpty(),
        notes = card.optString("notes"),
        phones = card.optJSONArray("phones").objects().map {
            Phone(
                raw = it.optString("raw"),
                normalised = it.optString("normalised"),
                kind = it.optString("kind", "work"),
            )
        },
        emails = card.optJSONArray("emails").objects().map { it.optString("value") },
        urls = card.optJSONArray("urls").objects().map { it.optString("value") },
        rawText = card.optString("rawText"),
    )
}

// ------------------------------------------------------------- persistence --

fun List<Contact>.toStoreJson(): String = JSONArray(
    map { contact ->
        JSONObject().apply {
            put("id", contact.id)
            put("name", contact.name)
            put("title", contact.title)
            put("company", contact.company)
            put("address", contact.address)
            put("notes", contact.notes)
            put("rawText", contact.rawText)
            contact.cardImagePath?.let { put("cardImagePath", it) }
            put("capturedAt", contact.capturedAt)
            contact.exportedAt?.let { put("exportedAt", it) }
            put("exportCount", contact.exportCount)
            put(
                "phones",
                JSONArray(
                    contact.phones.map {
                        JSONObject().apply {
                            put("raw", it.raw)
                            put("normalised", it.normalised)
                            put("kind", it.kind)
                        }
                    },
                ),
            )
            put("emails", JSONArray(contact.emails))
            put("urls", JSONArray(contact.urls))
        }
    },
).toString()

fun contactsFromStoreJson(json: String): List<Contact> {
    val array = JSONArray(json)
    return (0 until array.length()).mapNotNull { index ->
        runCatching {
            val row = array.getJSONObject(index)
            Contact(
                id = row.getLong("id"),
                name = row.optString("name"),
                title = row.optString("title"),
                company = row.optString("company"),
                address = row.optString("address"),
                notes = row.optString("notes"),
                rawText = row.optString("rawText"),
                cardImagePath = row.optString("cardImagePath").ifBlank { null },
                capturedAt = row.optLong("capturedAt", System.currentTimeMillis()),
                // Absent means never exported, which is different from exported
                // at the epoch — so it stays null rather than becoming 0.
                exportedAt = if (row.has("exportedAt")) row.getLong("exportedAt") else null,
                exportCount = row.optInt("exportCount"),
                phones = row.optJSONArray("phones").objects().map {
                    Phone(
                        raw = it.optString("raw"),
                        normalised = it.optString("normalised"),
                        kind = it.optString("kind", "work"),
                    )
                },
                emails = row.optJSONArray("emails").strings(),
                urls = row.optJSONArray("urls").strings(),
            )
        }.getOrNull() // One unreadable row must not lose the rest of the list.
    }
}

private fun JSONArray?.objects(): List<JSONObject> =
    if (this == null) emptyList() else (0 until length()).mapNotNull { optJSONObject(it) }

private fun JSONArray?.strings(): List<String> =
    if (this == null) emptyList() else (0 until length()).map { optString(it) }
