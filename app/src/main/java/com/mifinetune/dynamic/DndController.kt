package com.mifinetune.dynamic

import android.app.NotificationManager
import android.content.Context
import android.util.Log

/**
 * The only Android-platform-API executor in the app: per-app DND is decided
 * by the Rust daemon (`dnd` events) but must be applied through
 * `NotificationManager` with user-granted DND access — `zen_mode` itself is
 * framework-owned (etag-tracked) and is never written directly.
 *
 * State is persisted so a service restart can reconcile a filter we left on;
 * the restore only fires when the current filter is still the one we set
 * (never clobbers a user/MIUI change made in the meantime).
 */
class DndController(private val context: Context) {

    companion object {
        private const val TAG = "MiFineTune"
        private const val PREFS = "dnd_state"
        private const val K_APPLIED = "applied_filter"
        private const val K_SAVED = "saved_filter"
        private const val NONE = -1
    }

    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun nm(): NotificationManager? = runCatching {
        context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    }.getOrNull()

    fun isGranted(): Boolean = runCatching {
        nm()?.isNotificationPolicyAccessGranted == true
    }.getOrDefault(false)

    /** Applies a daemon-requested mode; idempotent. False = access missing. */
    fun apply(mode: String): Boolean {
        val manager = nm() ?: return false
        if (!isGranted()) return false
        val filter = when (mode) {
            "priority" -> NotificationManager.INTERRUPTION_FILTER_PRIORITY
            "total" -> NotificationManager.INTERRUPTION_FILTER_NONE
            else -> return false
        }
        val applied = prefs.getInt(K_APPLIED, NONE)
        if (applied == filter) return true
        if (applied == NONE) {
            // first apply ever: capture the user's filter as the restore point
            prefs.edit().putInt(K_SAVED, manager.currentInterruptionFilter).apply()
        }
        val ok = runCatching {
            manager.setInterruptionFilter(filter)
            true
        }.getOrDefault(false)
        if (ok) {
            prefs.edit().putInt(K_APPLIED, filter).apply()
            Log.d(TAG, "dnd applied: $mode")
        } else {
            Log.w(TAG, "dnd apply failed: $mode")
        }
        return ok
    }

    /** Restores the captured filter when the current one is still ours. */
    fun restore() {
        val applied = prefs.getInt(K_APPLIED, NONE)
        if (applied == NONE) return
        val saved = prefs.getInt(K_SAVED, NotificationManager.INTERRUPTION_FILTER_ALL)
        val manager = nm()
        val current = runCatching { manager?.currentInterruptionFilter }.getOrNull()
        if (manager != null && current == applied) {
            runCatching { manager.setInterruptionFilter(saved) }
            Log.d(TAG, "dnd restored: $saved")
        }
        prefs.edit().remove(K_APPLIED).remove(K_SAVED).apply()
    }
}
