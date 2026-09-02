package com.hsilighting.pagify.data.db

import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.ContactGroup
import com.hsilighting.pagify.core.Phone
import org.json.JSONArray
import org.json.JSONObject

/**
 * Between the app's types and the database's rows.
 *
 * Two shapes rather than one annotated class, so the app's `Contact` stays free
 * of Room and the schema stays free of defaults and computed properties. The
 * cost is this file; the benefit is that a schema change and a UI change are
 * never the same edit.
 *
 * Lists are stored as JSON in a column. Separate tables for phones, emails and
 * URLs would be three more joins for data that is only ever read whole, with one
 * card's worth of it at a time.
 */

fun ContactRow.toContact(): Contact = Contact(
    id = id,
    name = name,
    title = title,
    company = company,
    address = address,
    notes = notes,
    rawText = rawText,
    phones = phonesJson.toPhones(),
    emails = emailsJson.toStrings(),
    urls = urlsJson.toStrings(),
    cardImagePath = cardImagePath,
    capturedAt = capturedAt,
    exportedAt = exportedAt,
    exportCount = exportCount,
)

fun Contact.toRow(): ContactRow = ContactRow(
    id = id,
    name = name,
    title = title,
    company = company,
    address = address,
    notes = notes,
    rawText = rawText,
    phonesJson = JSONArray(
        phones.map {
            JSONObject().apply {
                put("raw", it.raw)
                put("normalised", it.normalised)
                put("kind", it.kind)
            }
        },
    ).toString(),
    emailsJson = JSONArray(emails).toString(),
    urlsJson = JSONArray(urls).toString(),
    cardImagePath = cardImagePath,
    capturedAt = capturedAt,
    exportedAt = exportedAt,
    exportCount = exportCount,
)

fun GroupRow.toGroup(): ContactGroup = ContactGroup(
    id = id,
    name = name,
    eventDate = eventDate,
    notes = notes,
    colour = colour,
    createdAt = createdAt,
    lastExportedAt = lastExportedAt,
)

fun ContactGroup.toRow(): GroupRow = GroupRow(
    id = id,
    name = name,
    eventDate = eventDate,
    notes = notes,
    colour = colour,
    createdAt = createdAt,
    lastExportedAt = lastExportedAt,
)

/**
 * A column that will not parse yields an empty list rather than throwing.
 *
 * One malformed column must not make a whole contact unreadable: the name and
 * the raw text are still there, and losing a phone number beats losing the
 * person.
 */
private fun String.toPhones(): List<Phone> = runCatching {
    val array = JSONArray(this)
    (0 until array.length()).mapNotNull { array.optJSONObject(it) }.map {
        Phone(
            raw = it.optString("raw"),
            normalised = it.optString("normalised"),
            kind = it.optString("kind", "work"),
        )
    }
}.getOrDefault(emptyList())

private fun String.toStrings(): List<String> = runCatching {
    val array = JSONArray(this)
    (0 until array.length()).map { array.optString(it) }
}.getOrDefault(emptyList())
