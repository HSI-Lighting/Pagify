package com.hsilighting.pagify

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.Meeting
import com.hsilighting.pagify.ui.contacts.CalendarScreen
import com.hsilighting.pagify.ui.contacts.dayOfMonth
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import java.util.Calendar

/**
 * Which sheet a day's row opens, and why the three are not the same.
 *
 * A **meeting** opens the person: by the time it is on the calendar the progress
 * is decided — that is why the row exists — and what is wanted ten minutes
 * beforehand is the telephone number.
 *
 * A **card collected that day** and a **contact to chase** both open the stage
 * and reminder editor, because both are the same question: where has this got
 * to. Chasing somebody *is* moving them along.
 *
 * The distinction is invisible from the screen — every row looks alike — so
 * without this it could be swapped back by anybody tidying the callbacks, and
 * nothing would look wrong until somebody reached for a phone number and got a
 * stage picker.
 */
class CalendarTapTest {

    @get:Rule
    val rule = createComposeRule()

    private var openedProgress: Contact? = null
    private var openedDetails: Contact? = null

    private val scannedLongAgo: Long =
        Calendar.getInstance().apply { add(Calendar.YEAR, -1) }.timeInMillis

    private fun todayAt(hour: Int): Long = Calendar.getInstance().apply {
        set(Calendar.HOUR_OF_DAY, hour)
        set(Calendar.MINUTE, 0)
        set(Calendar.SECOND, 0)
        set(Calendar.MILLISECOND, 0)
    }.timeInMillis

    /** Click a row by its text. */
    private fun tap(text: String) {
        rule.onNodeWithText(text).performClick()
    }

    private fun show(contacts: List<Contact>) {
        openedProgress = null
        openedDetails = null
        rule.setContent {
            CalendarScreen(
                contacts = contacts,
                onOpenProgress = { openedProgress = it },
                onOpenDetails = { openedDetails = it },
                onAddMeeting = { _, _ -> },
                onCancelMeeting = {},
                onBack = {},
            )
        }
        tap(dayOfMonth(System.currentTimeMillis()).toString())
    }

    @Test
    fun tapping_a_meeting_opens_the_person() {
        show(
            listOf(
                Contact(
                    id = 1,
                    name = "Priya Raman",
                    capturedAt = scannedLongAgo,
                    meetings = listOf(Meeting(contactId = 0, at = todayAt(14))),
                ),
            ),
        )
        tap("Priya Raman")

        assertEquals("Priya Raman", openedDetails?.name)
        assertNull("a meeting must not open the progress editor", openedProgress)
    }

    @Test
    fun tapping_a_follow_up_opens_the_progress() {
        show(
            listOf(
                Contact(
                    id = 2,
                    name = "Owen Hart",
                    capturedAt = scannedLongAgo,
                    followUpAt = todayAt(9),
                ),
            ),
        )
        tap("Owen Hart")

        assertEquals("Owen Hart", openedProgress?.name)
        assertNull("a follow-up must not open the detail sheet", openedDetails)
    }

    @Test
    fun tapping_a_card_collected_that_day_opens_the_progress() {
        show(
            listOf(
                Contact(id = 3, name = "Just Scanned", capturedAt = System.currentTimeMillis()),
            ),
        )
        tap("Just Scanned")

        assertEquals("Just Scanned", openedProgress?.name)
        assertNull("a collected card must not open the detail sheet", openedDetails)
    }
}
