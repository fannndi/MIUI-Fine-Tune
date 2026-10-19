package com.mifinetune.dynamic

import android.content.Context
import android.content.SharedPreferences
import com.mifinetune.core.RootBridge
import com.mifinetune.core.Tuner
import org.json.JSONObject

/**
 * One-time migration from the v0.5 SharedPreferences into `config.json`.
 * Kept separate so [DynamicProfileConfig] stays focused on live config.
 */
object ConfigMigration {

    private const val PREFS = "dynamic_profile"
    private const val LEGACY_PREFS = "automation"
    private const val K_MIGRATED = "prefs_migrated_from_automation"

    /** Returns the migrated JSON (caller persists + applies it). */
    fun migrate(context: Context): JSONObject {
        val sp = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (!sp.getBoolean(K_MIGRATED, false)) {
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
            .put("base_profile", sp.getString("base_profile", DynamicProfileConfig.DEFAULT_BASE) ?: DynamicProfileConfig.DEFAULT_BASE)
            .put("sync_miui_perf", sp.getBoolean("sync_miui_perf", true))
            .put("sync_miui_saver", sp.getBoolean("sync_miui_saver", true))
            .put("game_mode_checker", sp.getBoolean("game_mode_checker", true))
            .put("app_map", runCatching { JSONObject(sp.getString("app_map", "{}") ?: "{}") }.getOrDefault(JSONObject()))
        migrateHolds(context, sp)
        return json
    }

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
            val tmp = java.io.File(context.cacheDir, "holds.json.migrate")
            tmp.writeText(holds.toString())
            Tuner.bridge.sh(
                "mkdir -p \"${RootBridge.STATE_DIR}\"; " +
                    "cp \"${tmp.absolutePath}\" \"${RootBridge.STATE_DIR}/holds.json\"; " +
                    "rm \"${tmp.absolutePath}\""
            )
        }
    }
}