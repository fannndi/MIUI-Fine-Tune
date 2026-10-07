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
}
