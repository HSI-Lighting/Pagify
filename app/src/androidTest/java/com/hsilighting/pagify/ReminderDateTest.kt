package com.hsilighting.pagify

import com.hsilighting.pagify.ui.contacts.atLocalTime
import com.hsilighting.pagify.ui.contacts.morningIn
import java.util.Calendar
import java.util.TimeZone
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Turning what a date picker returns into the moment a reminder means.
 *
 * **The picker works in UTC and the reminder does not.** `selectedDateMillis` is
 * midnight UTC on the chosen day; adding a local hour to it directly puts the
 * reminder on the wrong day for anybody far enough from Greenwich — and the
 * mistake hides, because the date shown back is computed the same wrong way.
 *
 * These run in the device's own zone, whatever that is, which is the only zone
 * the app will ever be used in.
 */
class ReminderDateTest {

    /** Midnight UTC on a given day, as the picker would report it. */
    private fun utcMidnight(year: Int, month: Int, day: Int): Long =
        Calendar.getInstance(TimeZone.getTimeZone("UTC")).apply {
            clear()
            set(year, month, day, 0, 0, 0)
        }.timeInMillis

    /** The chosen day survives, in local terms, whatever the device's zone. */
    @Test
    fun the_day_chosen_is_the_day_stored() {
        val result = Calendar.getInstance().apply {
            timeInMillis = atLocalTime(utcMidnight(2026, Calendar.MARCH, 14), 15, 30)
        }

        assertEquals(2026, result.get(Calendar.YEAR))
        assertEquals(Calendar.MARCH, result.get(Calendar.MONTH))
        assertEquals(14, result.get(Calendar.DAY_OF_MONTH))
        assertEquals(15, result.get(Calendar.HOUR_OF_DAY))
        assertEquals(30, result.get(Calendar.MINUTE))
    }

    /** Seconds are cleared, so two reminders on the same minute compare equal. */
    @Test
    fun the_result_lands_on_the_minute() {
        val result = Calendar.getInstance().apply {
            timeInMillis = atLocalTime(utcMidnight(2026, Calendar.JULY, 1), 9, 0)
        }
        assertEquals(0, result.get(Calendar.SECOND))
        assertEquals(0, result.get(Calendar.MILLISECOND))
    }

    /** Midnight is a time like any other, and must not roll to the day before. */
    @Test
    fun midnight_stays_on_its_own_day() {
        val result = Calendar.getInstance().apply {
            timeInMillis = atLocalTime(utcMidnight(2026, Calendar.JANUARY, 1), 0, 0)
        }
        assertEquals(2026, result.get(Calendar.YEAR))
        assertEquals(Calendar.JANUARY, result.get(Calendar.MONTH))
        assertEquals(1, result.get(Calendar.DAY_OF_MONTH))
    }

    /** The offsets and the picker agree about what a day means. */
    @Test
    fun an_offset_lands_on_the_expected_day_at_nine() {
        val tomorrow = Calendar.getInstance().apply { timeInMillis = morningIn(1) }
        val expected = Calendar.getInstance().apply { add(Calendar.DAY_OF_YEAR, 1) }

        assertEquals(expected.get(Calendar.DAY_OF_MONTH), tomorrow.get(Calendar.DAY_OF_MONTH))
        assertEquals(9, tomorrow.get(Calendar.HOUR_OF_DAY))
        assertEquals(0, tomorrow.get(Calendar.MINUTE))
    }
}
