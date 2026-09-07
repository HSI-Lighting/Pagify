package com.hsilighting.pagify

import androidx.room.Room
import androidx.test.platform.app.InstrumentationRegistry
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.Meeting
import com.hsilighting.pagify.data.ContactStore
import com.hsilighting.pagify.data.db.ContactsDatabase
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/**
 * A contact can have more than one meeting, and arranging the second must not
 * erase the first.
 *
 * This is the bug the `meetings` table exists for. A meeting used to be a pair
 * of columns on the contact — `meetingAt` and `meetingDoneAt` — so a second
 * appointment overwrote the first with no warning and no trace. Nothing on
 * screen said anything had been lost; the only symptom was not turning up.
 *
 * Through [ContactStore] rather than the DAO, because the overwrite happened in
 * the layer above the SQL: the screen handed down a whole contact carrying one
 * meeting, and saving it replaced whatever was there.
 */
class ManyMeetingsTest {

    private lateinit var database: ContactsDatabase
    private lateinit var store: ContactStore

    private val hour = 3_600_000L
    private val now = System.currentTimeMillis()

    @Before
    fun open() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        database = Room.inMemoryDatabaseBuilder(context, ContactsDatabase::class.java)
            .allowMainThreadQueries()
            .build()
        store = ContactStore(database)
    }

    @After
    fun close() = database.close()

    /** What is stored for a contact, soonest first. */
    private fun meetingsOf(id: Long): List<Meeting> = runBlocking {
        store.contacts.first().single { it.id == id }.meetings.sortedBy { it.at }
    }

    /** The reported bug, exactly. */
    @Test
    fun arranging_a_second_meeting_keeps_the_first() = runBlocking {
        val owen = Contact(id = 1, name = "Owen Hart")
        store.saveProgress(owen.copy(meetings = listOf(Meeting(contactId = 1, at = now + hour))))

        // As the screen does it: read what is there, add to it, save the lot.
        val carrying = store.contacts.first().single { it.id == 1L }
        store.saveProgress(
            carrying.copy(meetings = carrying.meetings + Meeting(contactId = 1, at = now + 5 * hour)),
        )

        val stored = meetingsOf(1)
        assertEquals("the first meeting was lost", 2, stored.size)
        assertEquals(listOf(now + hour, now + 5 * hour), stored.map { it.at })
    }

    /** Each one gets its own id, which is what the notifications key off. */
    @Test
    fun each_meeting_is_its_own_record() = runBlocking {
        store.saveProgress(
            Contact(id = 2, name = "Priya Raman").copy(
                meetings = listOf(
                    Meeting(contactId = 2, at = now + hour),
                    Meeting(contactId = 2, at = now + 2 * hour),
                ),
            ),
        )

        val ids = meetingsOf(2).map { it.id }
        assertEquals(2, ids.size)
        assertEquals("two meetings sharing an id would collapse in the shade", 2, ids.toSet().size)
        assertTrue("an unsaved meeting kept its placeholder id", ids.none { it == 0L })
    }

    /** Two on the same day are two, not one. That is the calendar's version. */
    @Test
    fun two_on_one_day_both_survive() = runBlocking {
        store.saveProgress(
            Contact(id = 3, name = "Busy Tuesday").copy(
                meetings = listOf(
                    Meeting(contactId = 3, at = now + hour),
                    Meeting(contactId = 3, at = now + 2 * hour),
                ),
            ),
        )
        assertEquals(2, meetingsOf(3).size)
    }

    /** Cancelling one leaves the others alone. */
    @Test
    fun cancelling_one_leaves_the_rest() = runBlocking {
        store.saveProgress(
            Contact(id = 4, name = "Three Dates").copy(
                meetings = listOf(
                    Meeting(contactId = 4, at = now + hour),
                    Meeting(contactId = 4, at = now + 2 * hour),
                    Meeting(contactId = 4, at = now + 3 * hour),
                ),
            ),
        )
        val carrying = store.contacts.first().single { it.id == 4L }
        val middle = carrying.meetings.sortedBy { it.at }[1]

        store.saveProgress(
            carrying.copy(meetings = carrying.meetings.filterNot { it.id == middle.id }),
        )

        assertEquals(listOf(now + hour, now + 3 * hour), meetingsOf(4).map { it.at })
    }

    /**
     * Cancelling the **last** one actually cancels it.
     *
     * The awkward case: an empty list has to mean "none left" here, while the
     * same empty list on a freshly scanned card means "never loaded". Getting
     * that backwards either refuses to cancel the final meeting or wipes the
     * diary of every contact that comes off the camera — which is why the two
     * are separate calls rather than one clever one.
     */
    @Test
    fun cancelling_the_last_one_empties_it() = runBlocking {
        store.saveProgress(
            Contact(id = 5, name = "One Date").copy(
                meetings = listOf(Meeting(contactId = 5, at = now + hour)),
            ),
        )
        val carrying = store.contacts.first().single { it.id == 5L }

        store.saveProgress(carrying.copy(meetings = emptyList()))

        assertEquals(emptyList<Long>(), meetingsOf(5).map { it.at })
    }

    /**
     * An ordinary save leaves meetings alone.
     *
     * The other half of the same rule. A card off the camera, an edit in the
     * field editor and a vCard import all build a `Contact` without ever
     * loading its meetings; if that empty list were taken at face value, saving
     * a phone number would cancel the diary.
     */
    @Test
    fun an_ordinary_save_does_not_cancel_anything() = runBlocking {
        store.saveProgress(
            Contact(id = 6, name = "Edited Later").copy(
                meetings = listOf(Meeting(contactId = 6, at = now + hour)),
            ),
        )

        // As an edit arrives: a contact rebuilt from fields, carrying no meetings.
        store.save(Contact(id = 6, name = "Edited Later", title = "Buyer"))

        assertEquals("editing a contact cancelled its meeting", 1, meetingsOf(6).size)
    }

    /** Deleting the contact takes the meetings with it — a meeting needs somebody. */
    @Test
    fun deleting_the_contact_takes_its_meetings() = runBlocking {
        store.saveProgress(
            Contact(id = 7, name = "Gone").copy(
                meetings = listOf(Meeting(contactId = 7, at = now + hour)),
            ),
        )
        store.delete(7)

        assertEquals(emptyList<Long>(), database.contacts().meetingsOf(7).map { it.at })
    }
}
