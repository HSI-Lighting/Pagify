package com.hsilighting.pagify.core

import org.json.JSONObject

/** A rectangle on the photograph, in the photograph's own pixels. */
data class CardRegion(
    val left: Float,
    val top: Float,
    val right: Float,
    val bottom: Float,
) {
    val width: Float get() = right - left
    val height: Float get() = bottom - top
}

/**
 * What a line on the card turned out to be.
 *
 * Kept as a kind rather than a label so a corrected value goes back into the
 * right part of the contact. Editing "Design Enginer" to "Design Engineer" must
 * put it in the title; matching on the displayed word would break the moment the
 * wording changed.
 */
enum class CardFieldKind(val label: String) {
    NAME("Name"),
    TITLE("Designation"),
    COMPANY("Company"),
    PHONE("Phone"),
    EMAIL("Email"),
    WEBSITE("Website"),
    ADDRESS("Address"),
    NOTES("Other text"),
}

/** One value the engine read, and where on the card it read it. */
data class ReadField(
    val kind: CardFieldKind,
    val value: String,
    /** Null when it came from a QR code: there is nowhere to point at. */
    val region: CardRegion?,
    /** work / cell / fax / home, for a phone. */
    val phoneKind: String? = null,
    /**
     * The engine's dialable form, dropped the moment a human edits the number.
     *
     * A normalisation derived from the old digits would export a `TEL` that
     * disagrees with what is on screen.
     */
    val normalised: String? = null,
)

/**
 * Everything one card in a photograph was read as, ready to be checked.
 *
 * **All of it**, not a chosen four. The review is where a misreading is caught,
 * and a field that is not shown cannot be corrected — it goes into the contact
 * unseen. The four that matter most simply come first.
 */
data class CardReading(
    /** The base the corrected contact is built on: id, capture date, raw text. */
    val contact: Contact,
    val fields: List<ReadField>,
) {
    /**
     * Whether there is anything to point at.
     *
     * A card read from a QR is exact and has no regions, so there is nothing to
     * check and no place to draw. Reviewing it would be asking somebody to
     * confirm a value that cannot be wrong.
     */
    val worthReviewing: Boolean get() = fields.any { it.region != null }
}

/**
 * The contact these fields describe.
 *
 * Rebuilt from whatever survived the review rather than patched, so a field
 * swiped away is genuinely absent from what is saved — not merely hidden.
 * `rawText` is untouched: it is the evidence of what the recogniser saw, and a
 * value removed here is still findable by search afterwards.
 */
fun CardReading.contactFrom(kept: List<ReadField>): Contact = contact.copy(
    name = kept.firstOrNull { it.kind == CardFieldKind.NAME }?.value.orEmpty(),
    title = kept.firstOrNull { it.kind == CardFieldKind.TITLE }?.value.orEmpty(),
    company = kept.firstOrNull { it.kind == CardFieldKind.COMPANY }?.value.orEmpty(),
    address = kept.firstOrNull { it.kind == CardFieldKind.ADDRESS }?.value.orEmpty(),
    notes = kept.firstOrNull { it.kind == CardFieldKind.NOTES }?.value.orEmpty(),
    phones = kept.filter { it.kind == CardFieldKind.PHONE }.map {
        Phone(
            raw = it.value,
            normalised = it.normalised.orEmpty(),
            kind = it.phoneKind ?: "work",
        )
    },
    emails = kept.filter { it.kind == CardFieldKind.EMAIL }.map { it.value },
    urls = kept.filter { it.kind == CardFieldKind.WEBSITE }.map { it.value },
)

/**
 * Read a card the engine produced, keeping the regions.
 *
 * Shares [contactFromCardJson] for the parts a review never touches — the id,
 * the capture date, the raw text — so the thing reviewed and the thing saved
 * cannot describe different cards.
 */
fun cardReadingFrom(json: String, id: Long): CardReading {
    val card = JSONObject(json)
    val contact = contactFromCardJson(json, id)

    // Ordered as somebody checks a card: who, what they do, who for, then how to
    // reach them. The first four are what the eye goes to.
    val fields = buildList {
        card.field(CardFieldKind.NAME)?.let(::add)
        card.field(CardFieldKind.TITLE)?.let(::add)
        card.field(CardFieldKind.COMPANY)?.let(::add)

        card.optJSONArray("phones")?.let { phones ->
            for (index in 0 until phones.length()) {
                val phone = phones.optJSONObject(index) ?: continue
                val value = phone.stringOr("raw").ifBlank { phone.stringOr("normalised") }
                if (value.isBlank()) continue
                add(
                    ReadField(
                        kind = CardFieldKind.PHONE,
                        value = value,
                        region = phone.region(),
                        phoneKind = phone.stringOr("kind").ifBlank { "work" },
                        normalised = phone.stringOr("normalised"),
                    ),
                )
            }
        }

        card.values("emails", CardFieldKind.EMAIL).forEach(::add)
        card.values("urls", CardFieldKind.WEBSITE).forEach(::add)
        card.field(CardFieldKind.ADDRESS)?.let(::add)

        // Whatever no rule claimed. Shown so it can be deleted or corrected
        // rather than arriving in the contact unseen.
        card.stringOr("notes").takeIf { it.isNotBlank() }?.let {
            add(ReadField(CardFieldKind.NOTES, it, region = null))
        }
    }

    return CardReading(contact, fields)
}

private fun JSONObject.field(kind: CardFieldKind): ReadField? {
    val key = when (kind) {
        CardFieldKind.NAME -> "name"
        CardFieldKind.TITLE -> "title"
        CardFieldKind.COMPANY -> "company"
        CardFieldKind.ADDRESS -> "address"
        else -> return null
    }
    val holder = optJSONObject(key) ?: return null
    val value = holder.stringOr("value")
    if (value.isBlank()) return null
    return ReadField(kind, value, holder.region())
}

private fun JSONObject.values(key: String, kind: CardFieldKind): List<ReadField> {
    val array = optJSONArray(key) ?: return emptyList()
    return (0 until array.length()).mapNotNull { index ->
        val holder = array.optJSONObject(index) ?: return@mapNotNull null
        val value = holder.stringOr("value")
        if (value.isBlank()) null else ReadField(kind, value, holder.region())
    }
}

/** `optString` gives the four characters "null" for a JSON null. See [Contact]. */
private fun JSONObject.stringOr(key: String): String =
    if (isNull(key)) "" else optString(key)

private fun JSONObject.region(): CardRegion? {
    val region = optJSONObject("region") ?: return null
    return CardRegion(
        left = region.optDouble("left", 0.0).toFloat(),
        top = region.optDouble("top", 0.0).toFloat(),
        right = region.optDouble("right", 0.0).toFloat(),
        bottom = region.optDouble("bottom", 0.0).toFloat(),
    ).takeIf { it.width > 0f && it.height > 0f }
}
