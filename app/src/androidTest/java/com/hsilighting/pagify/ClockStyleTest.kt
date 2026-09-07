package com.hsilighting.pagify

import android.provider.Settings
import android.text.format.DateFormat
import androidx.test.platform.app.InstrumentationRegistry
import com.hsilighting.pagify.core.uses24Hour
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * Which clock the meeting picker offers.
 *
 * The picker asked `DateFormat.is24HourFormat`, which looks like it reads the
 * user's setting and does not: with the setting untouched it answers for the
 * *locale*, and en-GB's answer is 24-hour. The result was a dial numbered 0 to
 * 23 with no AM and no PM anywhere on it, on a phone that had never been asked
 * the question — and no way to say half past two in the afternoon.
 *
 * That is a bug you cannot see in a unit test of the picker, because the picker
 * was doing exactly what it was told. It is only visible one level down, in what
 * the question returns.
 */
class ClockStyleTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val setting: String?
        get() = Settings.System.getString(context.contentResolver, Settings.System.TIME_12_24)

    @Test
    fun withNothingSetTheClockNamesItsHalf() {
        assumeTrue("this phone has an explicit 12/24 setting", setting == null)
        assertFalse(
            "with no setting of their own the user must still get AM and PM",
            uses24Hour(context),
        )
    }

    @Test
    fun theLocaleFallbackIsWhatWentWrong() {
        assumeTrue("this phone has an explicit 12/24 setting", setting == null)
        // Not a test of Android — a test of the reason this function exists. If
        // the platform's answer ever stops differing from ours here, the
        // workaround has become dead weight and should be deleted rather than
        // left to be trusted for a reason that no longer holds.
        assumeTrue("this locale is a 12-hour one anyway", DateFormat.is24HourFormat(context))
        assertTrue(
            "the platform still falls back to the locale, so this function still earns its keep",
            DateFormat.is24HourFormat(context) != uses24Hour(context),
        )
    }

    @Test
    fun anExplicitSettingIsObeyed() {
        val explicit = setting ?: return
        assertTrue(
            "a user who has actually chosen must get the clock they chose",
            uses24Hour(context) == (explicit == "24"),
        )
    }
}
