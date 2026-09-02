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

/** One value the engine read, and where on the card it read it. */
data class ReadField(
    val label: String,
    val value: String,
    /** Null when it came from a QR code: there is nowhere to point at. */
    val region: CardRegion?,
)

/**
 * What one card in a photograph was read as, ready to be checked against it.
 *
 * The review shows **four fields and no more** — the name, what they do, how to
 * ring them, and who they work for. Those are what somebody checks when handed a
 * card, and showing everything would be showing the card again, in worse
 * typography. The rest is on the contact afterwards and can be edited there.
 */
data class CardReading(
    val contact: Contact,
    val highlights: List<ReadField>,
) {
    /**
     * Whether there is anything to point at.
     *
     * A card read from a QR is exact and has no regions, so there is nothing to
     * check and no place to draw. Reviewing it would be asking somebody to
     * confirm a value that cannot be wrong.
     */
    val worthReviewing: Boolean get() = highlights.any { it.region != null }
}

/**
 * Read a card the engine produced, keeping the regions.
 *
 * Shares [contactFromCardJson] for the contact itself rather than parsing twice,
 * so the thing reviewed and the thing saved cannot describe different cards.
 */
fun cardReadingFrom(json: String, id: Long): CardReading {
    val card = JSONObject(json)
    val contact = contactFromCardJson(json, id)

    val highlights = buildList {
        card.field("name", "Name")?.let(::add)
        card.field("title", "Designation")?.let(::add)
        // The first number only. A card with a landline and a fax would otherwise
        // put three highlights on top of each other in the same corner.
        card.optJSONArray("phones")?.optJSONObject(0)?.let { phone ->
            val value = phone.optString("raw").ifBlank { phone.optString("normalised") }
            if (value.isNotBlank()) {
                add(ReadField("Phone", value, phone.region()))
            }
        }
        card.field("company", "Company")?.let(::add)
    }

    return CardReading(contact, highlights)
}

private fun JSONObject.field(key: String, label: String): ReadField? {
    val holder = optJSONObject(key) ?: return null
    val value = holder.optString("value")
    if (value.isBlank()) return null
    return ReadField(label, value, holder.region())
}

private fun JSONObject.region(): CardRegion? {
    val region = optJSONObject("region") ?: return null
    return CardRegion(
        left = region.optDouble("left", 0.0).toFloat(),
        top = region.optDouble("top", 0.0).toFloat(),
        right = region.optDouble("right", 0.0).toFloat(),
        bottom = region.optDouble("bottom", 0.0).toFloat(),
    ).takeIf { it.width > 0f && it.height > 0f }
}
