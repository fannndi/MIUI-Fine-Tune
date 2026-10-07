package com.mifinetune.miui

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.provider.Settings
import android.util.Log
import androidx.core.content.ContextCompat
import com.mifinetune.core.RootBridge
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * Live view of the MIUI power-layer state, harvested from sources that are
 * reliable on this ROM (broadcasts get dropped, writes are not):
 *
 *  - `power_mode`      : MIUI's own Balanced/Performance switch
 *    (`Settings.System`, "middle" = balanced, "high" = performance — the
 *    hidden `PowerModeSettings` sheet writes exactly this key; root put
 *    verified 2026-10-07, the MIUI sheet reflects the change immediately).
 *  - `low_power`       : Android battery saver (`Settings.Global`) — MIUI's
 *    battery saver toggle drives this as well.
 *  - `ultraSaver`      : MIUI "Ultra battery saver" — a framework-only mode
 *    (`miui.intent.action.EXTREME_POWER_SAVE_MODE_CHANGED`). Broadcast-only:
 *    state is not persisted anywhere queryable from an app (PowerKeeper's
 *    provider surface has no extreme-mode authority on this build), so the
 *    FGS-attached receiver is the channel. When it is on, MiFineTune retires
 *    entirely: the framework owns the device and any baseline of ours would
 *    only fight it.
 *  - `gameModeActive`  : an empirical signature used by the game-mode
 *    checker: MIUI Game Booster holding a thermal scenario for the focused
 *    package (sconfig != 0, root cat — kernel file reads work from the su
 *    context, binder services do not).
 *
 * Responsibility: authoritative state reads + the MIUI perf-mode write.
 * Non-goals: decisions (ModeArbiter), engine IO (Tuner).
 */
class MiStateBridge(private val context: Context, private val root: RootBridge) {

    data class Snapshot(
        val powerMode: PowerMode,
        val saverOn: Boolean,
        val ultraSaver: Boolean,
        val gameModeActive: Boolean,
    )

    /** MIUI's own mode switch: middle = balanced, high = performance. */
    enum class PowerMode(val key: String) { Balanced("middle"), Performance("high");

        companion object {
            fun of(raw: String?): PowerMode = if (raw == Performance.key) Performance else Balanced
        }
    }

    /** Value of `power_mode` before our first perf-mode write (restore point). */
    private var savedPowerMode: PowerMode? = null
    @Volatile private var ultraSaver = false
    private val gameModeSignature = java.util.concurrent.atomic.AtomicBoolean(false)

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == ACTION_EXTREME) {
                ultraSaver = intent.getBooleanExtra(EXTRA_ENABLE, ultraSaver)
                Log.d(TAG, "MIUI extreme saver: $ultraSaver")
            }
        }
    }

    /** Registers the battery events; call from the service onCreate. */
    fun start() {
        val filter = IntentFilter().apply { addAction(ACTION_EXTREME) }
        ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
    }

    fun stop() {
        runCatching { context.unregisterReceiver(receiver) }
    }

    // --- reads ------------------------------------------------------------

    /** Root-free: system settings are world-readable from the app context. */
    fun readPowerMode(): PowerMode = runCatching {
        val raw = Settings.System.getString(context.contentResolver, KEY_POWER_MODE)
        PowerMode.of(raw)
    }.getOrDefault(PowerMode.Balanced)

    /** Root-free: battery saver state (Android + MIUI drive the same flag). */
    fun readSaver(): Boolean = runCatching {
        Settings.Global.getInt(context.contentResolver, "low_power", 0) == 1
    }.getOrDefault(false)

    /** Registered receiver state: MIUI Ultra battery saver (framework owns all). */
    fun ultra(): Boolean = ultraSaver

    /** Root read: MIUI Game Booster signature (thermal scenario held). */
    fun readGameModeSignature(): Boolean {
        val held = runCatching {
            val sconfig = root.sh("cat /sys/class/thermal/thermal_message/sconfig").out.trim()
            (sconfig.toIntOrNull() ?: 0) != 0
        }.getOrDefault(false)
        gameModeSignature.set(held)
        return held
    }

    fun lastGameModeSignature(): Boolean = gameModeSignature.get()

    /**
     * Root write path — flipping MIUI's own switch. The UI sheet reflects
     * the change immediately (verified 2026-10-07). No restore point is
     * taken twice: the caller owns the snapshot/restore cycle.
     */
    fun writePowerMode(mode: PowerMode) {
        runCatching {
            if (savedPowerMode == null) savedPowerMode = readPowerMode()
            root.sh("settings put system $KEY_POWER_MODE ${mode.key}")
            Log.d(TAG, "power_mode -> ${mode.key} (saved=${savedPowerMode?.key})")
        }.onFailure { Log.w(TAG, "power_mode write failed: $it") }
    }

    /** Restores the pre-bridge perf-mode when automation/game coupling ends. */
    fun restorePowerMode() {
        val saved = savedPowerMode ?: return
        writePowerMode(saved)
        savedPowerMode = null
    }

    fun snapshot(): Snapshot = Snapshot(
        powerMode = readPowerMode(),
        saverOn = readSaver(),
        ultraSaver = ultraSaver,
        gameModeActive = gameModeSignature.get(),
    )

    companion object {
        private const val TAG = "MiFineTune"

        // MIUI broadcast name (verified on device — the FGS receiver is the
        // reliable channel because the state is not persisted anywhere an
        // app can read).
        const val ACTION_EXTREME = "miui.intent.action.EXTREME_POWER_SAVE_MODE_CHANGED"
        const val EXTRA_ENABLE = "enabled"
        const val KEY_POWER_MODE = "power_mode"
        const val SAVER_KEY = "low_power"

        fun startIn(context: Context, root: RootBridge): MiStateBridge =
            MiStateBridge(context, root).apply { start() }
    }
}
