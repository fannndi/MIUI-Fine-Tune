package com.mifinetune.automation

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject

/**
 * Persistent automation settings (SharedPreferences) exposed as StateFlows
 * where the UI needs to react.
 *
 * Responsibility: typed config storage.
 * Non-goals: service lifecycle, arbiter logic.
 */
class AutomationConfig private constructor(context: Context) {

    companion object {
        private const val PREFS = "automation"
        private const val K_ENABLED = "enabled"
        private const val K_BASE = "base_profile"
        private const val K_APP_MAP = "app_map"
        private const val K_SYNC_PERF = "sync_miui_perf"
        private const val K_GMODE_CHECKER = "game_mode_checker"
        private const val K_SYNC_SAVER = "sync_miui_saver"
        private const val K_HOLD_PERF = "bridge_hold_perf"
        private const val K_SAVED_PERF = "bridge_saved_perf"
        private const val K_HOLD_SAVER = "bridge_hold_saver"
        private const val K_SAVED_SAVER = "bridge_saved_saver"

        /** Fresh installs start with Balance as the universal base. */
        const val DEFAULT_BASE = "balance"

        @Volatile
        private var instance: AutomationConfig? = null

        fun get(context: Context): AutomationConfig =
            instance ?: synchronized(this) {
                instance ?: AutomationConfig(context.applicationContext).also { instance = it }
            }
    }

    private val sp = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private val _enabled = MutableStateFlow(sp.getBoolean(K_ENABLED, true))
    val enabledFlow: StateFlow<Boolean> = _enabled
    var enabled: Boolean
        get() = _enabled.value
        set(v) {
            sp.edit().putBoolean(K_ENABLED, v).apply()
            _enabled.value = v
        }

    // --- MIUI bridge switches (v0.5) -------------------------------------

    private val _syncMiuiPerf = MutableStateFlow(sp.getBoolean(K_SYNC_PERF, true))
    /** Game in front → MIUI's own Performance switch follows our profile. */
    val syncMiuiPerfFlow: StateFlow<Boolean> = _syncMiuiPerf
    var syncMiuiPerf: Boolean
        get() = _syncMiuiPerf.value
        set(v) {
            sp.edit().putBoolean(K_SYNC_PERF, v).apply()
            _syncMiuiPerf.value = v
        }

    private val _gameModeChecker = MutableStateFlow(sp.getBoolean(K_GMODE_CHECKER, true))
    /** Warn when MIUI Game Booster still boosts a game mapped to us. */
    val gameModeCheckerFlow: StateFlow<Boolean> = _gameModeChecker
    var gameModeChecker: Boolean
        get() = _gameModeChecker.value
        set(v) {
            sp.edit().putBoolean(K_GMODE_CHECKER, v).apply()
            _gameModeChecker.value = v
        }

    private val _syncSaver = MutableStateFlow(sp.getBoolean(K_SYNC_SAVER, true))
    /** Frugal-mapped app in front → MIUI battery saver follows our profile. */
    val syncSaverFlow: StateFlow<Boolean> = _syncSaver
    var syncSaver: Boolean
        get() = _syncSaver.value
        set(v) {
            sp.edit().putBoolean(K_SYNC_SAVER, v).apply()
            _syncSaver.value = v
        }

    // --- bridge hold persistence (crash-safe restore points) --------------

    /** A bridge hold survives service death: on restart the restore point is
     *  still valid, so release writes the user's own value back. */
    var bridgeHoldPerf: Boolean
        get() = sp.getBoolean(K_HOLD_PERF, false)
        set(v) = sp.edit().putBoolean(K_HOLD_PERF, v).apply()

    var bridgeSavedPerf: String
        get() = sp.getString(K_SAVED_PERF, null) ?: "middle"
        set(v) = sp.edit().putString(K_SAVED_PERF, v).apply()

    var bridgeHoldSaver: Boolean
        get() = sp.getBoolean(K_HOLD_SAVER, false)
        set(v) = sp.edit().putBoolean(K_HOLD_SAVER, v).apply()

    var bridgeSavedSaver: Boolean
        get() = sp.getBoolean(K_SAVED_SAVER, false)
        set(v) = sp.edit().putBoolean(K_SAVED_SAVER, v).apply()

    /** The universal base: the last manually selected profile. */
    var baseProfile: String
        get() = sp.getString(K_BASE, DEFAULT_BASE) ?: DEFAULT_BASE
        set(v) {
            sp.edit().putString(K_BASE, v).apply()
        }

    // --- per-app mapping -------------------------------------------------

    private val _appMap = MutableStateFlow(readMap())
    val appMapFlow: StateFlow<Map<String, String>> = _appMap

    fun appMap(): Map<String, String> = _appMap.value

    /** null profileId removes the mapping (app falls back to the base). */
    fun setAppProfile(pkg: String, profileId: String?) {
        val next = _appMap.value.toMutableMap()
        if (profileId == null) next.remove(pkg) else next[pkg] = profileId
        sp.edit().putString(K_APP_MAP, JSONObject(next as Map<*, *>).toString()).apply()
        _appMap.value = next
    }

    private fun readMap(): Map<String, String> {
        val raw = sp.getString(K_APP_MAP, null) ?: return emptyMap()
        return runCatching {
            val obj = JSONObject(raw)
            obj.keys().asSequence().associateWith { obj.getString(it) }
        }.getOrDefault(emptyMap())
    }
}
