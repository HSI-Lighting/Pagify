package com.hsilighting.pagify.core

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The shape recognised text crosses into the engine in.
 *
 * Nothing else checks it. The engine decodes the array with serde, which rejects
 * the whole thing if one key is wrong, so a single misspelled name makes *every*
 * scan fail in exactly the same way — indistinguishable from recognition simply
 * not working, and nowhere near the line that caused it.
 *
 * The Rust half of this pair is `contacts::parse::tests::
 * the_json_android_sends_decodes`, which takes the same literal and decodes it.
 * Neither test alone proves anything: this one only says Kotlin writes what I
 * believe Rust reads. They meet at the string below, so changing one side breaks
 * the other's test rather than the feature.
 */
class CardScannerTest {

    @Test
    fun `a recognised line carries the field names the engine reads`() {
        val json = CardScanner.segmentJson(60, 70, 300, 104, "Yaseen Anwar")

        assertEquals(60, json.getInt("left"))
        assertEquals(70, json.getInt("top"))
        assertEquals(300, json.getInt("right"))
        assertEquals(104, json.getInt("bottom"))
        assertEquals("Yaseen Anwar", json.getString("text"))
    }

    /** And nothing else, so a stray key cannot go unnoticed. */
    @Test
    fun `a recognised line carries nothing the engine does not read`() {
        val json = CardScanner.segmentJson(60, 70, 300, 104, "Yaseen Anwar")
        assertEquals(
            listOf("bottom", "left", "right", "text", "top"),
            json.keys().asSequence().toList().sorted(),
        )
    }

    /**
     * The literal the Rust test decodes. Kept as an assertion rather than a
     * comment so the two cannot drift apart quietly.
     */
    @Test
    fun `two lines serialise to the array the engine is given`() {
        val array = JSONArray().apply {
            put(CardScanner.segmentJson(60, 70, 300, 104, "Yaseen Anwar"))
            put(CardScanner.segmentJson(60, 118, 280, 138, "HSI Lighting LLC"))
        }

        val decoded = JSONArray(array.toString())
        assertEquals(2, decoded.length())
        assertEquals(
            "HSI Lighting LLC",
            (decoded.get(1) as JSONObject).getString("text"),
        )
    }
}
