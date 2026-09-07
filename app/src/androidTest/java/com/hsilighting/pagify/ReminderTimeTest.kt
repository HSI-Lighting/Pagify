package com.hsilighting.pagify

import com.hsilighting.pagify.ui.contacts.MINUTE_STEP
import com.hsilighting.pagify.ui.contacts.firstOfferableTime
import com.hsilighting.pagify.ui.contacts.hasPassed
import com.hsilighting.pagify.ui.contacts.isOfferableDay
import com.hsilighting.pagify.ui.contacts.minuteIndexOf
import com.hsilighting.pagify.ui.contacts.pickerDayOf
import com.hsilighting.pagify.ui.contacts.to12Hour
import com.hsilighting.pagify.ui.contacts.to24Hour
import java.util.Calendar
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A reminder can only point forwards, and it has to land in the right half of
 * the day.
 *
 * Both of these went wrong in the same place. The old picker opened on nine
 * o'clock with AM already chosen, so setting a meeting for three in the
 * afternoon meant noticing a default nobody looks at; and it accepted the
 * result either way, which stored a moment that had already gone. A reminder in
 * the past does not fire late — it does not fire at all, or it fires instantly
 * and is dismissed as noise. Either way the person who set it believes it is
 * waiting for them.
 */
class ReminderTimeTest {

    /** A moment today, in the phone's own zone. */
    private fun todayAt(hour: Int, minute: Int): Long = Calendar.getInstance().apply {
        set(Calendar.HOUR_OF_DAY, hour)
        set(Calendar.MINUTE, minute)
        set(Calendar.SECOND, 0)
        set(Calendar.MILLISECOND, 0)
    }.timeInMillis

    private fun daysFromNow(days: Int): Long = Calendar.getInstance().apply {
        add(Calendar.DAY_OF_YEAR, days)
    }.timeInMillis

    // ---- where the wheels open ------------------------------------------------

    /** On a later day there is no urgency, so nine o'clock as before. */
    @Test
    fun a_later_day_opens_at_nine_in_the_morning() {
        assertEquals(
            9 to 0,
            firstOfferableTime(pickerDayOf(daysFromNow(3)), System.currentTimeMillis()),
        )
    }

    /**
     * The whole point. Half past three in the afternoon must open in the
     * afternoon — if this returns an hour under twelve, the AM trap is back.
     */
    @Test
    fun an_afternoon_today_opens_in_the_afternoon() {
        val now = todayAt(15, 31)
        val (hour, minute) = firstOfferableTime(pickerDayOf(now), now)

        assertEquals(15, hour)
        assertEquals(35, minute)
        assertTrue("must be an afternoon hour, or AM is chosen by default", hour >= 12)
        assertTrue("the afternoon must read as PM", to12Hour(hour).second)
    }

    /** Ahead of now, never level with it: a reminder for this instant is noise. */
    @Test
    fun a_time_exactly_on_a_mark_moves_to_the_next_one() {
        val now = todayAt(15, 35)
        assertEquals(15 to 40, firstOfferableTime(pickerDayOf(now), now))
    }

    /**
     * Late at night nothing is left before midnight. It stops at the last mark
     * rather than rolling into tomorrow, and that mark reads as gone — which is
     * what disables the button instead of silently moving the day.
     */
    @Test
    fun the_end_of_the_night_stops_rather_than_rolling_over() {
        val now = todayAt(23, 58)
        val day = pickerDayOf(now)
        val (hour, minute) = firstOfferableTime(day, now)

        assertEquals(23 to 55, hour to minute)
        assertTrue("the last mark of the day has gone", hasPassed(day, hour, minute, now))
    }

    /** Every opening time this offers is one that can actually be confirmed. */
    @Test
    fun what_it_offers_is_confirmable_at_every_minute_of_the_day() {
        for (hour in 0..23) {
            for (minute in 0 until 60) {
                val now = todayAt(hour, minute)
                val day = pickerDayOf(now)
                val (h, m) = firstOfferableTime(day, now)
                // The single exception is the tail of the night, which has no
                // later mark and is refused on purpose.
                val lastOfTheNight = h == 23 && m == 60 - MINUTE_STEP && hour == 23 && minute >= 55
                if (!lastOfTheNight) {
                    assertFalse(
                        "opened on a time already gone at $hour:$minute",
                        hasPassed(day, h, m, now),
                    )
                }
            }
        }
    }

