package com.mifinetune.dynamic

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import com.mifinetune.core.RootBridge
import com.mifinetune.core.Tuner
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject
import java.io.File

/**
 * App-owned user config (`filesDir/config.json`), exposed as StateFlows.
 *
 * Ownership: the app is the SINGLE writer; the Rust daemon reads the file
 * (start, `config_changed` hint, mtime fallback). Writes are atomic
 * (tmp + rename). One-time migration reads the old SharedPreferences.
 *
 * Responsibility: typed config storage + migration + daemon hints.
 * Non-goals: decisions (daemon), UI.
 */
class DynamicProfileConfig private constructor(context: Context) {

    companion object {
        private const val TAG = "MiFineTune"
        const val FILE = "config.json"
        const val DEFAULT_BASE = "balance"

        // legacy SharedPreferences (v0.5); kept as a downgrade fallback
        private const val PREFS = "dynamic_profile"
        private const val LEGACY_PREFS = "automation"
        private const val K_MIGRATED = "prefs_migrated_from_automation"

        /** Profile ids the UI/engine ship today (import validation). */
        private val KNOWN_PROFILES = setOf("powersave", "balance", "game")

        /** Keys a backup may carry (anything else is ignored). */
        private val IMPORT_KEYS = setOf(
            "enabled", "dynamic", "base_profile", "app_map",
            "sync_miui_perf", "sync_miui_saver", "game_mode_checker", "sync_refresh",
            "guard_battery", "battery_floor_pct", "guard_thermal", "thermal_ceiling_c",
        )

        @Volatile
        private var instance: DynamicProfileConfig? = null

        fun get(context: Context): DynamicProfileConfig =
            instance ?: synchronized(this) {
                instance ?: DynamicProfileConfig(context.applicationContext).also { instance = it }
            }

        fun path(context: Context): String =
            File(context.filesDir, FILE).absolutePath
    }

    private val file = File(context.filesDir, FILE)
    private val lock = Any()

    private val _enabled = MutableStateFlow(true)
    val enabledFlow: StateFlow<Boolean> = _enabled
    var enabled: Boolean
        get() = _enabled.value
        set(v) = put { it.put("enabled", v) }

    private val _dynamicEnabled = MutableStateFlow(true)
    val dynamicEnabledFlow: StateFlow<Boolean> = _dynamicEnabled
    var dynamicEnabled: Boolean
        get() = _dynamicEnabled.value
        set(v) = put { it.put("dynamic", v) }

    private val _syncMiuiPerf = MutableStateFlow(true)
    val syncMiuiPerfFlow: StateFlow<Boolean> = _syncMiuiPerf
    var syncMiuiPerf: Boolean
        get() = _syncMiuiPerf.value
        set(v) = put { it.put("sync_miui_perf", v) }

    private val _syncSaver = MutableStateFlow(true)
    val syncSaverFlow: StateFlow<Boolean> = _syncSaver
    var syncSaver: Boolean
        get() = _syncSaver.value
        set(v) = put { it.put("sync_miui_saver", v) }

    private val _gameModeChecker = MutableStateFlow(true)
    val gameModeCheckerFlow: StateFlow<Boolean> = _gameModeChecker
    var gameModeChecker: Boolean
        get() = _gameModeChecker.value
        set(v) = put { it.put("game_mode_checker", v) }

    // --- adaptive guards (v0.7) ---------------------------------------------

    private val _syncRefresh = MutableStateFlow(true)
    val syncRefreshFlow: StateFlow<Boolean> = _syncRefresh
    var syncRefresh: Boolean
        get() = _syncRefresh.value
        set(v) = put { it.put("sync_refresh", v) }

    // --- adaptive guards (v0.7) ---------------------------------------------

    private val _guardBattery = MutableStateFlow(true)
    val guardBatteryFlow: StateFlow<Boolean> = _guardBattery
    var guardBattery: Boolean
        get() = _guardBattery.value
        set(v) = put { it.put("guard_battery", v) }

    private val _batteryFloor = MutableStateFlow(20)
    val batteryFloorFlow: StateFlow<Int> = _batteryFloor
    var batteryFloor: Int
        get() = _batteryFloor.value
        set(v) = put { it.put("battery_floor_pct", v.coerceIn(5, 50)) }

    private val _guardThermal = MutableStateFlow(true)
    val guardThermalFlow: StateFlow<Boolean> = _guardThermal
    var guardThermal: Boolean
        get() = _guardThermal.value
        set(v) = put { it.put("guard_thermal", v) }

    private val _thermalCeiling = MutableStateFlow(75f)
    val thermalCeilingFlow: StateFlow<Float> = _thermalCeiling
    var thermalCeiling: Float
        get() = _thermalCeiling.value
        set(v) = put { it.put("thermal_ceiling_c", v.coerceIn(60f, 90f).toDouble()) }

    private val _appMap = MutableStateFlow<Map<String, String>>(emptyMap())
    val appMapFlow: StateFlow<Map<String, String>> = _appMap

