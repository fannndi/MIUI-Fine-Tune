package com.mifinetune.miui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Hold/restore + attribution rules of the MIUI bridge (pure state machine).
 */
class MiBridgeStateTest {

    private val B = MiStateBridge.PowerMode.Balanced
    private val P = MiStateBridge.PowerMode.Performance

    // --- perf follow ------------------------------------------------------

    @Test
    fun perf_hold_captures_user_value_then_restores_it() {
        var s = MiBridgeState()
        // user was on Balanced, game enters -> WRITE
        var (n, a) = s.requestPerf(B, want = true)
        assertEquals(MiBridgeState.PerfAction.WRITE, a)
        assertEquals(P, MiStateBridge.PowerMode.of("high"))
        assertTrue(n.perfHeld)
        assertEquals(B, n.perfSaved)

        // while held, repeated requests keep (no flash spam)
        val (n2, a2) = n.requestPerf(P, want = true)
        assertEquals(MiBridgeState.PerfAction.KEEP, a2)
        assertEquals(B, n2.perfSaved)

        // game leaves -> RESTORE to the captured user value
        val (n3, a3) = n2.requestPerf(P, want = false)
        assertEquals(MiBridgeState.PerfAction.RESTORE, a3)
        assertFalse(n3.perfHeld)
        assertEquals(B, n3.perfSaved)
    }

    @Test
    fun perf_hold_when_user_already_on_performance_keeps_without_write() {
        val s = MiBridgeState()
        val (n, a) = s.requestPerf(P, want = true)
        assertEquals(MiBridgeState.PerfAction.KEEP, a)
        assertTrue(n.perfHeld)
        assertEquals(P, n.perfSaved)
    }

    // --- saver follow + attribution ---------------------------------------

    @Test
    fun saver_hold_turns_on_then_restores() {
        var s = MiBridgeState()
        val (n, a) = s.requestSaver(live = false, want = true)
        assertEquals(MiBridgeState.SaverAction.TURN_ON, a)
        assertTrue(n.saverHeld)
        assertFalse(n.saverSaved)

        val (n2, a2) = n.requestSaver(live = true, want = false)
        assertEquals(MiBridgeState.SaverAction.RESTORE, a2)
        assertFalse(n2.saverHeld)
    }

    @Test
    fun user_saver_attribution_our_hold_is_invisible_to_arbiter() {
        var s = MiBridgeState()
        // we hold the saver for a mapped app (user's own was off)
        val (n, _) = s.requestSaver(live = false, want = true)
        // while held, the arbiter must NOT see our write as user intent
        assertFalse(n.userSaver(live = true))
        // after release it sees the live flag again
        val (n2, _) = n.requestSaver(live = true, want = false)
        assertTrue(n2.userSaver(live = true))
    }

    @Test
    fun user_saver_own_choice_passes_through() {
        val s = MiBridgeState(saverHeld = false)
        assertTrue(s.userSaver(live = true))
        assertFalse(s.userSaver(live = false))
    }

    // --- crash recovery semantics ----------------------------------------

    @Test
    fun recovered_hold_restores_captured_value_not_live() {
        // service died while holding (user's saver was off, ours is on)
        val s = MiBridgeState(saverHeld = true, saverSaved = false)
        // conditions gone -> release must write the user's value (false)
        val (n, a) = s.requestSaver(live = true, want = false)
        assertEquals(MiBridgeState.SaverAction.RESTORE, a)
        assertEquals(false, n.saverSaved)
    }
}
