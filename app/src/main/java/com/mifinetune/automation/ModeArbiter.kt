package com.mifinetune.automation

/**
 * Pure decision logic: given the device context, which profile should be
 * active right now?
 *
 * Priority: screen off -> sleep  >  mapped app -> its profile  >  default.
 * Never decides *how* to apply — the service does that through the engine.
 */
data class ArbiterInput(
    val automationEnabled: Boolean,
    val screenOn: Boolean,
    val keyguardLocked: Boolean,
    val foregroundPkg: String?,
    val appMap: Map<String, String>,
    val defaultProfile: String,
    val sleepEnabled: Boolean,
    val sleepProfile: String,
    val skipOnMusic: Boolean,
    val musicActive: Boolean,
    val skipOnCharging: Boolean,
    val charging: Boolean,
)

sealed interface Decision {
    /** Leave whatever is active in place. */
    data object None : Decision

    /** Apply [profileId]; [reason] is a UI/notification label. */
    data class Apply(val profileId: String, val reason: String) : Decision
}

object ModeArbiter {

    /**
     * Packages that must not trigger a switch: system chrome and dialogs
     * appear "in front" of the real foreground app. IMEs are matched by
     * substring. The launcher is NOT transient — it is the signal that the
     * user left an app (-> back to default).
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
            if (!input.sleepEnabled) return Decision.None
            if (input.skipOnMusic && input.musicActive) return Decision.None
            if (input.skipOnCharging && input.charging) return Decision.None
            return Decision.Apply(input.sleepProfile, "layar mati")
        }

        if (input.keyguardLocked) return Decision.None

        val pkg = input.foregroundPkg
        if (isTransient(pkg)) return Decision.None

        val mapped = input.appMap[pkg]
        return if (mapped != null) Decision.Apply(mapped, "app")
        else Decision.Apply(input.defaultProfile, "default")
    }
}
