package com.mifinetune.dynamic

import org.json.JSONObject

/**
 * Apps Profile entry (software layer, per app): the Device-Profile mapping
 * plus the audited software surfaces. Every field is opt-in; an absent field
 * means "leave MIUI untouched".
 *
 * The daemon re-validates everything (invalid values are ignored, never
 * written); this model keeps the UI + file shape honest.
 */
data class AppProfileEntry(
    /** Device-Profile mapping (mirrored into the legacy `app_map`). */
    val profile: String? = null,
    /** Per-app bypass charging (`input_suspend`) while in front. */
    val bypassCharge: Boolean = false,
    /** Do Not Disturb while in front: `priority` | `total`. */
    val dnd: String? = null,
    /**
     * Per-app display refresh target in Hz: 30 | 60 | 90 | 120.
     * `null` = Default: MIUI/system keeps control for this app.
     */
    val refreshHz: Int? = null,
) {
    val isEmpty: Boolean
        get() = profile == null && !bypassCharge && dnd == null && refreshHz == null

    companion object {
        val DND_MODES = setOf("priority", "total")
        val REFRESH_HZ = setOf(30, 60, 90, 120)

        fun fromJson(o: JSONObject): AppProfileEntry = AppProfileEntry(
            profile = o.optString("profile").takeIf { it.isNotEmpty() },
            bypassCharge = o.optBoolean("bypass_charge", false),
            dnd = o.optString("dnd").takeIf { it in DND_MODES },
            refreshHz = o.optInt("refresh_hz", 0).takeIf { it in REFRESH_HZ },
        )
    }

    fun toJson(): JSONObject = JSONObject().apply {
        profile?.let { put("profile", it) }
        if (bypassCharge) put("bypass_charge", true)
        dnd?.let { put("dnd", it) }
        refreshHz?.let { put("refresh_hz", it) }
    }
}
