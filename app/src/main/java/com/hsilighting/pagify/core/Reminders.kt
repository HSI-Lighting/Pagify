package com.hsilighting.pagify.core

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.hsilighting.pagify.MainActivity
import com.hsilighting.pagify.R
import com.hsilighting.pagify.data.db.ContactRow
import com.hsilighting.pagify.data.db.ContactsDatabase
import com.hsilighting.pagify.data.db.ReminderKind

/**
 * Getting a reminder in front of somebody who has closed the app.
 *
 * # Two channels, because the two reminders are not alike
 *
 * A **meeting** is an appointment. It arrives as a heads-up — the banner that
 * drops over whatever is on screen, the way a meeting alert does — because being
 * late to it is the whole failure, and a line in the shade that gets noticed
 * tomorrow is no use. A **follow-up** is a nudge; it waits quietly in the shade,
 * because being a few hours late to it costs nothing and a banner for it would
 * teach the user to swipe banners away.
 *
 * Separate channels rather than separate priorities on one, so the phone's own
 * settings can turn the loud one down without silencing both. That is the
 * user's decision to make and the only way to offer it.
 *
 * # One alarm, not one per contact
 *
 * Only the *soonest* reminder ahead holds an alarm. When it fires, everything
 * due is posted and the next is scheduled. A hundred contacts with reminders
 * cost one alarm rather than a hundred — Android caps how many a process may
 * hold and silently drops the excess, which shows up as reminders that never
 * arrive for whoever set the most.
 *
 * The cost is that a reminder set *earlier* than the pending alarm must
 * reschedule, which is why [reschedule] runs after every save.
 *
 * # Inexact on purpose
 *
 * `setAndAllowWhileIdle` rather than the exact form. Exact alarms need
 * `SCHEDULE_EXACT_ALARM`, which the user grants in a settings screen and which
 * Android reserves for alarm clocks and calendar invitations. Asking for a
 * permission that can be refused — leaving the feature silently dead — is worse
 * than arriving within a few minutes of the hour.
 */
object Reminders {

    private const val MEETING_CHANNEL = "contact-meetings"
    private const val FOLLOW_UP_CHANNEL = "contact-follow-ups"
    private const val ALARM_REQUEST = 4711

    const val ACTION_FIRE = "com.hsilighting.pagify.REMINDER"
    const val ACTION_DONE = "com.hsilighting.pagify.REMINDER_DONE"
    const val EXTRA_CONTACT = "contactId"
    const val EXTRA_KIND = "kind"