    private var _baseProfile: String = DEFAULT_BASE

    /** The universal base: the last manually selected profile. */
    var baseProfile: String
        get() = _baseProfile
        set(v) = put { it.put("base_profile", v) }

    init {
        val json = load() ?: migrateFromPrefs(context)
        apply(json)
    }

    fun appMap(): Map<String, String> = _appMap.value

    /** null profileId removes the mapping (app falls back to the base). */
    fun setAppProfile(pkg: String, profileId: String?) = put { json ->
        val map = JSONObject()
        val next = _appMap.value.toMutableMap()
        if (profileId == null) next.remove(pkg) else next[pkg] = profileId
        for ((k, v) in next) map.put(k, v)
        json.put("app_map", map)
    }

    // --- backup (export / import) -------------------------------------------

    /** Pretty JSON of the current config file, for backup (defaults filled). */
    fun exportJson(): String = synchronized(lock) {
        val json = load() ?: baseJson()
        fillDefaults(json)
        json.toString(2)
    }

    /**
     * Applies a backup: only recognized keys, values validated/clamped; the
     * write stays atomic and the daemon gets its hint. Returns false when the
     * payload carries nothing usable (unknown JSON / unrelated object).
     */
    fun importJson(raw: String): Boolean {
        val src = runCatching { JSONObject(raw) }.getOrNull() ?: return false
        if (src.keys().asSequence().none { it in IMPORT_KEYS }) return false
        synchronized(lock) {
            val next = load() ?: baseJson()
            if (src.has("enabled")) next.put("enabled", src.optBoolean("enabled"))
            if (src.has("dynamic")) next.put("dynamic", src.optBoolean("dynamic"))
            if (src.has("sync_miui_perf")) next.put("sync_miui_perf", src.optBoolean("sync_miui_perf"))
            if (src.has("sync_miui_saver")) next.put("sync_miui_saver", src.optBoolean("sync_miui_saver"))
            if (src.has("game_mode_checker")) next.put("game_mode_checker", src.optBoolean("game_mode_checker"))
            if (src.has("sync_refresh")) next.put("sync_refresh", src.optBoolean("sync_refresh"))
            if (src.has("guard_battery")) next.put("guard_battery", src.optBoolean("guard_battery"))
            if (src.has("guard_thermal")) next.put("guard_thermal", src.optBoolean("guard_thermal"))
            if (src.has("battery_floor_pct")) {
                next.put("battery_floor_pct", src.optInt("battery_floor_pct", 20).coerceIn(5, 50))
            }
            if (src.has("thermal_ceiling_c")) {
                next.put(
                    "thermal_ceiling_c",
                    src.optDouble("thermal_ceiling_c", 75.0).coerceIn(60.0, 90.0),
                )
            }
            src.optString("base_profile").takeIf { it in KNOWN_PROFILES }
                ?.let { next.put("base_profile", it) }
            src.optJSONObject("app_map")?.let { map ->
                val out = JSONObject()
                map.keys().forEach { k ->
                    map.optString(k).takeIf { it in KNOWN_PROFILES }?.let { out.put(k, it) }
                }
                next.put("app_map", out)
            }
            fillDefaults(next)
            writeAtomic(next.toString())
            apply(next)
        }
        DaemonLink.configChanged()
        return true
    }

    // --- internals ---------------------------------------------------------

    private fun load(): JSONObject? = runCatching {
        if (!file.exists()) return null
        JSONObject(file.readText())
    }.getOrNull()

    private fun apply(json: JSONObject) {
        _enabled.value = json.optBoolean("enabled", true)
        _dynamicEnabled.value = json.optBoolean("dynamic", true)
        _syncMiuiPerf.value = json.optBoolean("sync_miui_perf", true)
        _syncSaver.value = json.optBoolean("sync_miui_saver", true)
        _gameModeChecker.value = json.optBoolean("game_mode_checker", true)
        _syncRefresh.value = json.optBoolean("sync_refresh", true)
        _guardBattery.value = json.optBoolean("guard_battery", true)
        _batteryFloor.value = json.optInt("battery_floor_pct", 20).coerceIn(5, 50)
        _guardThermal.value = json.optBoolean("guard_thermal", true)
        _thermalCeiling.value = json.optDouble("thermal_ceiling_c", 75.0).toFloat().coerceIn(60f, 90f)
        _baseProfile = json.optString("base_profile", DEFAULT_BASE).ifEmpty { DEFAULT_BASE }
        val map = mutableMapOf<String, String>()
        json.optJSONObject("app_map")?.let { obj ->
            obj.keys().forEach { k -> map[k] = obj.optString(k) }
        }
        _appMap.value = map
    }

    /** Mutates one key, persists atomically, refreshes flows, hints daemon. */
    private fun put(mutate: (JSONObject) -> Unit) {
        synchronized(lock) {
            val json = load() ?: baseJson()
            mutate(json)
            json.put("schema", 1)
            fillDefaults(json)
            writeAtomic(json.toString())
            apply(json)
        }
        DaemonLink.configChanged()
    }

