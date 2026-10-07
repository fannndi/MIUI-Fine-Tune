package com.mifinetune.automation

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pure-logic tests for the automation decision table. */
class ModeArbiterTest {

    private fun input(
        enabled: Boolean = true,
        screenOn: Boolean = true,
        locked: Boolean = false,
        fg: String? = "com.example.app",
        map: Map<String, String> = emptyMap(),
        base: String = "balance",
        sleep: String = "sleep",
    ) = ArbiterInput(
        automationEnabled = enabled,
        screenOn = screenOn,
        keyguardLocked = locked,
        foregroundPkg = fg,
        appMap = map,
        baseProfile = base,
        sleepProfile = sleep,
    )

    @Test
    fun automationOff_isNone() {
        assertEquals(Decision.None, ModeArbiter.decide(input(enabled = false)))
    }

    @Test
    fun screenOff_appliesSleep() {
        assertEquals(
            Decision.Apply("sleep", "screen off"),
            ModeArbiter.decide(input(screenOn = false)),
        )
    }

    @Test
    fun keyguardLocked_isNone() {
        assertEquals(Decision.None, ModeArbiter.decide(input(locked = true)))
    }

    @Test
    fun mappedApp_appliesMappedProfile() {
        val d = ModeArbiter.decide(
            input(fg = "com.YoStarEN.AzurLane", map = mapOf("com.YoStarEN.AzurLane" to "game")),
        )
        assertEquals(Decision.Apply("game", "app"), d)
    }

    @Test
    fun unmappedApp_appliesBase() {
        val d = ModeArbiter.decide(input(fg = "com.whatsapp", base = "powersave"))
        assertEquals(Decision.Apply("powersave", "base"), d)
    }

    @Test
    fun launcher_revertsToBase() {
        val d = ModeArbiter.decide(input(fg = "com.miui.home", base = "powersave"))
        assertEquals(Decision.Apply("powersave", "base"), d)
    }

    @Test
    fun systemChrome_isTransient() {
        assertEquals(Decision.None, ModeArbiter.decide(input(fg = "com.android.systemui")))
        assertEquals(Decision.None, ModeArbiter.decide(input(fg = "com.mifinetune")))
        assertEquals(Decision.None, ModeArbiter.decide(input(fg = "com.lbe.security.miui")))
        assertEquals(
            Decision.None,
            ModeArbiter.decide(input(fg = "com.google.android.inputmethod.latin")),
        )
    }

    @Test
    fun nullForeground_isNone() {
        assertEquals(Decision.None, ModeArbiter.decide(input(fg = null)))
    }

    @Test
    fun transientPredicate_matchesImeSubstring() {
        assertTrue(ModeArbiter.isTransient("com.sohu.inputmethod.sogou.xiaomi"))
        assertTrue(ModeArbiter.isTransient("com.baidu.input_mi"))
        assertTrue(ModeArbiter.isTransient("com.iflytek.inputmethod.miui"))
    }
}
