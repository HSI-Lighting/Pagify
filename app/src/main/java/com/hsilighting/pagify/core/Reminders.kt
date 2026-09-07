package com.hsilighting.pagify.core

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
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
 * # A meeting rings; a follow-up does not
 *
 * A **meeting** is an appointment, and it gets the alarm-clock treatment:
 * [ReminderAlarmActivity] takes the screen, shows over the lock, and rings on
 * the alarm stream until it is answered. It was a heads-up banner with the
 * default notification chime, which is the same treatment a message gets and
 * got swiped away with the same reflex — two seconds of sound that a phone face
 * down, in a bag, or in a loud room does not deliver at all. Being late is the
 * entire failure being guarded against, so being ignorable was the wrong
 * design.
 *
 * A **follow-up** is a nudge and stays exactly as it was: a quiet line in the
 * shade. Being a few hours late to one costs nothing, and an alarm for it would
 * teach the user to silence alarms.
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
 * # Exact, since it started ringing
 *
 * This was deliberately inexact while a reminder was a banner, on the reasoning
 * that `SCHEDULE_EXACT_ALARM` can be refused in a settings screen and a feature
 * that dies silently is worse than one that arrives a few minutes late. An
 * alarm inverts that: one that goes off at *about* half past is not an alarm.
 *
 * So the app declares `USE_EXACT_ALARM` instead — granted at install, not
 * revocable, and meant for exactly this — and schedules with `setAlarmClock`,
 * the one form Doze never defers and the one that puts the alarm icon in the
 * status bar. The inexact call survives as the fallback for a phone holding
 * neither permission, where late still beats never.
 */
object Reminders {

    /**
     * The meeting channel, twice over, and why the old id had to go.
     *
     * **A channel's sound and importance are fixed when it is created**, and
     * every later `createNotificationChannel` for the same id is ignored. That
     * is the trap in changing how an alert sounds: the code says alarm, the
     * phone keeps playing the chime it was first told about, and nothing
     * reports the disagreement. So the alarm treatment needs a new id, and the
     * old one is deleted rather than left in the phone's settings as a channel
     * nothing posts to.
     *
     * There are two because which is used depends on whether this app is
     * allowed to take over the screen. When it is, [ReminderAlarmActivity] does
     * the ringing and the notification behind it stays silent — otherwise the
     * phone would play the tone once while the activity looped it. When it is
     * not, the fallback has to make the noise by itself, so it carries the alarm
     * tone and the vibration.
     */
    private const val MEETING_ALARM_CHANNEL = "contact-meetings-alarm"
    private const val MEETING_LOUD_CHANNEL = "contact-meetings-loud"
    private const val LEGACY_MEETING_CHANNEL = "contact-meetings"
    private const val FOLLOW_UP_CHANNEL = "contact-follow-ups"
    private const val ALARM_REQUEST = 4711

    const val ACTION_FIRE = "com.hsilighting.pagify.REMINDER"
    const val ACTION_DONE = "com.hsilighting.pagify.REMINDER_DONE"
    const val ACTION_SNOOZE = "com.hsilighting.pagify.REMINDER_SNOOZE"
    const val EXTRA_CONTACT = "contactId"
    const val EXTRA_KIND = "kind"

    /** What "ten more minutes" means. */
    const val SNOOZE_MILLIS = 600_000L

