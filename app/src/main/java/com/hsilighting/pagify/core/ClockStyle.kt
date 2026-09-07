package com.hsilighting.pagify.core

import android.content.Context
import android.provider.Settings

/**
 * Whether to show hours 0–23, or hours with AM and PM on them.
 *
 * **Only an explicit setting counts as 24-hour.** The obvious call here is
 * `DateFormat.is24HourFormat`, and it has a fallback nobody expects: when the
 * user has never touched the setting it answers for the *locale*, and en-GB's
 * locale answer is 24-hour. On a phone that had simply never been asked, the
 * meeting time picker therefore came up as a 0–23 dial with no AM and no PM
 * anywhere on it, and there was no way to say half past two in the afternoon.
 *
 * A locale default is a guess about a country. The setting is a person having
 * actually said so, and that is the only thing worth reading. Absent it, the
 * clock that names its half is the safe one: it cannot be misread by somebody
 * who thinks in 24 hours, where a bare "14" can by somebody who does not.
 *
 * Read wherever a time is shown as well as where one is chosen, so an alarm
 * reads back the hour the user set rather than its translation.
 */
internal fun uses24Hour(context: Context): Boolean =
    runCatching {
        Settings.System.getString(context.contentResolver, Settings.System.TIME_12_24)
    }.getOrNull() == "24"
