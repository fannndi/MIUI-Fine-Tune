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
)

sealed interface Decision {
    /** Leave whatever is active in place. */
    data object None : Decision

    /** Apply [profileId]; [reason] is a UI/notification label. */
    data class Apply(val profileId: String, val reason: String) : Decision
}

object ModeArbiter {

    const val SLEEP_PROFILE = "sleep"

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
    )

    fun isTransient(pkg: String?): Boolean =
        pkg == null || pkg in TRANSIENT_PACKAGES || pkg.contains("inputmethod", ignoreCase = true)

    fun decide(input: ArbiterInput): Decision {
        if (!input.automationEnabled) return Decision.None

        if (!input.screenOn) {
            return Decision.Apply(input.sleepProfile, "screen off")
        }

        if (input.keyguardLocked) return Decision.None

        val pkg = input.foregroundPkg
        if (isTransient(pkg)) return Decision.None

        val mapped = input.appMap[pkg]
        return if (mapped != null) Decision.Apply(mapped, "app")
        else Decision.Apply(input.baseProfile, "base")
    }
}