    // ---- what counts as gone --------------------------------------------------

    @Test
    fun earlier_today_has_gone_and_later_today_has_not() {
        val now = todayAt(12, 0)
        val day = pickerDayOf(now)

        assertTrue(hasPassed(day, 11, 55, now))
        assertFalse(hasPassed(day, 12, 5, now))
    }

    /** This instant counts as gone: setting it would fire before it was saved. */
    @Test
    fun this_very_minute_counts_as_gone() {
        val now = todayAt(12, 0)
        assertTrue(hasPassed(pickerDayOf(now), 12, 0, now))
    }

    @Test
    fun a_time_on_a_later_day_has_not_gone() {
        val later = daysFromNow(2)
        assertFalse(hasPassed(pickerDayOf(later), 0, 0, System.currentTimeMillis()))
    }

    // ---- twelve-hour arithmetic ----------------------------------------------

    /** Midnight and noon are the two that catch everybody. */
    @Test
    fun midnight_is_twelve_am_and_noon_is_twelve_pm() {
        assertEquals(12 to false, to12Hour(0))
        assertEquals(12 to true, to12Hour(12))
        assertEquals(0, to24Hour(12, pm = false))
        assertEquals(12, to24Hour(12, pm = true))
    }

    @Test
    fun every_hour_survives_the_round_trip() {
        for (hour in 0..23) {
            val (spoken, pm) = to12Hour(hour)
            assertTrue("$hour spoken as $spoken", spoken in 1..12)
            assertEquals("hour $hour", hour, to24Hour(spoken, pm))
        }
    }

    // ---- the minute wheel -----------------------------------------------------

    /** Nine o'clock, the offsets' own time, must sit on the first stop. */
    @Test
    fun the_offsets_time_sits_on_a_stop() {
        assertEquals(0, minuteIndexOf(0))
    }

    @Test
    fun a_minute_between_stops_rounds_down_and_stays_in_range() {
        assertEquals(minuteIndexOf(30), minuteIndexOf(34))
        for (minute in 0 until 60) {
            assertTrue(minuteIndexOf(minute) in 0 until 60 / MINUTE_STEP)
        }
    }

    // ---- the day the picker means --------------------------------------------

    /**
     * The picker works in UTC and a person does not. Just before midnight is
     * where a naive conversion moves the reminder to tomorrow — or, west of
     * Greenwich, to yesterday.
     */
    @Test
    fun the_local_day_survives_the_trip_through_utc() {
        val lateTonight = todayAt(23, 30)
        val local = Calendar.getInstance().apply { timeInMillis = lateTonight }
        val roundTrip = Calendar.getInstance().apply {
            timeInMillis = com.hsilighting.pagify.ui.contacts.atLocalTime(
                pickerDayOf(lateTonight),
                23,
                30,
            )
        }

        assertEquals(local.get(Calendar.YEAR), roundTrip.get(Calendar.YEAR))
        assertEquals(local.get(Calendar.MONTH), roundTrip.get(Calendar.MONTH))
        assertEquals(local.get(Calendar.DAY_OF_MONTH), roundTrip.get(Calendar.DAY_OF_MONTH))
    }

    // ---- which days are offered at all ----------------------------------------

    /** Today counts: most of it is usually still ahead. */
    @Test
    fun today_is_still_offered() {
        val now = todayAt(23, 0)
        assertTrue(isOfferableDay(pickerDayOf(now), now))
    }

    /** Yesterday is not, at any hour. A reminder cannot point backwards. */
    @Test
    fun yesterday_is_not_offered() {
        val now = System.currentTimeMillis()
        assertFalse(isOfferableDay(pickerDayOf(daysFromNow(-1)), now))
        assertFalse(isOfferableDay(pickerDayOf(daysFromNow(-30)), now))
    }

    @Test
    fun days_ahead_are_offered() {
        val now = System.currentTimeMillis()
        assertTrue(isOfferableDay(pickerDayOf(daysFromNow(1)), now))
        assertTrue(isOfferableDay(pickerDayOf(daysFromNow(365)), now))
    }
}