    /**
     * Look at what is due, say so, and set the alarm for what is next.
     *
     * Safe from anywhere and at any time — after a save, after a boot, after the
     * alarm itself. It reads the current state rather than trusting what it was
     * told, so a missed alarm and a duplicated one come out the same.
     */
    suspend fun reschedule(context: Context, notify: Boolean) {
        val application = context.applicationContext
        val dao = ContactsDatabase.get(application).contacts()
        val now = System.currentTimeMillis()

        if (notify) {
            val due = runCatching { dao.dueReminders(now) }.getOrElse {
                Log.w("Reminders", "could not read what is due", it)
                emptyList()
            }
            // One notification per contact rather than one summarising all of
            // them: a meeting and a chase need different urgency, and a single
            // line reading "3 reminders" says neither which nor how soon.
            due.forEach { row ->
                if (row.meetingAt != null && row.meetingDoneAt == null && row.meetingAt <= now) {
                    post(application, row, ReminderKind.Meeting)
                }
                if (row.followUpAt != null && row.followUpDoneAt == null && row.followUpAt <= now) {
                    post(application, row, ReminderKind.FollowUp)
                }
            }
        }

        val nextAt = runCatching { dao.nextReminderAt(now) }.getOrNull()
        val alarms = application.getSystemService(AlarmManager::class.java) ?: return
        val pending = firePendingIntent(application)

        if (nextAt == null) {
            // Nothing ahead: cancel rather than leave a stale alarm that would
            // wake the device to find nothing to say.
            alarms.cancel(pending)
            return
        }

        runCatching {
            alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, nextAt, pending)
        }.onFailure { Log.w("Reminders", "the alarm could not be set", it) }
    }

    /** Mark one reminder dealt with, from the notification's own button. */
    suspend fun markDone(context: Context, contactId: Long, kind: ReminderKind) {
        val application = context.applicationContext
        val dao = ContactsDatabase.get(application).contacts()
        val row = runCatching { dao.contactsById(listOf(contactId)) }.getOrNull()?.firstOrNull()
            ?: return
        val now = System.currentTimeMillis()

        runCatching {
            dao.setProgress(
                id = contactId,
                stage = row.stage,
                met = row.met,
                followUpAt = row.followUpAt,
                followUpDoneAt = if (kind == ReminderKind.FollowUp) now else row.followUpDoneAt,
                meetingAt = row.meetingAt,
                meetingDoneAt = if (kind == ReminderKind.Meeting) now else row.meetingDoneAt,
            )
        }.onFailure { Log.w("Reminders", "could not mark the reminder done", it) }

        NotificationManagerCompat.from(application).cancel(notificationId(contactId, kind))
        reschedule(application, notify = false)
    }

    private fun firePendingIntent(context: Context): PendingIntent {
        val intent = Intent(context, ReminderReceiver::class.java).setAction(ACTION_FIRE)
        return PendingIntent.getBroadcast(
            context,
            ALARM_REQUEST,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }

    /**
     * Stable per contact and per kind, so a meeting and a chase for the same
     * person are two notifications and re-posting either replaces itself rather
     * than stacking up a new one every time an alarm fires.
     */
    private fun notificationId(contactId: Long, kind: ReminderKind): Int =
        (contactId.toInt() * 2) + if (kind == ReminderKind.Meeting) 1 else 0

    private fun post(context: Context, row: ContactRow, kind: ReminderKind) {
        ensureChannels(context)

        val who = row.name.ifBlank { row.company }.ifBlank { "a contact" }
        val isMeeting = kind == ReminderKind.Meeting

        val open = PendingIntent.getActivity(
            context,
            notificationId(row.id, kind),
            Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val done = PendingIntent.getBroadcast(
            context,
            notificationId(row.id, kind) + 100_000,
            Intent(context, ReminderReceiver::class.java)
                .setAction(ACTION_DONE)
                .putExtra(EXTRA_CONTACT, row.id)
                .putExtra(EXTRA_KIND, kind.name),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

        val builder = NotificationCompat.Builder(
            context,
            if (isMeeting) MEETING_CHANNEL else FOLLOW_UP_CHANNEL,
        )
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(if (isMeeting) "Meeting with $who" else "Follow up with $who")
            .setContentText(
                row.company.takeIf { it.isNotBlank() && it != who }
                    ?: if (isMeeting) "Starting now" else "Time to get back to them",
            )
            .setCategory(if (isMeeting) Notification.CATEGORY_EVENT else Notification.CATEGORY_REMINDER)
            .setAutoCancel(true)
            .setContentIntent(open)
            .addAction(0, "Done", done)

        if (isMeeting) {
            // What makes it drop over whatever is on screen rather than waiting
            // in the shade. The channel importance decides this from Android 8
            // onward; the priority is what does it on 7 and earlier.
            builder.setPriority(NotificationCompat.PRIORITY_HIGH)
                .setDefaults(NotificationCompat.DEFAULT_ALL)
        } else {
            builder.setPriority(NotificationCompat.PRIORITY_DEFAULT)
        }

        // **Posting is allowed to fail and must not crash anything.** From
        // Android 13 the permission may simply not have been granted, and a
        // reminder nobody sees is a disappointment where a crash on a background
        // alarm is a bug report nobody can reproduce.
        runCatching {
            NotificationManagerCompat.from(context)
                .notify(notificationId(row.id, kind), builder.build())
        }.onFailure { Log.w("Reminders", "the notification was refused", it) }
    }

    private fun ensureChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return

        manager.createNotificationChannel(
            NotificationChannel(
                MEETING_CHANNEL,
                "Meetings",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "When a meeting you arranged with a contact comes round."
                enableVibration(true)
            },
        )
        manager.createNotificationChannel(
            NotificationChannel(
                FOLLOW_UP_CHANNEL,
                "Follow-ups",
                NotificationManager.IMPORTANCE_DEFAULT,
            ).apply {
                description = "When a contact you meant to chase comes due."
            },
        )
    }
}
