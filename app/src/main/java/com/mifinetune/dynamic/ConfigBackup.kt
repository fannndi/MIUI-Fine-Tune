package com.mifinetune.dynamic

import org.json.JSONObject

/**
 * Backup (export/import) helpers for [DynamicProfileConfig].
 *
 * Kept in a separate file so the config class stays focused on typed
 * StateFlows + persistence, while all backup validation/clamping lives here.
 */
object ConfigBackup {

    /** Pretty JSON of the current config file, for backup (defaults filled). */
    fun export(config: DynamicProfileConfig): String = synchronized(config.lock) {
        val json = config.load() ?: config.baseJson()
        config.fillDefaults(json)
        json.toString(2)
    }

    /**
     * Applies a backup: only recognized keys, values validated/clamped; the
     * write stays atomic and the daemon gets its hint. Returns false when the
     * payload carries nothing usable.
     */
    fun import(config: DynamicProfileConfig, raw: String): Boolean {
        val src = runCatching { JSONObject(raw) }.getOrNull() ?: return false
        val importKeys = setOf(
            "enabled", "dynamic", "base_profile", "app_map",
            "sync_miui_perf", "sync_miui_saver", "sync_refresh", "game_mode_checker",
            "guard_battery", "battery_floor_pct", "guard_thermal", "thermal_ceiling_c",
            "maintenance", "charge_limit", "charge_limit_pct", "jank_boost",
            "app_profiles", "bypass_floor_pct", "quiet_hours", "quiet_start_hour",
            "quiet_end_hour", "quiet_profile",
        )
        if (src.keys().asSequence().none { it in importKeys }) return false
        val knownProfiles = setOf("powersave", "balance", "game")
        synchronized(config.lock) {
            val next = config.load() ?: config.baseJson()
            if (src.has("enabled")) next.put("enabled", src.optBoolean("enabled"))
            if (src.has("dynamic")) next.put("dynamic", src.optBoolean("dynamic"))
            if (src.has("sync_miui_perf")) next.put("sync_miui_perf", src.optBoolean("sync_miui_perf"))
            if (src.has("sync_miui_saver")) next.put("sync_miui_saver", src.optBoolean("sync_miui_saver"))
            if (src.has("sync_refresh")) next.put("sync_refresh", src.optBoolean("sync_refresh"))
            if (src.has("game_mode_checker")) next.put("game_mode_checker", src.optBoolean("game_mode_checker"))
            if (src.has("guard_battery")) next.put("guard_battery", src.optBoolean("guard_battery"))
            if (src.has("guard_thermal")) next.put("guard_thermal", src.optBoolean("guard_thermal"))
            if (src.has("maintenance")) next.put("maintenance", src.optBoolean("maintenance"))
            if (src.has("jank_boost")) next.put("jank_boost", src.optBoolean("jank_boost"))
            if (src.has("charge_limit")) next.put("charge_limit", src.optBoolean("charge_limit"))
            if (src.has("charge_limit_pct")) {
                next.put("charge_limit_pct", src.optInt("charge_limit_pct", 80).coerceIn(60, 95))
            }
            if (src.has("quiet_hours")) next.put("quiet_hours", src.optBoolean("quiet_hours"))
            if (src.has("quiet_start_hour")) {
                next.put("quiet_start_hour", src.optInt("quiet_start_hour", 22).coerceIn(0, 23))
            }
            if (src.has("quiet_end_hour")) {
                next.put("quiet_end_hour", src.optInt("quiet_end_hour", 6).coerceIn(0, 23))
            }
            src.optString("quiet_profile").takeIf { it in knownProfiles }?.let {
                next.put("quiet_profile", it)
            }
            if (src.has("battery_floor_pct")) {
                next.put("battery_floor_pct", src.optInt("battery_floor_pct", 20).coerceIn(5, 50))
            }
            if (src.has("bypass_floor_pct")) {
                next.put("bypass_floor_pct", src.optInt("bypass_floor_pct", 30).coerceIn(15, 50))
            }
            src.optJSONObject("app_profiles")?.let { obj ->
                val out = JSONObject()
                obj.keys().forEach { k ->
                    val e = AppProfileEntry.fromJson(obj.optJSONObject(k) ?: return@forEach)
                    val clean = e.copy(profile = e.profile?.takeIf { it in knownProfiles })
                    if (!clean.isEmpty) out.put(k, clean.toJson())
                }
                next.put("app_profiles", out)
                val mirror = JSONObject()
                out.keys().forEach { k ->
                    out.optJSONObject(k)?.optString("profile")?.takeIf { it.isNotEmpty() }
                        ?.let { mirror.put(k, it) }
                }
                next.put("app_map", mirror)
            }
            if (src.has("thermal_ceiling_c")) {
                next.put(
                    "thermal_ceiling_c",
                    src.optDouble("thermal_ceiling_c", 75.0).coerceIn(60.0, 90.0),
                )
            }
            src.optString("base_profile").takeIf { it in knownProfiles }
                ?.let { next.put("base_profile", it) }
            if (!src.has("app_profiles")) {
                src.optJSONObject("app_map")?.let { map ->
                    val out = JSONObject()
                    map.keys().forEach { k ->
                        map.optString(k).takeIf { it in knownProfiles }?.let { out.put(k, it) }
                    }
                    next.put("app_map", out)
                }
            }
            config.fillDefaults(next)
            config.writeAtomic(next.toString())
            config.apply(next)
        }
        DaemonLink.configChanged()
        return true
    }
}