package com.mifinetune.dynamic

import kotlinx.coroutines.flow.MutableStateFlow

/** One `applied` event from the daemon (UI waits on seq for dialog flows). */
data class AppliedEvent(
    val seq: Long,
    val profile: String,
    val reason: String,
    val srcPkg: String?,
    val ok: Boolean,
    val wrote: Int,
    val verified: Int,
    val failed: Int,
    val ms: Long,
    val settleMs: Long,
)

/** One `restored`/`retired` event from the daemon. */
data class RestoredEvent(
    val seq: Long,
    val retire: Boolean,
    val ok: Boolean,
    val wrote: Int,
    val verified: Int,
    val failed: Int,
)

/**
 * Process-wide dynamic profile runtime state (daemon writes via the service,
 * UI observes).
 *
 * Responsibility: live status for the UI.
 * Non-goals: persistence (DynamicProfileConfig), decisions (Rust daemon).
 */
object DynamicProfileState {

    /** True while [DynamicProfileService] is alive. */
    val running = MutableStateFlow(false)

    /**
     * Display text of the current decision:
     * "base" | "screen off" | "multi-window" | app label | "MIUI saver".
     */
    val reason = MutableStateFlow<String?>(null)

    /** Profile the daemon last applied (null until first decision). */
    val appliedProfile = MutableStateFlow<String?>(null)

    /** Last foreground package seen by the daemon watcher. */
    val lastForeground = MutableStateFlow<String?>(null)

    /** The second pane's package while split screen / floating window is on. */
    val secondWindow = MutableStateFlow<String?>(null)

    /**
     * Bridge timeline (newest first, capped): MIUI mode writes the bridge
     * performed, so the user can verify behaviour without a cable.
     */
    val bridgeLog = MutableStateFlow<List<String>>(emptyList())

    /** Latest `applied` event (each event is a new instance). */
    val appliedEvents = MutableStateFlow<AppliedEvent?>(null)
    private var appliedSeq = 0L

    /** Latest `restored`/`retired` event (each event is a new instance). */
    val restoredEvents = MutableStateFlow<RestoredEvent?>(null)
    private var restoredSeq = 0L

    fun pushApplied(ev: AppliedEvent) {
        appliedSeq += 1
        appliedEvents.value = ev.copy(seq = appliedSeq)
    }

    fun pushRestored(ev: RestoredEvent) {
        restoredSeq += 1
        restoredEvents.value = ev.copy(seq = restoredSeq)
    }

    fun currentAppliedSeq(): Long = appliedSeq

    fun currentRestoredSeq(): Long = restoredSeq

    fun pushBridgeEvent(msg: String) {
        val time = java.text.SimpleDateFormat("HH:mm", java.util.Locale.US)
            .format(java.util.Date())
        bridgeLog.value = (bridgeLog.value + "$time  $msg").takeLast(20)
    }
}
