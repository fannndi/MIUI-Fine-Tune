package com.mifinetune.automation

/**
 * Pure decision logic: given the device context, which profile should be
 * active right now?
 *
 * Model: the last manually chosen profile is the universal BASE; apps with a
 * per-app mapping override it while they are in front; screen-off always
 * applies the sleep profile.
 */
data class ArbiterInput(
    val automationEnabled: Boolean,
    val screenOn: Boolean,
    val keyguardLocked: Boolean,
    val foregroundPkg: String?,
    val appMap: Map<String, String>,
    val baseProfile: String,
    val sleepProfile: String,
    /** MIUI battery saver / Android battery saver — forces the frugal base. */
    val saverOn: Boolean = false,
    /** MIUI Ultra battery saver — framework owns the device; we retire. */
    val ultraSaver: Boolean = false,
)

sealed interface Decision {
    /** Leave whatever is active in place. */
    data object None : Decision

    /** Apply [profileId]; [reason] is a UI/notification label. */
    data class Apply(val profileId: String, val reason: String) : Decision

    /** Meaty retirement: the framework owns tuning (Ultra battery saver). */
    data object Retire : Decision
}

object ModeArbiter {

    const val SLEEP_PROFILE = "sleep"

    /** MIUI/Android battery saver forces the frugal base. */
    const val SAVER_PROFILE = "powersave"

    /**
     * Packages that must not trigger a switch: system chrome and dialogs
     * appear "in front" of the real foreground app. IMEs are matched by
     * substring. The launcher is NOT transient — it is the signal that the
     * user left an app (-> back to base).
     */
    val TRANSIENT_PACKAGES = setOf(
        "com.android.systemui",
        "com.mifinetune",
        "com.android.permissioncontroller",
        "com.google.android.permissioncontroller",
        "com.lbe.security.miui", // MIUI permission dialogs
        "android",
        // MIUI-common IMEs without "inputmethod" in the package name
        "com.baidu.input_mi",
        // the bridge's own performance-follow surface (hidden sheet) — its
        // resume event must not read as "user is in Settings"
        "com.android.settings",
    )

    fun isTransient(pkg: String?): Boolean =
        pkg == null || pkg in TRANSIENT_PACKAGES || pkg.contains("inputmethod", ignoreCase = true)

    fun decide(input: ArbiterInput): Decision {
        if (!input.automationEnabled) return Decision.None

        // MIUI Ultra battery saver owns the whole device (its own CPU/GPU/
        // network regime, whitelisted apps only). Neither our baselines nor
        // mapped overrides belong here — retire with a full restore.
        if (input.ultraSaver) return Decision.Retire

        if (!input.screenOn) {
            return Decision.Apply(input.sleepProfile, "screen off")
        }

        if (input.keyguardLocked) return Decision.None

        val pkg = input.foregroundPkg
        if (isTransient(pkg)) return Decision.None

        val mapped = input.appMap[pkg]
        // MIUI battery saver: the unmapped universe is forced to the frugal
        // base — mapped apps still win (the user may game under saver).
        val effectiveBase = if (input.saverOn) SAVER_PROFILE else input.baseProfile
        return if (mapped != null) Decision.Apply(mapped, "app")
        else Decision.Apply(effectiveBase, "base")
    }
}
