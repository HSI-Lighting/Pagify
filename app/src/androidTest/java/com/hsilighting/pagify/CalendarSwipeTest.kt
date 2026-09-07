package com.hsilighting.pagify

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import androidx.compose.ui.test.swipeUp
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.ui.contacts.CalendarScreen
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

/**
 * The way out of the calendar.
 *
 * The screen shipped taking only a left-to-right swipe back, which is the same
 * gesture that opened it rather than its mirror — so dragging back the way you
 * came, the first thing anybody tries, did nothing. Nothing failed loudly; the
 * screen simply would not close, and the only way out was the arrow.
 *
 * Both directions are asserted rather than just the new one, because the fix is
 * a widening and a widening is the easy thing to overshoot into "any touch
 * leaves". The vertical cases are the ones that say it did not: the month grid
 * and the day list have to keep scrolling.
 */
class CalendarSwipeTest {
    @get:Rule
    val composeTestRule = createComposeRule()

    private val contacts = listOf(
        Contact(id = 1, name = "Jane Okafor", company = "Meridian"),
        Contact(id = 2, name = "Sam Reyes"),
    )

    /** Runs the calendar and reports how many times it asked to be closed. */
    private fun calendar(gesture: androidx.compose.ui.test.TouchInjectionScope.() -> Unit): Int {
        var backs = 0
        composeTestRule.setContent {
            CalendarScreen(contacts = contacts, onOpenContact = {}, onBack = { backs++ })
        }
        composeTestRule.onRoot().performTouchInput(gesture)
        composeTestRule.waitForIdle()
        return backs
    }

    @Test
    fun swipingRightToLeftGoesBack() {
        // The mirror of the swipe that opened the calendar, and the one that
        // used to do nothing.
        assertEquals(1, calendar { swipeLeft() })
    }

    @Test
    fun swipingLeftToRightGoesBack() {
        // Already worked, and has to keep working: it is the gesture people who
        // used the earlier build have in their fingers.
        assertEquals(1, calendar { swipeRight() })
    }

    @Test
    fun swipingUpDoesNotGoBack() {
        assertEquals(0, calendar { swipeUp() })
    }

    @Test
    fun swipingDownDoesNotGoBack() {
        assertEquals(0, calendar { swipeDown() })
    }
}
