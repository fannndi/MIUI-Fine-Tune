package com.mifinetune.miui

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.provider.Settings
import android.util.Log
import androidx.core.content.ContextCompat
import com.mifinetune.core.RootBridge

/**
 * Live view of the MIUI power-layer state, harvested from sources that are
 * reliable on this ROM (verified 2026-10-08):
 *
 *  - Battery saver      : `Settings.Global low_power`. Root put flips the
 *    AOSP battery saver live (system_server observes the key) and MIUI's own
 *    Battery saver page follows the same flag.
 *  - Performance switch : the truth is the system property
 *    `persist.sys.aries.power_profile` ("middle"/"high") — SELinux rejects
 *    setprop from every root-su context we have, so the write goes through
 *    MIUI's own hidden dialog (`PowerModeSettings`): start it, tap the row,
 *    it writes the property + the `Settings.System power_mode` mirror and
 *    dismisses itself. The sheet and PowerKeeper read the property, so this
 *    is the only write that actually flips the visible switch.
 *  - Ultra saver        : framework-only mode, announced via
 *    `miui.intent.action.EXTREME_POWER_SAVE_MODE_CHANGED`. Broadcast-only:
 *    not persisted anywhere an app can read. When it is on, MiFineTune
 *    retires entirely (framework owns the device).
 *  - Game Booster       : empirical signature for the checker — a held
 *    thermal scenario (thermal_message/sconfig != 0).
 *
 * The hold/restore cycle (snapshot of the user's own mode before our first
 * write) is owned by the caller via [MiBridgeState]; this class only reads
 * state and performs one-step writes.
 */
class MiStateBridge(private val context: Context, private val root: RootBridge) {

    /** MIUI's own mode switch: middle = balanced, high = performance. */
    enum class PowerMode(val key: String, val uiName: String) {
        Balanced("middle", "Balanced"), Performance("high", "Performance");

        companion object {
            fun of(raw: String?): PowerMode = if (raw == Performance.key) Performance else Balanced
        }
    }

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

    /** Registers the extreme-saver event; call from the service onCreate. */
    fun start() {
        val filter = IntentFilter().apply { addAction(ACTION_EXTREME) }
        ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
    }

    fun stop() {
        runCatching { context.unregisterReceiver(receiver) }
    }

    // --- reads ------------------------------------------------------------

    /** Registered receiver state: MIUI Ultra battery saver (framework owns all). */
    fun ultra(): Boolean = ultraSaver

    /** Root-free: battery saver state (Android + MIUI drive the same flag). */
    fun readSaver(): Boolean = runCatching {
        Settings.Global.getInt(context.contentResolver, SAVER_KEY, 0) == 1
    }.getOrDefault(false)

    /**
     * Root-free read of the performance switch mirror. (The truth lives in
     * the `persist.sys.aries.power_profile` property, unreachable from our
     * su contexts — the mirror key is what the bridge writes and reads.)
     */
    fun readPowerMode(): PowerMode = runCatching {
        PowerMode.of(Settings.System.getString(context.contentResolver, KEY_POWER_MODE))
    }.getOrDefault(PowerMode.Balanced)

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

    // --- writes -----------------------------------------------------------

    /** Root write: battery saver. Live effect (verified 2026-10-08). */
    fun writeSaver(on: Boolean) {
        runCatching {
            root.sh("settings put global $SAVER_KEY ${if (on) 1 else 0}")
            Log.d(TAG, "low_power -> $on")
        }.onFailure { Log.w(TAG, "low_power write failed: $it") }
    }

    /**
     * Performance switch write. The switch's truth lives in the system
     * property `persist.sys.aries.power_profile`, which SELinux keeps out of
     * reach for every root-su context on this setup (verified 2026-10-08:
     * shell, run-as and the app's own su are all silently dropped), and
     * MIUI's hidden dialog cannot open over a locked game. So the write is
     * the `Settings.System power_mode` mirror only: silent, harmless, and
     * consistent with what MIUI's own toggles maintain — the hidden sheet
     * (which reads the property) may not reflect it on this ROM.
     */
    fun writePowerMode(mode: PowerMode) {
        runCatching {
            root.sh("settings put system $KEY_POWER_MODE ${mode.key}")
            Log.d(TAG, "power_mode mirror -> ${mode.key}")
        }.onFailure { Log.w(TAG, "power_mode write failed: $it") }
    }

    private val powerManager get() = context.getSystemService(Context.POWER_SERVICE) as android.os.PowerManager

    companion object {
        private const val TAG = "MiFineTune"

        const val ACTION_EXTREME = "miui.intent.action.EXTREME_POWER_SAVE_MODE_CHANGED"
        const val EXTRA_ENABLE = "enabled"
        const val KEY_POWER_MODE = "power_mode"
        const val SAVER_KEY = "low_power"
        const val PROP_POWER_PROFILE = "persist.sys.aries.power_profile"

        fun startIn(context: Context, root: RootBridge): MiStateBridge =
            MiStateBridge(context, root).apply { start() }
    }
}
