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
        default: String = "balance",
        sleepEnabled: Boolean = true,
        sleepProfile: String = "sleep",
        skipMusic: Boolean = false,
        music: Boolean = false,
        skipCharging: Boolean = false,
        charging: Boolean = false,
    ) = ArbiterInput(
        automationEnabled = enabled,
        screenOn = screenOn,
        keyguardLocked = locked,
        foregroundPkg = fg,
        appMap = map,
        defaultProfile = default,
        sleepEnabled = sleepEnabled,
        sleepProfile = sleepProfile,
        skipOnMusic = skipMusic,
        musicActive = music,
        skipOnCharging = skipCharging,
        charging = charging,
    )

    @Test
    fun automationOff_isNone() {
        assertEquals(Decision.None, ModeArbiter.decide(input(enabled = false)))
    }

    @Test
    fun screenOff_appliesSleep() {
        assertEquals(Decision.Apply("sleep", "layar mati"), ModeArbiter.decide(input(screenOn = false)))
    }

    @Test
    fun screenOff_sleepDisabled_isNone() {
        assertEquals(Decision.None, ModeArbiter.decide(input(screenOn = false, sleepEnabled = false)))
    }

    @Test
    fun screenOff_skipOnMusic_respected() {
        assertEquals(
            Decision.None,
            ModeArbiter.decide(input(screenOn = false, skipMusic = true, music = true)),
        )
        // music not playing -> sleep still applies
        assertEquals(
            Decision.Apply("sleep", "layar mati"),
            ModeArbiter.decide(input(screenOn = false, skipMusic = true, music = false)),
        )
    }

    @Test
    fun screenOff_skipOnCharging_respected() {
        assertEquals(
            Decision.None,
            ModeArbiter.decide(input(screenOn = false, skipCharging = true, charging = true)),
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
    fun unmappedApp_appliesDefault() {
        val d = ModeArbiter.decide(input(fg = "com.whatsapp", default = "powersave"))
        assertEquals(Decision.Apply("powersave", "default"), d)
    }

    @Test
    fun launcher_revertsToDefault() {
        val d = ModeArbiter.decide(input(fg = "com.miui.home", default = "powersave"))
        assertEquals(Decision.Apply("powersave", "default"), d)
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
