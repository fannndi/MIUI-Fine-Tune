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
