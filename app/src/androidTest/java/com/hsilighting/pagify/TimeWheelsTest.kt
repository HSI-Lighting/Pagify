package com.hsilighting.pagify

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.test.swipeUp
import com.hsilighting.pagify.ui.contacts.TimeWheels
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/**
 * The wheels that replaced the clock face.
 *
 * A dial asks which number and which half of the day in one gesture, and answers
 * the second itself. These columns put AM and PM on the screen where they can be
 * read, and are scrolled rather than aimed at.
 */
class TimeWheelsTest {

    @get:Rule
    val rule = createComposeRule()

    private var reported: Pair<Int, Int>? = null

    private fun wheels(hour: Int, minute: Int, is24Hour: Boolean = false) {
        reported = null
        rule.setContent {
            TimeWheels(
                initialHour = hour,
                initialMinute = minute,
                is24Hour = is24Hour,
                onChange = { h, m -> reported = h to m },
            )
        }
        rule.waitForIdle()
    }

    /**
     * The wheels must report the time they were handed, before anybody touches
     * them.
     *
     * They read their own scroll position to decide what is selected, and a
     * position read before the list has been laid out reads as zero — which
     * would quietly turn every reminder into midnight while the screen showed
     * the right time.
     */
    @Test
    fun the_opening_time_is_reported_unchanged() {
        wheels(hour = 15, minute = 35)
        assertEquals(15 to 35, reported)
    }

    /** Midnight is the position that a zero-by-accident would be mistaken for. */
    @Test
    fun midnight_opens_on_midnight() {
        wheels(hour = 0, minute = 0)
        assertEquals(0 to 0, reported)
    }

    /** Both halves of the day are on the screen, which is the whole point. */
    @Test
    fun am_and_pm_are_both_visible() {
        wheels(hour = 15, minute = 35)
        rule.onNodeWithText("AM").assertIsDisplayed()
        rule.onNodeWithText("PM").assertIsDisplayed()
    }

    /** Scrolling the half-of-day column moves the time by twelve hours. */
    @Test
    fun scrolling_from_pm_to_am_takes_twelve_hours_off() {
        wheels(hour = 15, minute = 35)
        assertEquals(15 to 35, reported)

        rule.onNodeWithContentDescription("Morning or afternoon")
            .performTouchInput { swipeDown() }
        rule.waitForIdle()

        assertEquals(3 to 35, reported)
    }

    /**
     * And the hour column moves the hour, without disturbing the minutes.
     *
     * On 24-hour wheels, where the column and the hour are the same number. In
     * twelve-hour mode they are not: scrolling up from 9 reaches 12, which as an
     * AM hour is midnight — hour zero. "Further down the column" and "later in
     * the day" genuinely part company there, and asserting on the wrong one
     * fails against a perfectly correct app.
     */
    @Test
    fun scrolling_the_hour_column_changes_only_the_hour() {
        wheels(hour = 9, minute = 30, is24Hour = true)
        assertEquals(9 to 30, reported)

        rule.onNodeWithContentDescription("Hour").performTouchInput { swipeUp() }
        rule.waitForIdle()

        val after = reported!!
        assertEquals("the minutes must not move with the hour", 30, after.second)
        assertTrue("scrolling up goes further down the column", after.first > 9)
    }

    /** On a 24-hour phone there is no half-of-day column to get wrong. */
    @Test
    fun a_twenty_four_hour_phone_has_no_am_or_pm() {
        wheels(hour = 15, minute = 35, is24Hour = true)
        assertEquals(15 to 35, reported)
        rule.onNodeWithText("AM").assertDoesNotExist()
        rule.onNodeWithText("PM").assertDoesNotExist()
    }
}