    /**
     * Look at what is due, say so, and set the alarm for what is next.
     *
     * Safe from anywhere and at any time — after a save, after a boot, after the
     * alarm itself. It reads the current state rather than trusting what it was
     * told, so a missed alarm and a duplicated one come out the same.
     */
    suspend fun reschedule(context: Context, notify: Boolean, ring: Boolean = false) {
        val application = context.applicationContext
        val dao = ContactsDatabase.get(application).contacts()
        val now = System.currentTimeMillis()

        // **Here rather than at the first post.** Channels used to be made the
        // first time something was due, which left an upgraded phone carrying
        // the old chime channel in its settings — visible, apparently
        // adjustable, and attached to nothing — until the next meeting came
        // round. This runs after every save and every boot, so the swap happens
        // as soon as the app does anything at all.
        ensureChannels(application)

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
                    post(application, row, ReminderKind.Meeting, ring)
                }
                if (row.followUpAt != null && row.followUpDoneAt == null && row.followUpAt <= now) {
                    post(application, row, ReminderKind.FollowUp, ring = false)
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

        // **Exact, now that it rings.** This was deliberately inexact while a
        // reminder was a banner: a few minutes either side of a nudge costs
        // nothing, and `SCHEDULE_EXACT_ALARM` is a permission the user can
        // refuse, which would have left the feature silently dead. An alarm for
        // a meeting is the other case entirely — one that goes off at "about"
        // half past is not an alarm — so the app now declares `USE_EXACT_ALARM`,
        // which is granted at install and cannot be taken away.
        //
        // `setAlarmClock` rather than `setExactAndAllowWhileIdle`: it is the one
        // form Doze never defers, and it is the one that puts the alarm icon in
        // the status bar, which is a promise to the user that something is set.
        // The inexact form is still here for the case where neither permission
        // is held — late is better than never.
        runCatching {
            if (Build.VERSION.SDK_INT < 31 || alarms.canScheduleExactAlarms()) {
                alarms.setAlarmClock(AlarmManager.AlarmClockInfo(nextAt, showPendingIntent(application)), pending)
            } else {
                alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, nextAt, pending)
            }
        }.onFailure { Log.w("Reminders", "the alarm could not be set", it) }
    }

    /**
     * Push one meeting back by [SNOOZE_MILLIS] and re-arm.
     *
     * The time itself is moved rather than a second alarm being kept alongside
     * it. A snooze held separately is a reminder the calendar does not know
     * about: the day cell would still mark the original hour, and the contact
     * would still read as due at a time nothing was going to fire.
     */
    suspend fun snooze(context: Context, contactId: Long) {
        val application = context.applicationContext
        val dao = ContactsDatabase.get(application).contacts()
        val row = runCatching { dao.contactsById(listOf(contactId)) }.getOrNull()?.firstOrNull()
            ?: return

        runCatching {
            dao.setProgress(
                id = contactId,
                stage = row.stage,
                met = row.met,
                followUpAt = row.followUpAt,
                followUpDoneAt = row.followUpDoneAt,
                meetingAt = System.currentTimeMillis() + SNOOZE_MILLIS,
                meetingDoneAt = null,
            )
        }.onFailure { Log.w("Reminders", "could not put the meeting off", it) }

        NotificationManagerCompat.from(application)
            .cancel(notificationId(contactId, ReminderKind.Meeting))
        reschedule(application, notify = false)
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

    /**
     * Where the status bar's alarm icon leads when tapped.
     *
     * `AlarmClockInfo` asks for this separately from the alarm itself, and it is
     * what makes the icon a thing you can follow rather than a mystery — it
     * opens the app, where the reminder can be seen and changed.
     */
    private fun showPendingIntent(context: Context): PendingIntent = PendingIntent.getActivity(
        context,
        ALARM_REQUEST + 1,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

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

    /**
     * Whether a full-screen alarm is allowed here.
     *
     * From Android 14 this is a permission in its own right, held by alarm and
     * calling apps and revocable in settings, so it has to be asked rather than
     * assumed — a full-screen intent that is not allowed does not fail, it
     * quietly demotes itself to an ordinary banner, which is the silence this
     * whole change exists to fix.
     */
    private fun canUseFullScreenIntent(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < 34) return true
        val manager = context.getSystemService(NotificationManager::class.java) ?: return false
        return runCatching { manager.canUseFullScreenIntent() }.getOrDefault(false)
    }

    /**
     * Say one reminder out loud.
     *
     * @param ring whether this is the alarm actually going off, as opposed to a
     *   catch-up pass. **Only the real moment takes the screen.** A reboot and a
     *   reinstall both re-read what is due and both would otherwise throw a
     *   full-screen alarm at somebody who was only updating the app — and the
     *   one after a reinstall would arrive every single time. A catch-up still
     *   says the meeting was missed; it just says it in the shade.
     */
    private fun post(context: Context, row: ContactRow, kind: ReminderKind, ring: Boolean) {
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

        // Whether this phone will let the app put a screen in front of the user.
        // From Android 14 that is a permission of its own, revocable in
        // settings, and asking rather than assuming is what decides which of the
        // two meeting channels is used — and so whether the notification itself
        // has to be the thing that makes a noise.
        val canTakeTheScreen = isMeeting && ring && canUseFullScreenIntent(context)

        val builder = NotificationCompat.Builder(
            context,
            when {
                !isMeeting -> FOLLOW_UP_CHANNEL
                canTakeTheScreen -> MEETING_ALARM_CHANNEL
                else -> MEETING_LOUD_CHANNEL
            },
        )
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(if (isMeeting) "Meeting with $who" else "Follow up with $who")
            .setContentText(
                row.company.takeIf { it.isNotBlank() && it != who }
                    ?: if (isMeeting) "Starting now" else "Time to get back to them",
            )
            .setCategory(if (isMeeting) Notification.CATEGORY_ALARM else Notification.CATEGORY_REMINDER)
            .setAutoCancel(true)
            .setContentIntent(open)
            .addAction(0, "Done", done)

        // A meeting is loud whether or not it gets the screen — a catch-up after
        // a reboot is still a meeting that was missed, and it has to arrive
        // ahead of the day's messages rather than among them.
        builder.setPriority(
            if (isMeeting) NotificationCompat.PRIORITY_MAX else NotificationCompat.PRIORITY_DEFAULT,
        )

        if (canTakeTheScreen) {
            val alarmFace = ReminderAlarmActivity.intent(
                context = context,
                contactId = row.id,
                who = who,
                where = row.company.takeIf { it != who }.orEmpty(),
                at = row.meetingAt ?: System.currentTimeMillis(),
            )
            val fullScreen = PendingIntent.getActivity(
                context,
                notificationId(row.id, kind) + 200_000,
                alarmFace,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )

            // Ongoing, because an alarm that can be flicked off the shade with
            // the same reflex as an advert is not an alarm. It goes when it has
            // been answered.
            builder.setOngoing(true)
                .setAutoCancel(false)
                .setFullScreenIntent(fullScreen, true)

            // **And started directly, as well as through the notification.**
            // Android turns a full-screen intent into an ordinary heads-up
            // whenever the phone is unlocked and in use — which is most of the
            // working day, and exactly when a meeting alert matters. An alarm
            // broadcast buys the app a few seconds in which it may start an
            // activity from the background; this spends them. Both routes land
            // in the same task, so whichever arrives second finds it already up.
            runCatching { context.startActivity(alarmFace) }
                .onFailure { Log.w("Reminders", "the alarm screen would not open", it) }
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

        // The chime this used to be. Deleted rather than left behind: it would
        // otherwise sit in the phone's notification settings looking like
        // something that can be turned on, and turning it on would do nothing.
        runCatching { manager.deleteNotificationChannel(LEGACY_MEETING_CHANNEL) }

        manager.createNotificationChannel(
            NotificationChannel(
                MEETING_ALARM_CHANNEL,
                "Meetings",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "When a meeting comes round. Takes the screen and rings."
                // Silent on purpose — the alarm screen does the ringing. Giving
                // this one a tone as well would play it once underneath a
                // looping alarm, which sounds like a fault.
                setSound(null, null)
                enableVibration(false)
            },
        )

        manager.createNotificationChannel(
            NotificationChannel(
                MEETING_LOUD_CHANNEL,
                "Meetings (sound only)",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description =
                    "Used instead when Pagify is not allowed to show a full-screen alarm."
                // The alarm stream, not the notification one. It is what stays
                // audible with the ringer down — the reason a phone on silent
                // still wakes you in the morning — and a meeting has the same
                // claim on being heard as any other alarm.
                setSound(
                    RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM)
                        ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION),
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_ALARM)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                        .build(),
                )
                enableVibration(true)
                vibrationPattern = longArrayOf(0, 600, 600, 600, 600)
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
