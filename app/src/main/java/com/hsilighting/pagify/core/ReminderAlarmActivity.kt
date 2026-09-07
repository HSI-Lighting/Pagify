package com.hsilighting.pagify.core

import android.app.KeyguardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
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
 * The face of a meeting alarm: who, when, and the two ways out.
 *
 * **It does not ring.** It used to, and that was the whole bug —
 * [ReminderAlarmService] holds the sound now. A screen is a window, and a window
 * is something Android grants or refuses; with the app closed it refuses, and
 * everything hung off it went quiet with it. What is left here is what a window
 * is actually good for: showing over the lock, turning the display on, and
 * putting two large targets under a thumb.
 *
 * Dealt with, or ten more minutes. An alarm you can only silence teaches people
 * to silence alarms, and the meeting is still in the diary either way.
 *
 * Follow-ups never reach here. They stay a quiet line in the shade, which is
 * what a nudge deserves.
 */
class ReminderAlarmActivity : ComponentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        showOverLockScreen()

        val contactId = intent.getLongExtra(Reminders.EXTRA_CONTACT, -1L)
        val meetingId = intent.getLongExtra(Reminders.EXTRA_MEETING, 0L)
        val who = intent.getStringExtra(EXTRA_WHO).orEmpty().ifBlank { "a contact" }
        val where = intent.getStringExtra(EXTRA_WHERE).orEmpty()
        val at = intent.getLongExtra(EXTRA_AT, 0L)

        setContent {
            PagifyTheme {
                AlarmFace(
                    who = who,
                    where = where,
                    at = at,
                    onDone = {

                        sendBroadcast(
                            Intent(this, ReminderReceiver::class.java)
                                .setAction(Reminders.ACTION_DONE)
                                .putExtra(Reminders.EXTRA_CONTACT, contactId)
                                .putExtra(Reminders.EXTRA_MEETING, meetingId)
                                .putExtra(Reminders.EXTRA_KIND, ReminderKind.Meeting.name),
                        )
                        finish()
                    },
                    onSnooze = {

                        sendBroadcast(
                            Intent(this, ReminderReceiver::class.java)
                                .setAction(Reminders.ACTION_SNOOZE)
                                .putExtra(Reminders.EXTRA_CONTACT, contactId)
                                .putExtra(Reminders.EXTRA_MEETING, meetingId),
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

    /**
     * Closing the face quiets the alarm — but only when it is really closing.
     *
     * `isFinishing` rather than plain `onDestroy`, because a rotation destroys
     * this activity too, and an alarm that stops when the phone is turned over
     * is one that can be missed by picking it up.
     */
    override fun onDestroy() {
        if (isFinishing) ReminderAlarmService.stop(this)
        super.onDestroy()
    }

    companion object {
        const val EXTRA_WHO = "who"
        const val EXTRA_WHERE = "where"
        const val EXTRA_AT = "at"

        fun intent(
            context: Context,
            contactId: Long,
            meetingId: Long,
            who: String,
            where: String,
            at: Long,
        ): Intent =
            Intent(context, ReminderAlarmActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                .putExtra(Reminders.EXTRA_CONTACT, contactId)
                .putExtra(Reminders.EXTRA_MEETING, meetingId)
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
