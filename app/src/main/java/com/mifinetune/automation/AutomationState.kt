package com.mifinetune.automation

import kotlinx.coroutines.flow.MutableStateFlow

/**
 * Process-wide automation runtime state (service writes, UI observes).
 *
 * Responsibility: live status for the UI.
 * Non-goals: persistence (AutomationConfig) and decisions (ModeArbiter).
 */
object AutomationState {

    /** True while [AutomationService] is alive. */
    val running = MutableStateFlow(false)

    /**
     * Human-readable trigger of the current decision:
     * "base" | "screen off" | application label.
     */
    val reason = MutableStateFlow<String?>(null)

    /** Profile the automation last applied (null until first decision). */
    val appliedProfile = MutableStateFlow<String?>(null)

    /** Last foreground package seen by the watcher. */
    val lastForeground = MutableStateFlow<String?>(null)

    /** The second pane's package while split screen / floating window is on. */
    val secondWindow = MutableStateFlow<String?>(null)

    /**
     * Bridge timeline (newest first, capped): MIUI mode writes the bridge
     * performed, so the user can verify behaviour without a cable.
     */
    val bridgeLog = MutableStateFlow<List<String>>(emptyList())

    fun pushBridgeEvent(msg: String) {
        val time = java.text.SimpleDateFormat("HH:mm", java.util.Locale.US)
            .format(java.util.Date())
        bridgeLog.value = (bridgeLog.value + "$time  $msg").takeLast(20)
    }
}
