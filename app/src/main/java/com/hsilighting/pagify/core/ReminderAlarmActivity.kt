package com.hsilighting.pagify.core

import android.app.KeyguardManager
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.media.RingtoneManager
import android.os.Build
import android.os.Bundle
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Log
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.data.db.ReminderKind
import com.hsilighting.pagify.ui.theme.PagifyTheme
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * A meeting going off, taking the whole screen.
 *
 * A meeting alert used to be a banner with the default notification sound —
 * which is the same treatment a message gets, and gets swiped away with the same
 * reflex. It went out in a two-second chime that a phone face down on a table,
 * or in a bag, or on a desk in a loud room, does not deliver at all. A meeting
 * you are about to be late for is not a message.
 *
 * So this is the alarm-clock treatment instead: it takes the screen, it shows
 * over the lock without the phone being unlocked, it turns the display on, and
 * it **keeps ringing** on the alarm stream until somebody answers it. Two ways
 * out — dealt with, or ten more minutes — because an alarm you can only silence
 * teaches people to silence it.
 *
 * It rings on `USAGE_ALARM` deliberately. That is the stream that stays audible
 * when the ringer is down or silenced, which is the whole reason a phone on
 * silent still wakes you up in the morning; a reminder set for a meeting has the
 * same claim on being heard. Follow-ups do not get any of this — they are still
 * a quiet line in the shade, which is what a nudge deserves.
 *
 * It stops itself after [RING_LIMIT_MILLIS]. An alarm nobody is there to answer
 * should not flatten the battery, and the notification stays behind to say it
 * happened.
 */
class ReminderAlarmActivity : ComponentActivity() {

    private var player: MediaPlayer? = null
    private var vibrator: Vibrator? = null
    private val stopSoon = Runnable { stopRinging() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        showOverLockScreen()

        val contactId = intent.getLongExtra(Reminders.EXTRA_CONTACT, -1L)
        val who = intent.getStringExtra(EXTRA_WHO).orEmpty().ifBlank { "a contact" }
        val where = intent.getStringExtra(EXTRA_WHERE).orEmpty()
        val at = intent.getLongExtra(EXTRA_AT, 0L)

        startRinging()

        setContent {
            PagifyTheme {
                AlarmFace(
                    who = who,
                    where = where,
                    at = at,
                    onDone = {
                        stopRinging()
                        sendBroadcast(
                            Intent(this, ReminderReceiver::class.java)
                                .setAction(Reminders.ACTION_DONE)
                                .putExtra(Reminders.EXTRA_CONTACT, contactId)
                                .putExtra(Reminders.EXTRA_KIND, ReminderKind.Meeting.name),
                        )
                        finish()
                    },
                    onSnooze = {
                        stopRinging()
                        sendBroadcast(
                            Intent(this, ReminderReceiver::class.java)
                                .setAction(Reminders.ACTION_SNOOZE)
                                .putExtra(Reminders.EXTRA_CONTACT, contactId),
                        )
                        finish()
                    },
                )
            }
        }
    }

    /**
     * On top of the lock screen, with the display on.
     *
     * The flags and the setters do the same job on either side of API 27, and
     * both are set rather than one being chosen: the flags still work above 27,
     * and some manufacturers' lock screens honour one and not the other.
     */
    private fun showOverLockScreen() {
        if (Build.VERSION.SDK_INT >= 27) {
            setShowWhenLocked(true)
            setTurnScreenOn(true)
            runCatching {
                getSystemService(KeyguardManager::class.java)?.requestDismissKeyguard(this, null)
            }
        }
        @Suppress("DEPRECATION")
        window.addFlags(
            WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or
                WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON or
                WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON,
        )
    }

    private fun startRinging() {
        val tone = RingtoneManager.getActualDefaultRingtoneUri(this, RingtoneManager.TYPE_ALARM)
            ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM)
            ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION)

        // Ringing is the point, but it must never be the reason a reminder
        // crashes: a missing tone, a device with no vibrator, an audio focus
        // refusal. Every part of it is allowed to fail on its own.
        if (tone != null) {
            runCatching {
                player = MediaPlayer().apply {
                    setDataSource(this@ReminderAlarmActivity, tone)
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

        window.decorView.postDelayed(stopSoon, RING_LIMIT_MILLIS)
    }

    private fun stopRinging() {
        window.decorView.removeCallbacks(stopSoon)
        runCatching { player?.stop() }
        runCatching { player?.release() }
        player = null
        runCatching { vibrator?.cancel() }
        vibrator = null
    }

    override fun onDestroy() {
        stopRinging()
        super.onDestroy()
    }

    companion object {
        const val EXTRA_WHO = "who"
        const val EXTRA_WHERE = "where"
        const val EXTRA_AT = "at"

        /** Two minutes, then it gives up and leaves the notification behind. */
        const val RING_LIMIT_MILLIS = 120_000L

        fun intent(context: Context, contactId: Long, who: String, where: String, at: Long): Intent =
            Intent(context, ReminderAlarmActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                .putExtra(Reminders.EXTRA_CONTACT, contactId)
                .putExtra(EXTRA_WHO, who)
                .putExtra(EXTRA_WHERE, where)
                .putExtra(EXTRA_AT, at)
    }
}

@Composable
private fun AlarmFace(
    who: String,
    where: String,
    at: Long,
    onDone: () -> Unit,
    onSnooze: () -> Unit,
) {
    Box(
        Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.surface),
    ) {
        Column(
            Modifier
                .fillMaxSize()
                .safeDrawingPadding()
                .padding(horizontal = 28.dp, vertical = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            Text(
                text = "MEETING",
                style = MaterialTheme.typography.labelLarge,
                fontWeight = FontWeight.Bold,
                color = MaterialTheme.colorScheme.error,
            )
            Spacer(Modifier.height(12.dp))
            Text(
                text = who,
                style = MaterialTheme.typography.displaySmall,
                fontWeight = FontWeight.Bold,
                textAlign = TextAlign.Center,
            )
            if (where.isNotBlank()) {
                Spacer(Modifier.height(6.dp))
                Text(
                    text = where,
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }
            if (at > 0) {
                Spacer(Modifier.height(20.dp))
                Text(
                    // Named the same way the picker that set it names hours, so
                    // the alarm reads back what the user typed rather than its
                    // 24-hour translation.
                    text = SimpleDateFormat(
                        if (uses24Hour(LocalContext.current)) "EEE d MMM, HH:mm" else "EEE d MMM, h:mm a",
                        Locale.getDefault(),
                    ).format(Date(at)),
                    style = MaterialTheme.typography.titleLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            Spacer(Modifier.height(48.dp))

            // The bigger, filled one is the one that ends it. Snooze is the
            // quieter of the two on purpose: it is the answer that leaves the
            // meeting still to deal with.
            Button(
                onClick = onDone,
                shape = RoundedCornerShape(16.dp),
                modifier = Modifier.fillMaxWidth().height(64.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                ),
            ) {
                Text("Done", style = MaterialTheme.typography.titleMedium)
            }
            Spacer(Modifier.height(12.dp))
            OutlinedButton(
                onClick = onSnooze,
                shape = RoundedCornerShape(16.dp),
                modifier = Modifier.fillMaxWidth().height(56.dp),
            ) {
                Text("Ten more minutes")
            }
        }
    }
}
