package com.hsilighting.pagify

import android.app.Notification
import android.app.NotificationManager
import android.media.AudioAttributes
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.ReminderAlarmActivity
import com.hsilighting.pagify.core.Reminders
import com.hsilighting.pagify.data.db.ContactsDatabase
import com.hsilighting.pagify.data.db.toRow
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
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

    /** The alarm channel, and the one used when no screen can be taken. */
    private val ALARM = "contact-meetings-alarm"
    private val LOUD = "contact-meetings-loud"

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

    private fun fire(ring: Boolean = false) =
        runBlocking { Reminders.reschedule(context, notify = true, ring = ring) }

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
    fun aCaughtUpMeetingArrivesWithTheAlarmToneOnIt() {
        val due = System.currentTimeMillis() - 60_000
        save(Contact(id = meetingId, name = "Priya Raman", company = "Northwind", meetingAt = due))

        fire()

        val posted = postedOn(LOUD)
        assertNotNull("a meeting due a minute ago posted nothing", posted)
        assertEquals(
            "Meeting with Priya Raman",
            posted!!.notification.extras.getString("android.title"),
        )
        val channel = manager.getNotificationChannel(LOUD)
        assertEquals(NotificationManager.IMPORTANCE_HIGH, channel.importance)
        // This is the channel used when no screen can be taken, so it is the one
        // that has to make the noise by itself. On the alarm stream, or a phone
        // with the ringer down hears nothing at all.
        assertNotNull("the fallback channel has no sound of its own", channel.sound)
        assertEquals(
            "the fallback must ring on the alarm stream, not the notification one",
            AudioAttributes.USAGE_ALARM,
            channel.audioAttributes?.usage,
        )
    }

    @Test
    fun theAlarmMeetingTakesTheScreenAndWillNotBeSwipedAway() {
        val due = System.currentTimeMillis() - 60_000
        save(Contact(id = meetingId, name = "Priya Raman", company = "Northwind", meetingAt = due))

        // **Watched for, then shut.** The alarm screen is a real activity that
        // really opens and really starts ringing, and the first version of this
        // test pressed Back and hoped. It did not always land: the screen stayed
        // up, every Compose test after it was looking at an alarm face instead
        // of the screen it expected, and the run died fifty tests later
        // somewhere that had nothing to do with reminders.
        //
        // A monitor is both the fix and the better assertion — waiting for the
        // activity is how you find out the screen was taken at all, which is the
        // thing a user would call working.
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val monitor = instrumentation.addMonitor(ReminderAlarmActivity::class.java.name, null, false)
        var opened: android.app.Activity? = null
        try {
            fire(ring = true)

            opened = instrumentation.waitForMonitorWithTimeout(monitor, 10_000)
            assertNotNull("the alarm never took the screen", opened)

            val posted = postedOn(ALARM)
            assertNotNull("the alarm went off and posted nothing", posted)
            assertNotNull(
                "no full-screen intent, so it is a banner and not an alarm",
                posted!!.notification.fullScreenIntent,
            )
            // An alarm that clears with the same flick as an advert is not an
            // alarm. FLAG_ONGOING_EVENT is what refuses the flick.
            assertTrue(
                "the alarm can be swiped away without being answered",
                posted.notification.flags and Notification.FLAG_ONGOING_EVENT != 0,
            )
            // Silent on purpose: the alarm screen loops the tone. A sound here
            // as well would play once underneath it, which sounds like a fault.
            assertNull(
                "the alarm channel must stay silent or it doubles with the ringing",
                manager.getNotificationChannel(ALARM).sound,
            )
        } finally {
            opened?.let { instrumentation.runOnMainSync { it.finish() } }
            instrumentation.removeMonitor(monitor)
            instrumentation.waitForIdleSync()
        }
    }

    @Test
    fun theChimeChannelIsGone() {
        fire()
        // Renamed rather than reconfigured, because a channel's sound is fixed
        // when it is created and every later change is ignored. Leaving the old
        // id behind would put a channel in the phone's settings that looks like
        // it can be turned on and posts nothing.
        assertNull(
            "the old meeting channel is still registered",
            manager.getNotificationChannel("contact-meetings"),
        )
    }

    @Test
    fun snoozingMovesTheMeetingRatherThanKeepingASecondOne() = runBlocking {
        val due = System.currentTimeMillis() - 60_000
        save(Contact(id = meetingId, name = "Priya Raman", meetingAt = due))

        Reminders.snooze(context, meetingId)

        val row = ContactsDatabase.get(context).contacts().contactsById(listOf(meetingId)).single()
        val moved = row.meetingAt ?: 0L
        // Ten minutes on, give or take the time the call itself took.
        assertTrue(
            "the meeting was not moved forward: $moved against $due",
            moved > System.currentTimeMillis() + Reminders.SNOOZE_MILLIS - 30_000,
        )
        assertNull("a snoozed meeting must not read as dealt with", row.meetingDoneAt)
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

        assertNull("a reminder already marked done posted anyway", postedOn(LOUD, waitMillis = 1_000))
    }

    @Test
    fun oneStillAheadWaits() {
        val notYet = System.currentTimeMillis() + 3_600_000
        save(Contact(id = meetingId, name = "Priya Raman", meetingAt = notYet))

        fire()

        assertNull("a meeting an hour away posted now", postedOn(LOUD, waitMillis = 1_000))
    }
}
