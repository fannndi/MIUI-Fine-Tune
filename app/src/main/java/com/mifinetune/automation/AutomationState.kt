package com.mifinetune.automation

import kotlinx.coroutines.flow.MutableStateFlow

/**
 * Process-wide automation runtime state (service writes, UI observes).
 *
 * Responsibility: live status + the one-shot manual-override handshake.
 * Non-goals: persistence (AutomationConfig) and decisions (ModeArbiter).
 */
object AutomationState {

    /** True while [AutomationService] is alive. */
    val running = MutableStateFlow(false)

    /** Human-readable reason for the current decision ("default", "layar mati", app label). */
    val reason = MutableStateFlow<String?>(null)

    /** Profile the automation last applied (null until first decision). */
    val appliedProfile = MutableStateFlow<String?>(null)

    /** Last foreground package seen by the detector. */
    val lastForeground = MutableStateFlow<String?>(null)

    /**
     * Manual override: set by the UI when the user taps a profile card while
     * automation is ON. The service consumes it at the next trigger (app
     * switch / screen cycle) — the tap does NOT change the daily default.
     */
    @Volatile
    var overrideProfile: String? = null
}
