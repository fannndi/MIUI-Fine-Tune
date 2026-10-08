package com.mifinetune.automation

import android.app.KeyguardManager
import android.content.Context
import android.os.PowerManager
import android.util.Log
import com.mifinetune.core.RootBridge
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Screen/keyguard context readers for the arbiter.
 *
 * Responsibility: turning Android services into plain values.
 * Non-goals: decisions (ModeArbiter), lifecycle (AutomationService).
 */
class DeviceContextReader(private val context: Context) {

    private val keyguard =
        context.getSystemService(Context.KEYGUARD_SERVICE) as KeyguardManager
    private val power =
        context.getSystemService(Context.POWER_SERVICE) as PowerManager

    val screenOn: Boolean get() = power.isInteractive
    val keyguardLocked: Boolean get() = keyguard.isKeyguardLocked
}

/**
 * Foreground detection, event-driven:
 *
 *  - Primary: a root `logcat -b events -s am_resume_activity:V` stream.
 *    Every activity resume emits an event line within the same second —
 *    instant, reliable and cheap (no polling).
 *  - Fallback: a one-shot `dumpsys window` root peek (used to seed the
 *    foreground after wake/unlock and while the stream is down).
 */
class ForegroundWatcher(private val bridge: RootBridge) {

    companion object {
        private const val TAG = "MiFineTune"
        private const val MAX_PEEK_AGE_MS = 60_000L

        /**
         * Two tags carry the focused component on this ROM:
         *  - `am_resume_activity: [user, token, task, pkg/class, pid]`
         *    (task-level resume, the common launcher path)
         *  - `am_set_resumed_activity: [user, pkg/class, reason]`
         *    (some transitions — e.g. monkey/new-task launches — log ONLY
         *    this one; verified 2026-10-08: Azur Lane start emitted
         *    am_set_resumed_activity with no am_resume_activity at all)
         */
        private val RESUME_RE = Regex("am_resume_activity: \\[\\d+,\\d+,\\d+,([^\\s/]+)/")
        private val SET_RESUMED_RE = Regex("am_set_resumed_activity: \\[\\d+,([^\\s/]+)/")
        private val ANY_RE = Regex("(am_resume_activity|am_set_resumed_activity): ")
    }

    private var process: Process? = null
    private var readerJob: Job? = null

    /** True while the event stream is up. */
    val isAlive: Boolean get() = process?.isAlive == true

    /**
     * Starts streaming resume events; [onPackage] fires for every resumed
     * package (duplicates included — the caller dedupes). Safe to call again
     * to restart after a death.
     */
    fun start(scope: CoroutineScope, onPackage: (String) -> Unit) {
        stop()
        val p = runCatching {
            bridge.stream("logcat -b events -s am_resume_activity:V am_set_resumed_activity:V")
        }.getOrNull() ?: run {
            Log.w(TAG, "foreground watcher: failed to spawn logcat")
            return
        }
        process = p
        Log.d(TAG, "watcher started")
        readerJob = scope.launch(Dispatchers.IO) {
            try {
                p.inputStream.bufferedReader().useLines { lines ->
                    for (line in lines) {
                        if (!isActive) break
                        if (ANY_RE.find(line) == null) continue
                        val pkg = RESUME_RE.find(line)?.groupValues?.get(1)
                            ?: SET_RESUMED_RE.find(line)?.groupValues?.get(1)
                            ?: continue
                        onPackage(pkg)
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "foreground watcher stream ended: $e")
            } finally {
                if (process === p) process = null
            }
        }
    }

    fun stop() {
        readerJob?.cancel()
        readerJob = null
        process?.destroy()
        process = null
    }

    /** One-shot root peek of the focused window (fallback + wake seeding). */
    /**
     * One-shot peek of the most recent resume event via the event-log buffer.
     * Uses `logcat` (kernel buffer) on purpose: `dumpsys` (binder) is not
     * usable from this app's su context on APatch, while logcat works.
     * Only events younger than [MAX_PEEK_AGE_MS] are trusted — the buffer
     * keeps hours of history, and a stale resume (e.g. last night's game)
     * must not seed the first decision.
     */
    fun peekEvents(): String? {
        val out = runCatching {
            bridge.sh("logcat -b events -d -s am_resume_activity:V am_set_resumed_activity:V").out
        }.getOrDefault("")
        val pkg = out.lineSequence()
            .lastOrNull { ANY_RE.find(it) != null && isFresh(it) }
            ?.let { RESUME_RE.find(it)?.groupValues?.get(1) ?: SET_RESUMED_RE.find(it)?.groupValues?.get(1) }
        Log.d(TAG, "peek: pkg=$pkg (len=${out.length})")
        return pkg
    }

    private fun isFresh(line: String): Boolean = runCatching {
        val m = Regex("(\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2})").find(line) ?: return false
        val t = java.text.SimpleDateFormat("MM-dd HH:mm:ss", java.util.Locale.US).parse(m.groupValues[1])
        val age = System.currentTimeMillis() - (t?.time ?: return false)
        age in 0..MAX_PEEK_AGE_MS
    }.getOrDefault(false)
}
