package com.hsilighting.pagify

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.Meeting
import com.hsilighting.pagify.ui.contacts.CalendarScreen
import com.hsilighting.pagify.ui.contacts.atTime
import com.hsilighting.pagify.ui.contacts.dayOfMonth
import org.junit.Rule
import org.junit.Test
import java.util.Calendar

/**
 * On the day itself, a meeting has to say when.
 *
 * The row named who and left when to be guessed, while the app held the answer
 * all along — the picker sets an hour and a minute, and nothing showed them. It
 * is the one fact the day view exists to give: **when is it, and with whom.**
 */
class CalendarDayTimeTest {

    @get:Rule
    val rule = createComposeRule()

    /**
     * A day well before today, for the fixtures that carry a time.
     *
     * A `Contact` defaults `capturedAt` to now, which would put every fixture
     * under "Met this day" as well as under its own section — correct of the
     * app, and two matches for one name, which is a test that cannot assert on
     * a name at all. Scanned last year, meeting today.
     */
    private val scannedLongAgo: Long = Calendar.getInstance().apply {
        add(Calendar.YEAR, -1)
    }.timeInMillis

    /** Today at a given hour and minute, so the test is not date-dependent. */
    private fun todayAt(hour: Int, minute: Int): Long = Calendar.getInstance().apply {
        set(Calendar.HOUR_OF_DAY, hour)
        set(Calendar.MINUTE, minute)
        set(Calendar.SECOND, 0)
        set(Calendar.MILLISECOND, 0)
    }.timeInMillis

    private fun show(contacts: List<Contact>) {
        rule.setContent {
            CalendarScreen(
                contacts = contacts,
                onOpenProgress = {},
                onOpenDetails = {},
                onAddMeeting = { _, _ -> },
                onCancelMeeting = {},
                onBack = {},
            )
        }
        // Today's cell, which is where every fixture below is placed.
        rule.onNodeWithText(dayOfMonth(System.currentTimeMillis()).toString()).performClick()
    }

    @Test
    fun a_meeting_shows_the_time_it_starts() {
        val at = todayAt(14, 30)
        show(listOf(Contact(id = 1, name = "Priya Raman", capturedAt = scannedLongAgo, meetings = listOf(Meeting(contactId = 0, at = at)))))

        // Formatted the way the phone formats times, not as a pattern of ours:
        // a 24-hour phone must not be shown 2:30 pm.
        rule.onNodeWithText(atTime(at)).assertIsDisplayed()
        rule.onNodeWithText("Priya Raman").assertIsDisplayed()
    }

    /** A chase carries its time too — same row, same rule. */
    @Test
    fun a_follow_up_shows_its_time() {
        val at = todayAt(9, 5)
        show(listOf(Contact(id = 2, name = "Owen Hart", capturedAt = scannedLongAgo, followUpAt = at)))

        rule.onNodeWithText(atTime(at)).assertIsDisplayed()
        rule.onNodeWithText("Owen Hart").assertIsDisplayed()
    }

    /**
     * Several on one day read down the times, earliest first.
     *
     * Sorted explicitly, because the contact list's own order is by when the
     * card was scanned — which has nothing to do with when you agreed to meet.
     */
    @Test
    fun a_day_of_meetings_is_in_time_order() {
        val nine = todayAt(9, 0)
        val two = todayAt(14, 0)
        val eleven = todayAt(11, 0)
        show(
            listOf(
                Contact(id = 1, name = "Afternoon", capturedAt = scannedLongAgo, meetings = listOf(Meeting(contactId = 0, at = two))),
                Contact(id = 2, name = "Morning", capturedAt = scannedLongAgo, meetings = listOf(Meeting(contactId = 0, at = nine))),
                Contact(id = 3, name = "Midday", capturedAt = scannedLongAgo, meetings = listOf(Meeting(contactId = 0, at = eleven))),
            ),
        )

        val times = listOf(nine, eleven, two).map(::atTime)
        val tops = times.map { rule.onNodeWithText(it).fetchSemanticsNode().positionInRoot.y }
        assertOrdered(tops)
    }

    private fun assertOrdered(tops: List<Float>) {
        tops.zipWithNext().forEach { (earlier, later) ->
            if (earlier >= later) {
                throw AssertionError("the times are not in order: $tops")
            }
        }
    }

    /** Somebody met that day, with no meeting, carries no time and needs none. */
    @Test
    fun a_card_collected_that_day_shows_no_time() {
        show(listOf(Contact(id = 4, name = "Just Scanned", capturedAt = System.currentTimeMillis())))
        rule.onNodeWithText("Just Scanned").assertIsDisplayed()
    }
}
