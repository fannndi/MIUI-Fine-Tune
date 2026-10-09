package com.mifinetune.dynamic

import org.json.JSONArray
import org.json.JSONObject

/**
 * Diagnostics data shapes for the daemon's read-only telemetry.
 *
 * Parsing lives here (not in the service) so the JSON contract is greppable
 * in one file; the daemon is the source of truth for every value.
 */

/** One environment sample from the daemon's `env` event. */
data class EnvSnapshot(
    val batteryPct: Int?,
    val charging: Boolean?,
    val batteryTempC: Float?,
    val cpuTempC: Float?,
    val gpuTempC: Float?,
    val gpuBusyPct: Int?,
    /** Panel frame rate from the read-only DRM `measured_fps` node. */
    val screenFps: Float? = null,
    /** Battery current µA: negative = charging, positive = discharging. */
    val batteryCurrentUa: Long? = null,
    val batteryVoltageUv: Long? = null,
    val littleFreqMhz: Int? = null,
    val bigFreqMhz: Int? = null,
    val gpuFreqMhz: Int? = null,
    /** F2FS userdata lifetime write counter (KB, read-only node). */
    val storageWrittenKb: Long? = null,
    /** Battery full-charge capacity in mAh (`charge_full`), health readout. */
    val chargeFullMah: Int? = null,
    /** Battery cycle count (`cycle_count`), when the fuel gauge exposes it. */
    val cycleCount: Int? = null,
)

/** Auto-revive (watchdog) totals from the last `stats` reply. */
data class HealsInfo(
    val total: Long = 0,
    val lastT: Long = 0,
    val lastKeys: Int = 0,
)

/** One transition from the daemon's `stats` reply. */
data class StatEntry(
    val t: Long,
    val from: String?,
    val to: String,
    val reason: String,
    val batteryPct: Int?,
    val tempC: Float?,
)

/** Daemon health snapshot from the `diag` reply. */
data class DiagInfo(
    val pid: Int,
    val uptimeS: Long,
    val active: String?,
    val reason: String?,
    val foreground: String?,
    val screenOn: Boolean,
    val locked: Boolean,
    val multiWindow: Boolean,
    val watcherFg: Boolean,
    val watcherMw: Boolean,
    val perfHeld: Boolean,
    val saverHeld: Boolean,
    val chargeHeld: Boolean,
    val bypassHeld: Boolean,
    val dndMode: String?,
    val statsLen: Int,
    val configEnabled: Boolean,
    val configDynamic: Boolean,
    val baseProfile: String,
)

object DiagnosticsParse {

    fun env(ev: JSONObject): EnvSnapshot {
        val e = ev.optJSONObject("env") ?: JSONObject()
        return EnvSnapshot(
            batteryPct = e.intOrNull("battery_pct"),
            charging = e.boolOrNull("charging"),
            batteryTempC = e.floatOrNull("battery_temp_c"),
            cpuTempC = e.floatOrNull("cpu_temp_c"),
            gpuTempC = e.floatOrNull("gpu_temp_c"),
            gpuBusyPct = e.intOrNull("gpu_busy_pct"),
            screenFps = e.floatOrNull("screen_fps"),
            batteryCurrentUa = e.longOrNull("battery_current_ua"),
            batteryVoltageUv = e.longOrNull("battery_voltage_uv"),
            littleFreqMhz = e.intOrNull("little_freq_mhz"),
            bigFreqMhz = e.intOrNull("big_freq_mhz"),
            gpuFreqMhz = e.intOrNull("gpu_freq_mhz"),
            storageWrittenKb = e.longOrNull("storage_written_kb"),
            chargeFullMah = e.intOrNull("charge_full_mah"),
            cycleCount = e.intOrNull("cycle_count"),
        )
    }

    /** Auto-revive totals carried by the `stats` reply. */
    fun heals(ev: JSONObject): HealsInfo = HealsInfo(
        total = ev.optLong("heals_total"),
        lastT = ev.optLong("heals_last_t"),
        lastKeys = ev.optInt("heals_last_keys"),
    )

    fun stats(ev: JSONObject): List<StatEntry> {
        val arr: JSONArray = ev.optJSONArray("entries") ?: return emptyList()
        return (0 until arr.length()).mapNotNull { i ->
            val o = arr.optJSONObject(i) ?: return@mapNotNull null
            StatEntry(
                t = o.optLong("t"),
                from = o.strOrNull("from"),
                to = o.optString("to", "?"),
                reason = o.optString("reason", "?"),
                batteryPct = o.intOrNull("battery"),
                tempC = o.floatOrNull("temp_c"),
            )
        }
    }

    fun diag(ev: JSONObject): DiagInfo? {
        val d = ev.optJSONObject("diag") ?: return null
        val cfg = d.optJSONObject("config") ?: JSONObject()
        val w = d.optJSONObject("watchers") ?: JSONObject()
        val h = d.optJSONObject("holds") ?: JSONObject()
        return DiagInfo(
            pid = d.optInt("pid"),
            uptimeS = d.optLong("uptime_s"),
            active = d.strOrNull("active"),
            reason = d.strOrNull("reason"),
            foreground = d.strOrNull("foreground"),
            screenOn = d.optBoolean("screen_on"),
            locked = d.optBoolean("locked"),
            multiWindow = d.optBoolean("multi_window"),
            watcherFg = w.optBoolean("fg"),
            watcherMw = w.optBoolean("mw"),
            perfHeld = h.optBoolean("perf_held"),
            saverHeld = h.optBoolean("saver_held"),
            chargeHeld = h.optBoolean("charge_held"),
            bypassHeld = h.optBoolean("bypass_held"),
            dndMode = h.optString("dnd_mode").ifEmpty { null },
            statsLen = d.optInt("stats_len"),
            configEnabled = cfg.optBoolean("enabled"),
            configDynamic = cfg.optBoolean("dynamic"),
            baseProfile = cfg.optString("base_profile", "?"),
        )
    }
}

// --- JSONObject null-safe helpers -------------------------------------------

internal fun JSONObject.strOrNull(key: String): String? =
    if (has(key) && !isNull(key)) optString(key).ifEmpty { null } else null

internal fun JSONObject.intOrNull(key: String): Int? =
    if (has(key) && !isNull(key)) optInt(key) else null

internal fun JSONObject.floatOrNull(key: String): Float? =
    if (has(key) && !isNull(key)) optDouble(key).toFloat() else null

internal fun JSONObject.longOrNull(key: String): Long? =
    if (has(key) && !isNull(key)) optLong(key) else null

internal fun JSONObject.boolOrNull(key: String): Boolean? =
    if (has(key) && !isNull(key)) optBoolean(key) else null
