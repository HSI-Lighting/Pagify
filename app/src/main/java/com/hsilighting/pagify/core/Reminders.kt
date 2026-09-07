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
import androidx.core.content.ContextCompat
import androidx.core.app.NotificationManagerCompat
import com.hsilighting.pagify.MainActivity
import com.hsilighting.pagify.R
import com.hsilighting.pagify.data.db.ContactRow
import com.hsilighting.pagify.data.db.ContactsDatabase
import com.hsilighting.pagify.data.db.MeetingRow
import com.hsilighting.pagify.data.db.ReminderKind

/**
 * Getting a reminder in front of somebody who has closed the app.
 *
 * # A meeting rings; a follow-up does not
 *
 * A **meeting** is an appointment, and it gets the alarm-clock treatment:
 * [ReminderAlarmService] rings on the alarm stream until it is answered, and
 * [ReminderAlarmActivity] shows the face over the lock screen when the system
 * lets it. It was a heads-up banner with the default notification chime, which
 * is the same treatment a message gets and got swiped away with the same reflex
 * — two seconds of sound that a phone face down, in a bag, or in a loud room
 * does not deliver at all. Being late is the entire failure being guarded
 * against, so being ignorable was the wrong design.
 *
 * **The sound is in the service and not in the screen, and that distinction is
 * the whole feature.** The first version rang from the activity and launched it
 * straight from the alarm broadcast; with the app closed Android answered that
 * launch with `BAL_BLOCK`, and since the notification behind it was deliberately
 * silent, a closed app got no alert whatsoever. A foreground service can be
 * started from an alarm broadcast where an activity cannot, so the noise lives
 * somewhere that does not need permission to appear.
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
     * There are two because which is used depends on whether the ringing
     * service came up. When it did, it is doing the noise and the notification
     * behind it stays silent — a tone here as well would play once underneath a
     * looping alarm, which sounds like a fault. When it did not, this fallback
     * is the entire alert, so it carries the alarm tone and the vibration
     * itself. Neither case leaves a meeting announced in silence.
     */
    /**
     * The channels, named here so Settings can open the phone's own controls
     * for them.
     *
     * Public because sound, vibration and importance belong to Android, not to
     * this app: a channel's settings are fixed at creation and every later
     * change from code is ignored, so an in-app sound picker would set a value
     * the phone never plays. Deep-linking to the system screen is not a
     * shortcut here -- it is the only thing that works.
     */
    const val MEETING_ALARM_CHANNEL = "contact-meetings-alarm"
    const val MEETING_LOUD_CHANNEL = "contact-meetings-loud"
    private const val LEGACY_MEETING_CHANNEL = "contact-meetings"
    const val FOLLOW_UP_CHANNEL = "contact-follow-ups"
    private const val ALARM_REQUEST = 4711

    const val ACTION_FIRE = "com.hsilighting.pagify.REMINDER"
    const val ACTION_DONE = "com.hsilighting.pagify.REMINDER_DONE"
    const val ACTION_SNOOZE = "com.hsilighting.pagify.REMINDER_SNOOZE"
    const val EXTRA_CONTACT = "contactId"
    const val EXTRA_MEETING = "meetingId"
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
            val chases = runCatching { dao.dueFollowUps(now) }.getOrElse {
                Log.w("Reminders", "could not read what is due", it)
                emptyList()
            }
            val meetings = runCatching { dao.dueMeetings(now) }.getOrElse {
                Log.w("Reminders", "could not read the meetings due", it)
                emptyList()
            }
            val people = runCatching { dao.contactsById(meetings.map { it.contactId }.distinct()) }
                .getOrElse { emptyList() }
                .associateBy { it.id }

            // One notification per meeting rather than per contact: two
            // appointments with the same person on the same day are two
            // things to be at, and collapsing them would silently drop one.
            meetings.forEach { meeting ->
                val row = people[meeting.contactId] ?: return@forEach
                post(application, row, ReminderKind.Meeting, ring, meeting)
            }
            chases.forEach { row ->
                post(application, row, ReminderKind.FollowUp, ring = false)
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
    suspend fun snooze(context: Context, meetingId: Long) {
        val application = context.applicationContext
        val dao = ContactsDatabase.get(application).contacts()
        val meeting = runCatching { dao.meetingById(meetingId) }.getOrNull() ?: return

        runCatching {
            dao.moveMeeting(meetingId, System.currentTimeMillis() + SNOOZE_MILLIS)
        }.onFailure { Log.w("Reminders", "could not put the meeting off", it) }

        ReminderAlarmService.stop(application)
        NotificationManagerCompat.from(application)
            .cancel(meetingNotificationId(meetingId))
        reschedule(application, notify = false)
    }
    /**
     * Mark one reminder dealt with, from the notification's own button.
     *
     * A meeting is named by its own id rather than by whose it is: a contact
     * can have several, and finishing one must not finish the rest.
     */
    suspend fun markDone(
        context: Context,
        contactId: Long,
        kind: ReminderKind,
        meetingId: Long = 0,
    ) {
        val application = context.applicationContext
        val dao = ContactsDatabase.get(application).contacts()
        val now = System.currentTimeMillis()

        if (kind == ReminderKind.Meeting) {
            if (meetingId <= 0) return
            runCatching { dao.markMeetingDone(meetingId, now) }
                .onFailure { Log.w("Reminders", "could not mark the meeting done", it) }
            ReminderAlarmService.stop(application)
            NotificationManagerCompat.from(application).cancel(meetingNotificationId(meetingId))
            reschedule(application, notify = false)
            return
        }

        val row = runCatching { dao.contactsById(listOf(contactId)) }.getOrNull()?.firstOrNull()
            ?: return
        runCatching {
            dao.setProgress(
                id = contactId,
                stage = row.stage,
                met = row.met,
                followUpAt = row.followUpAt,
                followUpDoneAt = now,
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
     * A follow-up's notification id: stable per contact, and always even.
     *
     * Re-posting replaces itself rather than stacking a new one up every time
     * an alarm fires.
     */
    private fun notificationId(contactId: Long, kind: ReminderKind): Int =
        (contactId.toInt() * 2) + if (kind == ReminderKind.Meeting) 1 else 0

    /**
     * The id a meeting posts under.
     *
     * **Keyed by the meeting, not by whose it is.** Two appointments with the
     * same person would otherwise share one id, and the second would replace the
     * first in the shade — the same collapse the single `meetingAt` column used
     * to do in the database. Odd numbers here, even ones for follow-ups, so the
     * two spaces cannot meet.
     */
    internal fun meetingNotificationId(meetingId: Long): Int = (meetingId.toInt() * 2) + 1

    /**
     * Hand the ringing to [ReminderAlarmService].
     *
     * @return whether it was taken. **A foreground service can be started from
     *   an alarm broadcast where an activity cannot** — that exemption is the
     *   one piece of this that is real, and everything about the alert now hangs
     *   off it. If it is refused anyway, the caller falls back to a notification
     *   that carries the alarm tone itself, so the worst case is a single tone
     *   rather than none.
     */
    private fun startAlarmService(
        context: Context,
        row: ContactRow,
        meeting: MeetingRow,
        who: String,
    ): Boolean =
        runCatching {
            ContextCompat.startForegroundService(
                context,
                ReminderAlarmService.intent(
                    context = context,
                    contactId = row.id,
                    meetingId = meeting.id,
                    who = who,
                    where = row.company.takeIf { it != who }.orEmpty(),
                    at = meeting.at,
                ),
            )
            true
        }.getOrElse {
            Log.w("Reminders", "the alarm service would not start", it)
            false
        }

    /**
     * The notification the ringing service shows while it rings.
     *
     * Silent by channel, because the service is already making the noise, and
     * carrying the full-screen intent, which is what puts the alarm face in
     * front of a locked phone. Ongoing, because an alarm that clears with the
     * same flick as an advert is not an alarm.
     */
    internal fun alarmNotification(
        context: Context,
        contactId: Long,
        meetingId: Long,
        who: String,
        where: String,
        at: Long,
    ): Notification {
        ensureChannels(context)
        val id = meetingNotificationId(meetingId)
        val alarmFace = ReminderAlarmActivity.intent(context, contactId, meetingId, who, where, at)

        return NotificationCompat.Builder(context, MEETING_ALARM_CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("Meeting with ${who.ifBlank { "a contact" }}")
            .setContentText(where.ifBlank { "Starting now" })
            .setCategory(Notification.CATEGORY_ALARM)
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setOngoing(true)
            .setAutoCancel(false)
            .setContentIntent(
                PendingIntent.getActivity(
                    context,
                    id + 200_000,
                    alarmFace,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ),
            )
            .setFullScreenIntent(
                PendingIntent.getActivity(
                    context,
                    id + 300_000,
                    alarmFace,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ),
                true,
            )
            .addAction(
                0,
                "Done",
                PendingIntent.getBroadcast(
                    context,
                    id + 100_000,
                    Intent(context, ReminderReceiver::class.java)
                        .setAction(ACTION_DONE)
                        .putExtra(EXTRA_CONTACT, contactId)
                        .putExtra(EXTRA_MEETING, meetingId)
                        .putExtra(EXTRA_KIND, ReminderKind.Meeting.name),
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ),
            )
            .addAction(
                0,
                "Ten more minutes",
                PendingIntent.getBroadcast(
                    context,
                    id + 400_000,
                    Intent(context, ReminderReceiver::class.java)
                        .setAction(ACTION_SNOOZE)
                        .putExtra(EXTRA_CONTACT, contactId)
                        .putExtra(EXTRA_MEETING, meetingId),
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ),
            )
            .build()
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
    private fun post(
        context: Context,
        row: ContactRow,
        kind: ReminderKind,
        ring: Boolean,
        meeting: MeetingRow? = null,
    ) {
        val who = row.name.ifBlank { row.company }.ifBlank { "a contact" }
        val isMeeting = kind == ReminderKind.Meeting
        // A meeting is identified by itself; a follow-up by whose it is.
        val postId =
            if (isMeeting && meeting != null) meetingNotificationId(meeting.id)
            else notificationId(row.id, kind)

        val open = PendingIntent.getActivity(
            context,
            postId,
            Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val done = PendingIntent.getBroadcast(
            context,
            postId + 100_000,
            Intent(context, ReminderReceiver::class.java)
                .setAction(ACTION_DONE)
                .putExtra(EXTRA_CONTACT, row.id)
                .putExtra(EXTRA_MEETING, meeting?.id ?: 0L)
                .putExtra(EXTRA_KIND, kind.name),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

        // **The ringing is the service's job, and it either takes it or the
        // notification has to.** Nothing here assumes a window will be granted:
        // that assumption is what made a closed app silent. If the service comes
        // up it posts its own notification on the silent channel and rings; if
        // it cannot, the noisy channel below is the whole alert.
        val ringing = isMeeting && ring && meeting != null &&
            startAlarmService(context, row, meeting, who)
        if (ringing) return

        val builder = NotificationCompat.Builder(
            context,
            when {
                !isMeeting -> FOLLOW_UP_CHANNEL
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

        // **Posting is allowed to fail and must not crash anything.** From
        // Android 13 the permission may simply not have been granted, and a
        // reminder nobody sees is a disappointment where a crash on a background
        // alarm is a bug report nobody can reproduce.
        runCatching {
            NotificationManagerCompat.from(context)
                .notify(postId, builder.build())
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
