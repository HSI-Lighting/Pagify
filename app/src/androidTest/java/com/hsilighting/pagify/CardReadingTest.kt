package com.hsilighting.pagify

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.hsilighting.pagify.core.cardReadingFrom
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * What the review screen is handed.
 *
 * The regions cross from Rust as JSON, and a highlight drawn from a region that
 * arrived wrong is worse than no highlight: it points confidently at the wrong
 * line, on a screen whose whole purpose is checking the reading against the card.
 *
 * Instrumented, because it parses with `org.json` and the JVM and Android
 * implementations disagree about nulls — see [ContactJsonTest].
 */
@RunWith(AndroidJUnit4::class)
class CardReadingTest {

    private val printed = """
        {
          "name": {"value":"Yaseen Anwar","confidence":0.55,
                   "region":{"left":60,"top":70,"right":284,"bottom":104}},
          "title": {"value":"Design Engineer","confidence":0.55,
                    "region":{"left":60,"top":118,"right":225,"bottom":138}},
          "company": {"value":"HSI Lighting LLC","confidence":0.55,
                      "region":{"left":60,"top":165,"right":271,"bottom":189}},
          "phones": [{"raw":"050 123 4567","normalised":"0501234567","kind":"cell",
                      "confidence":0.9,
                      "region":{"left":60,"top":400,"right":227,"bottom":416}}],
          "emails": [], "urls": [], "rawText": "Yaseen Anwar"
        }
    """.trimIndent()

    @Test
    fun theFourFieldsComeThroughWithWhereTheyWereRead() {
        val reading = cardReadingFrom(printed, id = 1)

        assertEquals(
            listOf("Name", "Designation", "Phone", "Company"),
            reading.highlights.map { it.label },
        )
        assertEquals("Yaseen Anwar", reading.highlights[0].value)
        assertEquals("050 123 4567", reading.highlights[2].value)

        val name = reading.highlights[0].region!!
        assertEquals(60f, name.left)
        assertEquals(70f, name.top)
        assertEquals(34f, name.height)
    }

    /** And the contact saved is the same one the review showed. */
    @Test
    fun theContactMatchesWhatWasHighlighted() {
        val reading = cardReadingFrom(printed, id = 7)
        assertEquals(7L, reading.contact.id)
        assertEquals("Yaseen Anwar", reading.contact.name)
        assertEquals("HSI Lighting LLC", reading.contact.company)
    }

    @Test
    fun aPrintedCardIsWorthReviewing() {
        assertTrue(cardReadingFrom(printed, id = 1).worthReviewing)
    }

    /**
     * A card from a QR is exact and has no regions, so there is nothing to check
     * and nowhere to draw. Reviewing it would be asking somebody to confirm a
     * value that cannot be wrong.
     */
    @Test
    fun aQrCardIsNotWorthReviewing() {
        val fromQr = """
            {"name":{"value":"Jane Okafor","confidence":1.0},
             "phones":[{"raw":"+441234567890","normalised":"+441234567890",
                        "kind":"work","confidence":1.0}],
             "emails":[],"urls":[],"rawText":""}
        """.trimIndent()

        val reading = cardReadingFrom(fromQr, id = 1)
        assertFalse("a QR card was sent to be checked", reading.worthReviewing)
        assertNull(reading.highlights.first().region)
        // The values are still there — it is the pointing that is impossible.
        assertEquals("Jane Okafor", reading.highlights.first().value)
    }

    /** A card the parser could make nothing of does not crash the review. */
    @Test
    fun anEmptyCardHasNothingToShow() {
        val reading = cardReadingFrom("""{"phones":[],"emails":[],"urls":[],"rawText":""}""", 1)
        assertTrue(reading.highlights.isEmpty())
        assertFalse(reading.worthReviewing)
    }

    /**
     * Only the first number is highlighted.
     *
     * A card printing a landline, a mobile and a fax would otherwise stack three
     * highlights in the same corner, on top of each other.
     */
    @Test
    fun onlyTheFirstNumberIsHighlighted() {
        val many = """
            {"phones":[
               {"raw":"020 7946 0000","normalised":"","kind":"work","confidence":0.9,
                "region":{"left":60,"top":400,"right":227,"bottom":416}},
               {"raw":"020 7946 0001","normalised":"","kind":"fax","confidence":0.9,
                "region":{"left":60,"top":420,"right":227,"bottom":436}}],
             "emails":[],"urls":[],"rawText":""}
        """.trimIndent()

        val reading = cardReadingFrom(many, id = 1)
        assertEquals(1, reading.highlights.count { it.label == "Phone" })
        assertEquals("020 7946 0000", reading.highlights.single().value)
    }
}
