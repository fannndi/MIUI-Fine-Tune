package com.mifinetune.automation

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject

/**
 * Persistent automation settings (SharedPreferences) exposed as StateFlows so
 * both the UI and the service stay in sync without broadcasts.
 *
 * Responsibility: typed config storage.
 * Non-goals: service lifecycle, arbiter logic.
 */
class AutomationConfig private constructor(context: Context) {

    companion object {
        private const val PREFS = "automation"
        private const val K_ENABLED = "enabled"
        private const val K_DEFAULT = "default_profile"
        private const val K_SLEEP_ENABLED = "sleep_enabled"
        private const val K_SLEEP_PROFILE = "sleep_profile"
        private const val K_SKIP_MUSIC = "skip_on_music"
        private const val K_SKIP_CHARGING = "skip_on_charging"
        private const val K_BOOT_APPLY = "boot_apply"
        private const val K_SHOW_SYSTEM = "show_system_apps"
        private const val K_APP_MAP = "app_map"

        const val DEFAULT_PROFILE = "balance"
        const val DEFAULT_SLEEP_PROFILE = "sleep"

        @Volatile
        private var instance: AutomationConfig? = null

        fun get(context: Context): AutomationConfig =
            instance ?: synchronized(this) {
                instance ?: AutomationConfig(context.applicationContext).also { instance = it }
            }
    }

    private val sp = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun bool(key: String, def: Boolean) = MutableStateFlow(sp.getBoolean(key, def))
    private fun string(key: String, def: String) = MutableStateFlow(sp.getString(key, def) ?: def)

    private val _enabled = bool(K_ENABLED, false)
    val enabledFlow: StateFlow<Boolean> = _enabled
    var enabled: Boolean
        get() = _enabled.value
        set(v) = setBool(K_ENABLED, _enabled, v)

    private val _defaultProfile = string(K_DEFAULT, DEFAULT_PROFILE)
    val defaultProfileFlow: StateFlow<String> = _defaultProfile
    var defaultProfile: String
        get() = _defaultProfile.value
        set(v) = setString(K_DEFAULT, _defaultProfile, v)

    private val _sleepEnabled = bool(K_SLEEP_ENABLED, true)
    val sleepEnabledFlow: StateFlow<Boolean> = _sleepEnabled
    var sleepEnabled: Boolean
        get() = _sleepEnabled.value
        set(v) = setBool(K_SLEEP_ENABLED, _sleepEnabled, v)

    private val _sleepProfile = string(K_SLEEP_PROFILE, DEFAULT_SLEEP_PROFILE)
    val sleepProfileFlow: StateFlow<String> = _sleepProfile
    var sleepProfile: String
        get() = _sleepProfile.value
        set(v) = setString(K_SLEEP_PROFILE, _sleepProfile, v)

    private val _skipOnMusic = bool(K_SKIP_MUSIC, false)
    val skipOnMusicFlow: StateFlow<Boolean> = _skipOnMusic
    var skipOnMusic: Boolean
        get() = _skipOnMusic.value
        set(v) = setBool(K_SKIP_MUSIC, _skipOnMusic, v)

    private val _skipOnCharging = bool(K_SKIP_CHARGING, false)
    val skipOnChargingFlow: StateFlow<Boolean> = _skipOnCharging
    var skipOnCharging: Boolean
        get() = _skipOnCharging.value
        set(v) = setBool(K_SKIP_CHARGING, _skipOnCharging, v)

    private val _bootApply = bool(K_BOOT_APPLY, true)
    val bootApplyFlow: StateFlow<Boolean> = _bootApply
    var bootApply: Boolean
        get() = _bootApply.value
        set(v) = setBool(K_BOOT_APPLY, _bootApply, v)

    private val _showSystemApps = bool(K_SHOW_SYSTEM, false)
    val showSystemAppsFlow: StateFlow<Boolean> = _showSystemApps
    var showSystemApps: Boolean
        get() = _showSystemApps.value
        set(v) = setBool(K_SHOW_SYSTEM, _showSystemApps, v)

    // --- per-app mapping -------------------------------------------------

    private val _appMap = MutableStateFlow(readMap())
    val appMapFlow: StateFlow<Map<String, String>> = _appMap

    /** package -> profile id ("powersave" | "balance" | "game"). */
    fun appMap(): Map<String, String> = _appMap.value

    /** null profileId removes the mapping (app falls back to the default). */
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

    private fun setBool(key: String, flow: MutableStateFlow<Boolean>, v: Boolean) {
        sp.edit().putBoolean(key, v).apply()
        flow.value = v
    }

    private fun setString(key: String, flow: MutableStateFlow<String>, v: String) {
        sp.edit().putString(key, v).apply()
        flow.value = v
    }
}
