package com.hsilighting.pagify

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.hsilighting.pagify.ui.contacts.DateAndTimePicker
import java.util.Calendar
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test

/**
 * The dialog itself refusing a moment that has gone.
 *
 * The arithmetic behind this is covered elsewhere; what is covered here is the
 * wiring, which is the part that gets lost. A predicate that is right and a
 * button that ignores it look identical from the outside — and the symptom, a
 * reminder that never arrives, appears days later with nothing to connect it
 * back to this screen.
 *
 * Both cases run against a fixed clock. The end of the night is the only time
 * of day where the wheels open on something unusable, and waiting until 23:58
 * to find out is not a test.
 */
class PastTimeRefusedTest {

    @get:Rule
    val rule = createComposeRule()

    private var picked: Long? = null

    private fun todayAt(hour: Int, minute: Int): Long = Calendar.getInstance().apply {
        set(Calendar.HOUR_OF_DAY, hour)
        set(Calendar.MINUTE, minute)
        set(Calendar.SECOND, 0)
        set(Calendar.MILLISECOND, 0)
    }.timeInMillis

    /**
     * Opens the dialog at a given moment, on today, and steps past the date.
     *
     * An existing reminder for today is what makes the date step land on today.
     * With none, the picker sensibly opens on tomorrow — where nothing has gone
     * yet, and neither case below would say anything.
     */
    private fun openAt(now: Long) {
        picked = null
        rule.setContent {
            DateAndTimePicker(
                initial = todayAt(20, 0),
                onPicked = { picked = it },
                onDismiss = {},
                now = now,
            )
        }
        // The date step opens on today, which is the day both cases are about.
        rule.onNodeWithText("Next").performClick()
        rule.waitForIdle()
    }

    /**
     * Two minutes to midnight: every five-minute mark left today has gone, so
     * the dialog says so and will not set one.
     */
    @Test
    fun a_time_that_has_gone_cannot_be_set() {
        openAt(todayAt(23, 58))

        rule.onNodeWithText("That time has already gone. Pick a later one.").assertIsDisplayed()
        rule.onNodeWithText("Set").assertIsNotEnabled()

        rule.onNodeWithText("Set").performClick()
        assertNull("a refused time must not slip through the button", picked)
    }

    /** At midday there is plenty of day left, so nothing is in the way. */
    @Test
    fun a_time_still_ahead_can_be_set() {
        openAt(todayAt(12, 0))

        rule.onNodeWithText("That time has already gone. Pick a later one.").assertDoesNotExist()
        rule.onNodeWithText("Set").assertIsEnabled()
    }
}
