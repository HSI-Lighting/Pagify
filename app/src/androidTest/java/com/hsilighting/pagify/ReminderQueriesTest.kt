package com.hsilighting.pagify

import androidx.room.Room
import androidx.test.platform.app.InstrumentationRegistry
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.data.db.MeetingRow
import com.hsilighting.pagify.data.db.ContactsDatabase
import com.hsilighting.pagify.data.db.DealStage
import com.hsilighting.pagify.data.db.toRow
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/**
 * Which reminders are due, and which one holds the alarm.
 *
 * **These two queries decide whether a reminder ever arrives.** Only the next
 * one ahead gets an alarm, so if `nextReminder` returns the wrong row the alarm
 * fires at the wrong time; if it returns nothing, nothing is ever scheduled and
 * the failure is complete silence — no crash, no log, just a follow-up that
 * never happened.
 */
class ReminderQueriesTest {

    private lateinit var database: ContactsDatabase

    private val now = 1_700_000_000_000L
    private val hour = 3_600_000L

    @Before
    fun open() {
        database = Room.inMemoryDatabaseBuilder(
            InstrumentationRegistry.getInstrumentation().targetContext,
            ContactsDatabase::class.java,
        ).build()
    }

    @After
    fun close() = database.close()

    private fun save(
        id: Long,
        name: String,
        reminderAt: Long?,
        doneAt: Long? = null,
    ) = runBlocking {
        database.contacts().save(
            Contact(
                id = id,
                name = name,
                capturedAt = now,
                followUpAt = reminderAt,
                followUpDoneAt = doneAt,
            ).toRow(),
        )
    }

    /** Past and undealt-with is due; future, done and absent are not. */
    @Test
    fun only_a_reminder_that_has_passed_and_is_unfinished_is_due() = runBlocking {
        save(1, "Passed", reminderAt = now - hour)
        save(2, "Still ahead", reminderAt = now + hour)
        save(3, "Already dealt with", reminderAt = now - hour, doneAt = now)
        save(4, "No reminder at all", reminderAt = null)

        val due = database.contacts().dueFollowUps(now).map { it.name }
        assertEquals(listOf("Passed"), due)
    }

    /** Exactly now counts as due, which is the boundary an alarm lands on. */
    @Test
    fun a_reminder_falling_exactly_now_is_due() = runBlocking {
        save(1, "On the dot", reminderAt = now)
        assertEquals(1, database.contacts().dueFollowUps(now).size)
    }

    /** Oldest first, so a notification names the one that has waited longest. */
    @Test
    fun the_longest_wait_comes_first() = runBlocking {
        save(1, "Yesterday", reminderAt = now - 24 * hour)
        save(2, "An hour ago", reminderAt = now - hour)
        save(3, "Last week", reminderAt = now - 168 * hour)

        val due = database.contacts().dueFollowUps(now).map { it.name }
        assertEquals(listOf("Last week", "Yesterday", "An hour ago"), due)
    }

    /**
     * The alarm goes to the *soonest* one still ahead.
     *
     * Only one alarm is ever held, so picking the wrong row here means every
     * other reminder waits behind it.
     */
    @Test
    fun the_soonest_reminder_still_ahead_takes_the_alarm() = runBlocking {
        save(1, "Next month", reminderAt = now + 720 * hour)
        save(2, "Tomorrow", reminderAt = now + 24 * hour)
        save(3, "Next week", reminderAt = now + 168 * hour)
        save(4, "Already passed", reminderAt = now - hour)

        assertEquals(now + 24 * hour, database.contacts().nextReminderAt(now))
    }

    /** A finished reminder never takes the alarm, however soon it is. */
    @Test
    fun a_finished_reminder_does_not_take_the_alarm() = runBlocking {
        save(1, "Soon but finished", reminderAt = now + hour, doneAt = now)
        save(2, "Later and open", reminderAt = now + 48 * hour)

        assertEquals(now + 48 * hour, database.contacts().nextReminderAt(now))
    }

    /** Nothing ahead means no alarm, so the pending one is cancelled. */
    @Test
    fun nothing_ahead_means_nothing_to_schedule() = runBlocking {
        save(1, "Passed", reminderAt = now - hour)
        assertNull(database.contacts().nextReminderAt(now))
    }

    /** The model's own test of the same thing, without a database. */
    @Test
    fun the_contact_agrees_about_what_is_due() {
        assertTrue(Contact(id = 1, followUpAt = now - hour).followUpIsDue(now))
        assertFalse(Contact(id = 1, followUpAt = now + hour).followUpIsDue(now))
        assertFalse(Contact(id = 1, followUpAt = now - hour, followUpDoneAt = now).followUpIsDue(now))
        assertFalse(Contact(id = 1, followUpAt = null).followUpIsDue(now))
    }

    /**
     * A meeting and a follow-up are separate, and both reach the alarm.
     *
     * The whole reason for two columns: setting one must not clear the other,
     * and whichever is sooner is the one the single alarm goes to.
     */
    @Test
    fun a_meeting_and_a_follow_up_live_side_by_side() = runBlocking {
        database.contacts().save(Contact(id = 5, name = "Both", followUpAt = now + 100 * hour).toRow())
        database.contacts().addMeeting(MeetingRow(contactId = 5, at = now + 2 * hour))

        val back = database.contacts().meetingsOf(5).single()
        assertEquals(now + 2 * hour, back.at)

        // The sooner of the two takes the alarm, whichever kind it is.
        assertEquals(now + 2 * hour, database.contacts().nextReminderAt(now))
    }

    /** A due meeting shows up in the same sweep as a due follow-up. */
    @Test
    fun a_due_meeting_is_found_too() = runBlocking {
        database.contacts().save(Contact(id = 6, name = "Meeting only").toRow())
        database.contacts().addMeeting(MeetingRow(contactId = 6, at = now - hour))
        save(7, "Follow-up only", reminderAt = now - 2 * hour)

        // Two sweeps now, because they live in two tables — a meeting is no
        // longer a column on the contact.
        assertEquals(listOf("Follow-up only"), database.contacts().dueFollowUps(now).map { it.name })
        assertEquals(listOf(now - hour), database.contacts().dueMeetings(now).map { it.at })
    }
    /** Stage and met survive a round trip through the database. */
    @Test
    fun the_stage_and_the_met_flag_are_stored() = runBlocking {
        database.contacts().save(
            Contact(id = 9, name = "Owen", stage = DealStage.Quoted, met = true).toRow(),
        )
        val back = database.contacts().contactsById(listOf(9)).single()
        assertEquals(DealStage.Quoted.stored, back.stage)
        assertTrue(back.met)
    }
}
