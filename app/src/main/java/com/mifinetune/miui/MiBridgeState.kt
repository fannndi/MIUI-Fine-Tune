package com.mifinetune.miui

/**
 * Pure hold/restore state machine for the MIUI bridge (unit tested).
 *
 * When MiFineTune drives a MIUI mode for a mapped app, it must never stomp
 * the user's own choice: the first hold captures the live value as the
 * restore point, the release writes it back. While a hold is active the
 * arbiter must see the USER's value, not our transient write (attribution):
 * a saver we turned on for YouTube must not force the base to powersave
 * after the user backs out.
 */
data class MiBridgeState(
    val perfHeld: Boolean = false,
    val perfSaved: MiStateBridge.PowerMode = MiStateBridge.PowerMode.Balanced,
    val saverHeld: Boolean = false,
    val saverSaved: Boolean = false,
) {

    enum class PerfAction { NONE, WRITE, KEEP, RESTORE }
    enum class SaverAction { NONE, TURN_ON, KEEP, RESTORE }

    /**
     * @param live   current performance switch state (from the device)
     * @param want   bridge wants Performance (mapped game in front)
     */
    fun requestPerf(live: MiStateBridge.PowerMode, want: Boolean): Pair<MiBridgeState, PerfAction> {
        return when {
            want && !perfHeld ->
                copy(perfHeld = true, perfSaved = live) to
                    (if (live == MiStateBridge.PowerMode.Performance) PerfAction.KEEP else PerfAction.WRITE)
            want && perfHeld -> this to PerfAction.KEEP
            !want && perfHeld ->
                copy(perfHeld = false) to PerfAction.RESTORE
            else -> this to PerfAction.NONE
        }
    }

    /**
     * @param live   current battery saver state (from the device)
     * @param want   bridge wants saver ON (frugal-mapped app in front)
     */
    fun requestSaver(live: Boolean, want: Boolean): Pair<MiBridgeState, SaverAction> {
        return when {
            want && !saverHeld ->
                copy(saverHeld = true, saverSaved = live) to
                    (if (live) SaverAction.KEEP else SaverAction.TURN_ON)
            want && saverHeld -> this to SaverAction.KEEP
            !want && saverHeld ->
                copy(saverHeld = false) to SaverAction.RESTORE
            else -> this to SaverAction.NONE
        }
    }

    /**
     * The battery-saver value the arbiter should treat as user intent.
     * While we hold the flag for a mapped app, the user's captured value
     * stands in — our own write is invisible to the decision.
     */
    fun userSaver(live: Boolean): Boolean = if (saverHeld) saverSaved else live
}
