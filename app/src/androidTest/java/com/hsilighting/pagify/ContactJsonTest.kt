package com.hsilighting.pagify

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.hsilighting.pagify.core.contactFromCardJson
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Reading a card the engine wrote.
 *
 * The engine serialises an absent optional as JSON `null`, and Android's
 * `optString` returns the four characters **"null"** for that rather than an
 * empty string. Every card with nothing left over showed a Notes field reading
 * "null", and it reached a real screen before anybody noticed — because a string
 * that says "null" is not an error, it is just wrong.
 *
 * **An instrumented test, and it has to be.** The JVM `org.json` used by unit
 * tests returns an empty string for a JSON null; Android's returns "null". Run
 * off-device this passes whether the bug is fixed or not — it did, which is how
 * the difference was noticed.
 */
@RunWith(AndroidJUnit4::class)
class ContactJsonTest {

    @Test
    fun anAbsentNoteDoesNotBecomeTheWordNull() {
        val contact = contactFromCardJson(
            """{"name":{"value":"Jane Okafor","confidence":0.9},"notes":null,"rawText":"Jane"}""",
            id = 1,
        )
        assertEquals("Jane Okafor", contact.name)
        assertEquals("", contact.notes)
    }

    /** The same trap on every other optional the engine can leave out. */
    @Test
    fun absentFieldsAreEmptyRatherThanTheWordNull() {
        val contact = contactFromCardJson(
            """{"name":null,"title":null,"company":null,"address":null,"notes":null,
               "rawText":null,"phones":[],"emails":[],"urls":[]}""",
            id = 1,
        )
        assertEquals("", contact.title)
        assertEquals("", contact.company)
        assertEquals("", contact.address)
        assertEquals("", contact.notes)
        assertEquals("", contact.rawText)
        // And with nothing at all to show, the list still has something to say.
        assertEquals("Untitled contact", contact.displayName)
    }

    /** A phone with no normalised form keeps its printed one and a real kind. */
    @Test
    fun aPhoneWithNullsInItStillReads() {
        val contact = contactFromCardJson(
            """{"phones":[{"raw":"050 123 4567","normalised":null,"kind":null,"confidence":0.9}]}""",
            id = 1,
        )
        assertEquals("050 123 4567", contact.phones.single().raw)
        assertEquals("", contact.phones.single().normalised)
        assertEquals("work", contact.phones.single().kind)
    }
}
