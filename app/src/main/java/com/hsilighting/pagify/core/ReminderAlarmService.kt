package com.hsilighting.pagify.core

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.media.RingtoneManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Log

/**
 * The thing that actually makes the noise.
 *
 * **The ringing was in the activity, and the activity does not always open.**
 * That was the whole bug, and it is worth writing down because the reasoning
 * behind it was plausible and wrong. The alarm screen was launched straight from
 * the broadcast, on the belief that an app handling an exact alarm is briefly
 * allowed to start an activity from the background. It is not — not on Android
 * 16 with a modern target:
 *
 * ```
 * Background activity launch blocked! ... callingUidProcState: RECEIVER ... BAL_BLOCK
 * ```
 *
 * With the app open the launch went through and the alarm rang. With the app
 * closed the launch was refused, and because the alarm notification was
 * deliberately silent — the screen was supposed to be doing the ringing — the
 * refusal produced no sound at all. A meeting alarm that only works while you
 * are looking at the app is not a meeting alarm.
 *
 * A **foreground service** is the piece that can be started from an alarm
 * broadcast; that exemption is real where the activity one was imagined. So the
 * sound lives here, where it does not depend on a window being granted, and the
 * alarm screen is now only a face on top of it. The screen still gets its chance
 * — the notification carries a full-screen intent, which is what puts it in
 * front of a locked phone — but nothing is silent if it does not appear.
 *
 * `shortService` is the declared type. It fits: the ring stops itself at two
 * minutes, well inside the roughly three the type allows, and it is the one kind
 * of foreground service that asks the user for no permission at all.
 */
class ReminderAlarmService : Service() {

    private var player: MediaPlayer? = null
    private var vibrator: Vibrator? = null
    private val handler = Handler(Looper.getMainLooper())
    private val giveUp = Runnable { stopSelf() }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSelf()
            return START_NOT_STICKY
        }

        val contactId = intent?.getLongExtra(Reminders.EXTRA_CONTACT, -1L) ?: -1L
        val who = intent?.getStringExtra(ReminderAlarmActivity.EXTRA_WHO).orEmpty()
        val where = intent?.getStringExtra(ReminderAlarmActivity.EXTRA_WHERE).orEmpty()
        val at = intent?.getLongExtra(ReminderAlarmActivity.EXTRA_AT, 0L) ?: 0L

        // **First thing, before anything that can fail.** A foreground service
        // that does not call this within a few seconds of being started is killed
        // with an exception, and the reminder would be lost to a crash rather
        // than to silence — which is worse, not better.
        val notification = Reminders.alarmNotification(this, contactId, who, where, at)
        runCatching {
            if (Build.VERSION.SDK_INT >= 34) {
                startForeground(
                    Reminders.meetingNotificationId(contactId),
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE,
                )
            } else {
                startForeground(Reminders.meetingNotificationId(contactId), notification)
            }
        }.onFailure {
            Log.w("Reminders", "the alarm service could not come to the foreground", it)
            stopSelf()
            return START_NOT_STICKY
        }

        startRinging()
        handler.removeCallbacks(giveUp)
        handler.postDelayed(giveUp, RING_LIMIT_MILLIS)

        // Worth one attempt from here even though the same launch was refused
        // from the receiver: a running foreground service is a different process
        // state, and on the phones where it is allowed the alarm takes the
        // screen instead of waiting in a banner. Where it is refused, nothing is
        // lost — it is already ringing.
        if (contactId > 0) {
            runCatching {
                startActivity(ReminderAlarmActivity.intent(this, contactId, who, where, at))
            }.onFailure { Log.i("Reminders", "the alarm screen stayed in its banner", it) }
        }

        return START_NOT_STICKY
    }

    /** The `shortService` deadline. Reached only if the give-up timer did not. */
    override fun onTimeout(startId: Int) {
        stopSelf()
    }

    private fun startRinging() {
        if (player != null) return

        val tone = RingtoneManager.getActualDefaultRingtoneUri(this, RingtoneManager.TYPE_ALARM)
            ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM)
            ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION)

        if (tone != null) {
            runCatching {
                player = MediaPlayer().apply {
                    setDataSource(this@ReminderAlarmService, tone)
                    // The alarm stream, not the notification one. It is what
                    // stays audible with the ringer down — the reason a phone on
                    // silent still wakes you in the morning — and a meeting has
                    // the same claim on being heard.
                    setAudioAttributes(
                        AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_ALARM)
                            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                            .build(),
                    )
                    isLooping = true
                    prepare()
                    start()
                }
            }.onFailure { Log.w("Reminders", "the alarm tone would not play", it) }
        }

        runCatching {
            vibrator = if (Build.VERSION.SDK_INT >= 31) {
                getSystemService(VibratorManager::class.java)?.defaultVibrator
            } else {
                @Suppress("DEPRECATION")
                getSystemService(Vibrator::class.java)
            }
            val pattern = longArrayOf(0, 600, 600)
            if (Build.VERSION.SDK_INT >= 26) {
                vibrator?.vibrate(VibrationEffect.createWaveform(pattern, 0))
            } else {
                @Suppress("DEPRECATION")
                vibrator?.vibrate(pattern, 0)
            }
        }.onFailure { Log.w("Reminders", "the phone would not buzz", it) }
    }

    override fun onDestroy() {
        handler.removeCallbacks(giveUp)
        runCatching { player?.stop() }
        runCatching { player?.release() }
        player = null
        runCatching { vibrator?.cancel() }
        vibrator = null
        super.onDestroy()
    }

    companion object {
        const val ACTION_STOP = "com.hsilighting.pagify.ALARM_STOP"

        /** Two minutes, then it gives up and leaves the notification behind. */
        const val RING_LIMIT_MILLIS = 120_000L

        fun intent(context: Context, contactId: Long, who: String, where: String, at: Long): Intent =
            Intent(context, ReminderAlarmService::class.java)
                .putExtra(Reminders.EXTRA_CONTACT, contactId)
                .putExtra(ReminderAlarmActivity.EXTRA_WHO, who)
                .putExtra(ReminderAlarmActivity.EXTRA_WHERE, where)
                .putExtra(ReminderAlarmActivity.EXTRA_AT, at)

        /** Quiet, from wherever the alarm was answered. */
        fun stop(context: Context) {
            runCatching {
                context.startService(
                    Intent(context, ReminderAlarmService::class.java).setAction(ACTION_STOP),
                )
            }
        }
    }
}
