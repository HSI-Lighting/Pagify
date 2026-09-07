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
import com.hsilighting.pagify.data.db.ContactsDatabase

/**
 * Getting a reminder in front of somebody who has closed the app.
 *
 * # One alarm, not one per contact
 *
 * Only the *next* reminder ever holds an alarm. When it fires, everything that
 * has come due is posted and the following one is scheduled. A hundred contacts
 * with reminders therefore cost one alarm rather than a hundred — Android caps
 * how many a process may hold, and silently drops the excess, which would show
 * up as reminders that simply never arrive for whoever set the most.
 *
 * The cost is that a reminder set *earlier* than the pending alarm has to
 * reschedule, which is why [reschedule] is called after every save rather than
 * only when a reminder is added.
 *
 * # Inexact on purpose
 *
 * `setAndAllowWhileIdle` rather than `setExactAndAllowWhileIdle`. Exact alarms
 * need `SCHEDULE_EXACT_ALARM`, which on Android 14 is granted by the user in a
 * settings screen and is meant for alarm clocks and calendars with invitations.
 * A reminder to follow up a business card does not need to land on the second,
 * and asking for a permission that can be refused — leaving the feature silently
 * dead — is worse than arriving within a few minutes.
 */
object Reminders {

    private const val CHANNEL = "contact-reminders"
    private const val REQUEST = 4711
    const val ACTION_FIRE = "com.hsilighting.pagify.REMINDER"

    /**
     * Look at what is due, tell the user, and set the alarm for what is next.
     *
     * Safe to call at any time and from anywhere — after a save, after a boot,
     * after the alarm itself. It reads the current state rather than trusting
     * what it was told, so a missed alarm or a duplicated one both come out the
     * same.
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
            if (due.isNotEmpty()) post(application, due.size, due.first().name.ifBlank { due.first().company })
        }

        val next = runCatching { dao.nextReminder(now) }.getOrNull()
        val alarms = application.getSystemService(AlarmManager::class.java) ?: return
        val pending = firePendingIntent(application)

        if (next?.reminderAt == null) {
            // Nothing ahead: cancel rather than leave a stale alarm that would
            // wake the device to find nothing to say.
            alarms.cancel(pending)
            return
        }

        runCatching {
            alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next.reminderAt, pending)
        }.onFailure { Log.w("Reminders", "the alarm could not be set", it) }
    }

    /**
     * Whether anything is due right now, for the badge on the list.
     *
     * Separate from [reschedule] because the list asks this on every change and
     * must not post a notification for doing so.
     */
    suspend fun dueCount(context: Context): Int =
        runCatching {
            ContactsDatabase.get(context.applicationContext)
                .contacts()
                .dueReminders(System.currentTimeMillis())
                .size
        }.getOrDefault(0)

    private fun firePendingIntent(context: Context): PendingIntent {
        val intent = Intent(context, ReminderReceiver::class.java).setAction(ACTION_FIRE)
        return PendingIntent.getBroadcast(
            context,
            REQUEST,
            intent,
            // Mutable would be a security warning and nothing here needs the
            // system to fill anything in.
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }

    private fun post(context: Context, count: Int, firstName: String) {
        ensureChannel(context)

        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

        val name = firstName.ifBlank { "a contact" }
        val text = when (count) {
            1 -> "Follow up with $name"
            2 -> "Follow up with $name and one other"
            else -> "Follow up with $name and ${count - 1} others"
        }

        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("Contacts to follow up")
            .setContentText(text)
            .setCategory(Notification.CATEGORY_REMINDER)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setAutoCancel(true)
            .setContentIntent(open)
            .build()

        // **Posting is allowed to fail and must not crash anything.** On Android
        // 13 and later the user may simply not have granted the permission, and
        // a reminder nobody sees is a disappointment where a crash on a
        // background alarm is a bug report nobody can reproduce.
        runCatching {
            NotificationManagerCompat.from(context).notify(REQUEST, notification)
        }.onFailure { Log.w("Reminders", "the notification was refused", it) }
    }

    private fun ensureChannel(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        val channel = NotificationChannel(
            CHANNEL,
            "Contact reminders",
            NotificationManager.IMPORTANCE_DEFAULT,
        ).apply {
            description = "When a contact you asked to be reminded about comes due."
        }
        // Creating an existing channel is a no-op, so this needs no guard and
        // survives the user having renamed nothing.
        manager.createNotificationChannel(channel)
    }
}
