package com.hsilighting.pagify

import android.app.NotificationManager
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.Reminders
import com.hsilighting.pagify.data.db.ContactsDatabase
import com.hsilighting.pagify.data.db.toRow
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test

/**
 * Does a due reminder actually reach the shade?
 *
 * Everything up to this point was tested and none of it was this. The queries
 * know what is due, the migration keeps the columns, the sheet writes the
 * times — and a notification that is never posted passes all of them. The whole
 * feature fails silently by design: there is no crash to see and no log to read,
 * only a reminder that does not arrive, on a day you find out too late.
 *
 * These run against the **real on-disk database**, not an in-memory one, because
 * [Reminders] opens the real one by itself — that coupling is the thing under
 * test, and a test that handed it a database of its own would prove only that
 * the code it was given works. The rows are removed afterwards; they carry made
 * up names and no card text.
 *
 * **The notifications are cancelled at the start of each test rather than the
 * end, and are deliberately left on screen.** Stable notification ids mean a
 * re-run replaces rather than stacks, so a clean slate at the start is worth as
 * much as a tidy one at the end — and it leaves the actual banner on the phone
 * for a person to look at, which is the one thing about a notification no
 * assertion covers: whether it says something a human can act on.
 */
class ReminderNotificationTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val manager = context.getSystemService(NotificationManager::class.java)

    /** Far above anything a real scan will allocate, so nothing collides. */
    private val meetingId = 900_001L
    private val followUpId = 900_002L

    /**
     * The notification ids those two contacts can occupy.
     *
     * `Reminders` derives them as `id * 2` plus one for a meeting, and both
     * contacts could carry either kind, so it is four. Kept in step with that
     * rule by hand because the function producing it is private — if it ever
     * changes, these tests stop cleaning up after themselves and start failing
     * on the second run, which is a loud enough way to find out.
     */
    private val ownNotificationIds = listOf(meetingId, followUpId)
        .flatMap { listOf((it * 2).toInt(), (it * 2).toInt() + 1) }

    @Before
    fun clearTheShade() {
        if (Build.VERSION.SDK_INT >= 33) {
            InstrumentationRegistry.getInstrumentation().uiAutomation.grantRuntimePermission(
                context.packageName,
                "android.permission.POST_NOTIFICATIONS",
            )
        }
        // Only this test's own four, not `cancelAll`. The database here is the
        // app's real one, so its reminders are real too — a test that cleared
        // the shade would take a genuine pending meeting down with it, and the
        // person whose meeting it was would never know it had been there.
        ownNotificationIds.forEach(manager::cancel)
        runBlocking {
            val dao = ContactsDatabase.get(context).contacts()
            dao.deleteContact(meetingId)
            dao.deleteContact(followUpId)
        }
    }

    /**
     * The rows go, the notification stays.
     *
     * Nothing invented is left in the database, but a notification cancelled the
     * moment it was asserted is one nobody ever sees. It costs nothing to leave:
     * the ids are stable, so the next run replaces it rather than adding a
     * second, and it is a real one — the same builder, channel and wording a
     * reminder set in the app produces.
     */
    @After
    fun removeTheRows() = runBlocking {
        val dao = ContactsDatabase.get(context).contacts()
        dao.deleteContact(meetingId)
        dao.deleteContact(followUpId)
    }

    private fun save(contact: Contact) = runBlocking {
        ContactsDatabase.get(context).contacts().save(contact.toRow())
    }

    private fun fire() = runBlocking { Reminders.reschedule(context, notify = true) }

    /**
     * What is in the shade on that channel, after giving it a moment to arrive.
     *
     * `notify` hands the notification to a system service and returns; it is in
     * the shade a beat later. Reading straight after posting is the classic way
     * to write a test that passes on a quiet phone and fails on a busy one — and
     * the negative cases need the same wait for the opposite reason: "nothing
     * posted" measured instantly only proves the check was quick.
     */
    private fun postedOn(channel: String, waitMillis: Long = 3_000) =
        generateSequence(0) { it + 50 }
            .takeWhile { it <= waitMillis }
            .firstNotNullOfOrNull { elapsed ->
                if (elapsed > 0) Thread.sleep(50)
                manager.activeNotifications.firstOrNull { it.notification.channelId == channel }
            }

    @Test
    fun aDueMeetingArrivesOnTheMeetingChannel() {
        val due = System.currentTimeMillis() - 60_000
        save(Contact(id = meetingId, name = "Priya Raman", company = "Northwind", meetingAt = due))

        fire()

        val posted = postedOn("contact-meetings")
        assertNotNull("a meeting due a minute ago posted nothing", posted)
        assertEquals(
            "Meeting with Priya Raman",
            posted!!.notification.extras.getString("android.title"),
        )
        // The channel is what makes it drop over whatever is on screen. A
        // meeting posted quietly is a meeting missed, and the two channels exist
        // so the phone's own settings can silence one without the other.
        assertEquals(
            NotificationManager.IMPORTANCE_HIGH,
            manager.getNotificationChannel("contact-meetings").importance,
        )
    }

    @Test
    fun aDueFollowUpArrivesOnItsOwnChannel() {
        val due = System.currentTimeMillis() - 60_000
        save(Contact(id = followUpId, name = "Tomas Iverson", followUpAt = due))

        fire()

        val posted = postedOn("contact-follow-ups")
        assertNotNull("a follow-up due a minute ago posted nothing", posted)
        assertEquals(
            "Follow up with Tomas Iverson",
            posted!!.notification.extras.getString("android.title"),
        )
    }

    @Test
    fun oneAlreadyDealtWithSaysNothing() {
        // The case that would turn the feature into a nuisance: a reminder
        // marked done still being due by date, and so posted again at every
        // alarm until the date passes out of reach.
        val due = System.currentTimeMillis() - 60_000
        save(
            Contact(
                id = meetingId,
                name = "Priya Raman",
                meetingAt = due,
                meetingDoneAt = due + 1_000,
            ),
        )

        fire()

        assertNull("a reminder already marked done posted anyway", postedOn("contact-meetings", waitMillis = 1_000))
    }

    @Test
    fun oneStillAheadWaits() {
        val notYet = System.currentTimeMillis() + 3_600_000
        save(Contact(id = meetingId, name = "Priya Raman", meetingAt = notYet))

        fire()

        assertNull("a meeting an hour away posted now", postedOn("contact-meetings", waitMillis = 1_000))
    }
}
