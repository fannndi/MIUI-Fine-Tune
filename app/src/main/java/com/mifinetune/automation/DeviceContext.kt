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
                        if (!isFresh(line)) continue
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

    /**
     * Age check of a raw logcat line. Built from Calendar (not
     * SimpleDateFormat) on purpose: two-digit-log stamps without a year
     * defaulted to 1970 there, making every line look ~56 years old and
     * disabling the whole stream (verified 2026-10-08). Calendar keeps the
     * parse inside the current year on the same clock.
     */
    private fun isFresh(line: String): Boolean = runCatching {
        val m = Regex("(\\d{2})-(\\d{2}) (\\d{2}):(\\d{2}):(\\d{2})").find(line) ?: return false
        val (mo, dd, hh, mi, ss) = m.destructured
        val cal = java.util.Calendar.getInstance()
        cal.set(
            cal.get(java.util.Calendar.YEAR),
            mo.toInt() - 1,
            dd.toInt(),
            hh.toInt(),
            mi.toInt(),
            ss.toInt(),
        )
        cal.set(java.util.Calendar.MILLISECOND, 0)
        val age = System.currentTimeMillis() - cal.timeInMillis
        age in 0..MAX_PEEK_AGE_MS
    }.getOrDefault(false)
}

/**
 * Multi-window (split screen / floating window) detector.
 *
 * Signal (verified 2026-10-08 with YouTube+Chrome in split): MIUI's own
 * GameBoosterService logs the dual-pane state in its onGameStatusChange line:
 *
 *   mForegroundPackageName='<focused>' ... mMultiWindowForegroundPackageName='<other>'
 *
 * `mMultiWindowForegroundPackageName` is 'null' in full-screen and holds the
 * other pane's package while two windows are visible. Root `logcat -b main`
 * streaming is the reliable channel (dumpsys/binder stays unusable from the
 * app's su context). Plain `logcat` replays the existing buffer first, so
 * the last parsed line seeds the initial state — no missed split at start.
 */
class MultiWindowWatcher(private val bridge: RootBridge) {

    companion object {
        private const val TAG = "MiFineTune"
        /** Extracted name is 'null' (literal) when no second window exists. */
        private val MW_RE =
            Regex("mMultiWindowForegroundPackageName='([^']+)'")
    }

    data class State(val active: Boolean = false, val otherPkg: String? = null)

    private var process: Process? = null
    private var readerJob: Job? = null

    val isAlive: Boolean get() = process?.isAlive == true

    fun start(scope: CoroutineScope, onState: (State) -> Unit) {
        stop()
        val p = runCatching {
            bridge.stream("logcat -b main -s GameBoosterService:V")
        }.getOrNull() ?: run {
            Log.w(TAG, "multi-window watcher: failed to spawn logcat")
            return
        }
        process = p
        readerJob = scope.launch(Dispatchers.IO) {
            try {
                p.inputStream.bufferedReader().useLines { lines ->
                    // NOTE: the buffer replay is the seed — the LAST line's
                    // state is the current one regardless of age (split mode
                    // persists until exited), so stale lines are NOT skipped
                    // here. The service dedupes identical states.
                    for (line in lines) {
                        if (!isActive) break
                        val name = MW_RE.find(line)?.groupValues?.get(1) ?: continue
                        val st = if (name == "null") State(false, null)
                        else State(true, name)
                        onState(st)
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "multi-window watcher stream ended: $e")
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
}