    /** Keeps the file explicit: missing keys would otherwise be daemon defaults. */
    private fun fillDefaults(json: JSONObject) {
        fun def(key: String, value: Any) {
            if (!json.has(key)) json.put(key, value)
        }
        def("enabled", true)
        def("dynamic", true)
        def("base_profile", DEFAULT_BASE)
        def("sync_miui_perf", true)
        def("sync_miui_saver", true)
        def("game_mode_checker", true)
        def("sync_refresh", true)
        def("guard_battery", true)
        def("battery_floor_pct", 20)
        def("guard_thermal", true)
        def("thermal_ceiling_c", 75.0)
        def("app_map", JSONObject())
    }

    private fun baseJson(): JSONObject = JSONObject()
        .put("schema", 1)
        .put("enabled", _enabled.value)
        .put("dynamic", _dynamicEnabled.value)
        .put("base_profile", _baseProfile)
        .put("sync_miui_perf", _syncMiuiPerf.value)
        .put("sync_miui_saver", _syncSaver.value)
        .put("game_mode_checker", _gameModeChecker.value)
        .put("sync_refresh", _syncRefresh.value)
        .put("guard_battery", _guardBattery.value)
        .put("battery_floor_pct", _batteryFloor.value)
        .put("guard_thermal", _guardThermal.value)
        .put("thermal_ceiling_c", _thermalCeiling.value.toDouble())
        .put("app_map", JSONObject())

    private fun writeAtomic(body: String) {
        runCatching {
            val tmp = File(file.parentFile, "$FILE.tmp")
            tmp.writeText(body)
            if (!tmp.renameTo(file)) {
                file.writeText(body)
                tmp.delete()
            }
        }.onFailure { Log.w(TAG, "config write failed: $it") }
    }

    /**
     * One-time migration from the v0.5 SharedPreferences. The legacy file
     * is left in place (downgrade-safe); bridge holds are copied to the
     * daemon's holds.json when one was active at upgrade time.
     */
    private fun migrateFromPrefs(context: Context): JSONObject {
        val sp = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (!sp.getBoolean(K_MIGRATED, false)) {
            // first: copy the pre-rebrand file (v0.5 migration chain)
            val legacy = context.getSharedPreferences(LEGACY_PREFS, Context.MODE_PRIVATE)
            sp.edit().apply {
                for ((k, v) in legacy.all) {
                    when (v) {
                        is Boolean -> putBoolean(k, v)
                        is String -> putString(k, v)
                        is Int -> putInt(k, v)
                        is Long -> putLong(k, v)
                        is Float -> putFloat(k, v)
                        is Set<*> -> @Suppress("UNCHECKED_CAST") putStringSet(k, v as Set<String>)
                    }
                }
                putBoolean(K_MIGRATED, true)
            }.apply()
        }
        val json = JSONObject()
            .put("schema", 1)
            .put("enabled", sp.getBoolean("enabled", true))
            .put("dynamic", sp.getBoolean("dynamic_enabled", true))
            .put("base_profile", sp.getString("base_profile", DEFAULT_BASE) ?: DEFAULT_BASE)
            .put("sync_miui_perf", sp.getBoolean("sync_miui_perf", true))
            .put("sync_miui_saver", sp.getBoolean("sync_miui_saver", true))
            .put("game_mode_checker", sp.getBoolean("game_mode_checker", true))
            .put("app_map", runCatching { JSONObject(sp.getString("app_map", "{}") ?: "{}") }.getOrDefault(JSONObject()))
        writeAtomic(json.toString())
        migrateHolds(context, sp)
        Log.d(TAG, "config migrated from prefs -> ${file.name}")
        return json
    }

    /** Copies an active bridge hold into the daemon's holds.json (rare path). */
    private fun migrateHolds(context: Context, sp: SharedPreferences) {
        val perfHeld = sp.getBoolean("bridge_hold_perf", false)
        val saverHeld = sp.getBoolean("bridge_hold_saver", false)
        if (!perfHeld && !saverHeld) return
        runCatching {
            val holds = JSONObject()
                .put("perf_held", perfHeld)
                .put("perf_saved", sp.getString("bridge_saved_perf", "middle"))
                .put("saver_held", saverHeld)
                .put("saver_saved", sp.getBoolean("bridge_saved_saver", false))
            val tmp = File(context.cacheDir, "holds.json.migrate")
            tmp.writeText(holds.toString())
            Tuner.bridge.sh(
                "mkdir -p ${RootBridge.STATE_DIR}; " +
                    "cp ${tmp.absolutePath} ${RootBridge.STATE_DIR}/holds.json; " +
                    "rm ${tmp.absolutePath}"
            )
        }.onFailure { Log.w(TAG, "holds migration skipped: $it") }
    }
}
