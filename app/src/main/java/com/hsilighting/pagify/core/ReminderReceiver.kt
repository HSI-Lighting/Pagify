package com.hsilighting.pagify.core

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * The alarm going off, and the phone coming back on.
 *
 * Both do the same thing — read what is due, say so, and set the next alarm —
 * because both are the same question asked at a different moment. A boot is not
 * a special case; it is simply the point at which every alarm the system was
 * holding has been forgotten and has to be asked for again.
 *
 * **Without the boot half, reminders silently stop at the first restart.** The
 * alarm is gone, nothing reports it, and the failure only shows up as a
 * follow-up that never happened — weeks later, if at all.
 */
class ReminderReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        val action = intent.action
        val isBoot = action == Intent.ACTION_BOOT_COMPLETED ||
            action == "android.intent.action.QUICKBOOT_POWERON" ||
            action == Intent.ACTION_MY_PACKAGE_REPLACED
        if (action != Reminders.ACTION_FIRE && !isBoot) return

        // The work outlives this call, so the process must be asked to stay
        // alive for it. Without this the read can be killed halfway and the
        // next alarm never gets set — the same silent stop as a missing boot
        // receiver, but intermittent.
        val finish = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                // A boot notifies too. Anything that came due while the phone
                // was off is still due, and staying quiet about it would lose
                // exactly the reminders that waited longest.
                Reminders.reschedule(context, notify = true)
            } catch (error: Throwable) {
                Log.w("Reminders", "the reminder pass failed", error)
            } finally {
                finish.finish()
            }
        }
    }
}
