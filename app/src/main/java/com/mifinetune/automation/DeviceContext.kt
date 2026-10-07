package com.mifinetune.automation

import android.app.KeyguardManager
import android.content.Context
import android.os.Build
import android.os.PowerManager
import android.util.Log
import com.mifinetune.core.RootBridge

/**
 * Screen/keyguard/charging/audio context readers for the arbiter.
 *
 * Responsibility: turning Android services + one root probe into plain values.
 * Non-goals: decisions (ModeArbiter), lifecycle (AutomationService).
 */
class DeviceContextReader(private val context: Context) {

    private val keyguard =
        context.getSystemService(Context.KEYGUARD_SERVICE) as KeyguardManager
    private val power =
        context.getSystemService(Context.POWER_SERVICE) as PowerManager
    private val audio =
        context.getSystemService(Context.AUDIO_SERVICE) as android.media.AudioManager

    val screenOn: Boolean get() = power.isInteractive
    val keyguardLocked: Boolean get() = keyguard.isKeyguardLocked
    val musicActive: Boolean get() = runCatching { audio.isMusicActive }.getOrDefault(false)

    fun charging(): Boolean = runCatching {
        val battery = context.registerReceiver(
            null,
            android.content.IntentFilter(android.content.Intent.ACTION_BATTERY_CHANGED),
        )
        val status = battery?.getIntExtra(android.os.BatteryManager.EXTRA_STATUS, -1) ?: -1
        status == android.os.BatteryManager.BATTERY_STATUS_CHARGING ||
            status == android.os.BatteryManager.BATTERY_STATUS_FULL
    }.getOrDefault(false)
}

/**
 * Foreground-package detector: UsageStats events as the primary source,
 * root `dumpsys window` as fallback.
 *
 * Responsibility: answer "which package is in front" cheaply.
 * Non-goals: deciding what to do with it.
 */
class ForegroundDetector(private val context: Context, private val bridge: RootBridge) {

    companion object {
        private const val TAG = "MiFineTune"
    }

    private val usm =
        context.getSystemService(Context.USAGE_STATS_SERVICE) as android.app.usage.UsageStatsManager

    private var lastTs = System.currentTimeMillis()
    private var startedAt = System.currentTimeMillis()
    private var sawAnyEvent = false

    /** True when UsageStats proved unusable and we poll the root shell. */
    var usingFallback = false
        private set

    /** Grant PACKAGE_USAGE_STATS app-op through root (idempotent). */
    fun ensureUsageAccess(bridge: RootBridge): Boolean {
        val pkg = context.packageName
        val current = runCatching { bridge.sh("appops get $pkg GET_USAGE_STATS") }.getOrNull()
        if (current?.out?.contains("allow") == true) return true
        val set = runCatching { bridge.sh("appops set $pkg GET_USAGE_STATS allow") }.getOrNull()
        val ok = set?.ok == true
        if (!ok) Log.w(TAG, "usage-access grant failed: ${set?.out}")
        return ok
    }

    fun reset() {
        lastTs = System.currentTimeMillis()
    }

    /**
     * Returns the last package that came to the foreground since the previous
     * call, or null when nothing happened.
     */
    fun poll(): String? {
        if (usingFallback) return pollRoot()
        val now = System.currentTimeMillis()
        val events = try {
            usm.queryEvents(lastTs.coerceAtLeast(now - 10_000), now)
        } catch (e: Exception) {
            Log.w(TAG, "UsageStats unavailable, switching to root fallback: $e")
            usingFallback = true
            return pollRoot()
        }
        lastTs = now
        var last: String? = null
        val ev = android.app.usage.UsageEvents.Event()
        while (events.hasNextEvent()) {
            events.getNextEvent(ev)
            if (ev.eventType == EVENT_RESUMED) {
                last = ev.packageName
                sawAnyEvent = true
            }
        }
        // If UsageStats silently returns nothing for a while (some ROMs neuter
        // the app-op), switch to the root fallback.
        if (!sawAnyEvent && now - startedAt > 60_000) {
            Log.w(TAG, "no usage events observed — using root fallback")
            usingFallback = true
        }
        return last
    }

    private fun pollRoot(): String? = runCatching {
        val r = bridge.sh("dumpsys window 2>/dev/null | grep -m1 mCurrentFocus")
        val m = Regex("u\\d+ ([A-Za-z0-9_.]+)/").find(r.out)
        m?.groupValues?.get(1)
    }.getOrNull()

    private val EVENT_RESUMED: Int
        get() = if (Build.VERSION.SDK_INT >= 29) {
            android.app.usage.UsageEvents.Event.ACTIVITY_RESUMED
        } else {
            @Suppress("DEPRECATION")
            android.app.usage.UsageEvents.Event.MOVE_TO_FOREGROUND
        }
}
