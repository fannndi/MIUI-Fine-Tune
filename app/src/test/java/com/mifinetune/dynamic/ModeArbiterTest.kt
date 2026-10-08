package com.mifinetune.dynamic

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pure-logic tests for the dynamic profile decision table. */
class ModeArbiterTest {

    private fun input(
        enabled: Boolean = true,
        screenOn: Boolean = true,
        locked: Boolean = false,
        fg: String? = "com.example.app",
        map: Map<String, String> = emptyMap(),
        base: String = "balance",
        sleep: String = "sleep",
        saverOn: Boolean = false,
        ultraSaver: Boolean = false,
        mw: Boolean = false,
        dynamic: Boolean = true,
    ) = ArbiterInput(
        serviceEnabled = enabled,
        screenOn = screenOn,
        keyguardLocked = locked,
        foregroundPkg = fg,
        appMap = map,
        baseProfile = base,
        sleepProfile = sleep,
        saverOn = saverOn,
        ultraSaver = ultraSaver,
        multiWindow = mw,
        dynamicProfile = dynamic,
    )

    @Test
    fun serviceOff_isNone() {
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

    @Test
    fun transientPredicate_coversSettingsBridgeSurface() {
        // MIUI's hidden PowerModeSettings sheet — opened by the bridge itself
        // for the performance follow — must not read as a real foreground
        assertTrue(ModeArbiter.isTransient("com.android.settings"))
    }

    // --- MIUI bridge rules ------------------------------------------------

    @Test
    fun saver_on_forces_powersave_base_but_mapping_still_wins() {
        val d = ModeArbiter.decide(input(saverOn = true))
        assertEquals(Decision.Apply("powersave", "base"), d)

        val d2 = ModeArbiter.decide(
            input(saverOn = true, fg = "com.YoStarEN.AzurLane", map = mapOf("com.YoStarEN.AzurLane" to "game")),
        )
        assertEquals(Decision.Apply("game", "app"), d2)
    }

    @Test
    fun extreme_saver_retires_the_service() {
        assertEquals(Decision.Retire, ModeArbiter.decide(input(ultraSaver = true)))
        // retire wins over everything, even screen-off
        assertEquals(
            Decision.Retire,
            ModeArbiter.decide(input(ultraSaver = true, screenOn = false)),
        )
    }

    // --- multi-window rule --------------------------------------------------

    @Test
    fun multi_window_forces_balance_over_mapping() {
        val d = ModeArbiter.decide(
            input(fg = "com.YoStarEN.AzurLane", map = mapOf("com.YoStarEN.AzurLane" to "game"), mw = true),
        )
        assertEquals(Decision.Apply("balance", "multi-window"), d)
    }

    @Test
    fun multi_window_forces_balance_over_saver_base() {
        val d = ModeArbiter.decide(input(saverOn = true, mw = true))
        assertEquals(Decision.Apply("balance", "multi-window"), d)
    }

    @Test
    fun multi_window_does_not_beat_screen_off() {
        val d = ModeArbiter.decide(input(screenOn = false, mw = true))
        assertEquals(Decision.Apply("sleep", "screen off"), d)
    }

    // --- dynamic profile rule -----------------------------------------------

    @Test
    fun dynamic_off_mapped_app_falls_back_to_base() {
        val d = ModeArbiter.decide(
            input(
                fg = "com.YoStarEN.AzurLane",
                map = mapOf("com.YoStarEN.AzurLane" to "game"),
                base = "balance",
                dynamic = false,
            ),
        )
        assertEquals(Decision.Apply("balance", "base"), d)
    }

    @Test
    fun dynamic_off_still_forces_powersave_under_saver() {
        val d = ModeArbiter.decide(input(saverOn = true, dynamic = false))
        assertEquals(Decision.Apply("powersave", "base"), d)
    }

    @Test
    fun dynamic_off_still_applies_sleep() {
        val d = ModeArbiter.decide(input(screenOn = false, dynamic = false))
        assertEquals(Decision.Apply("sleep", "screen off"), d)
    }

    @Test
    fun dynamic_off_still_forces_balance_on_multi_window() {
        val d = ModeArbiter.decide(input(mw = true, dynamic = false))
        assertEquals(Decision.Apply("balance", "multi-window"), d)
    }

    @Test
    fun dynamic_off_keeps_transient_noop() {
        assertEquals(Decision.None, ModeArbiter.decide(input(fg = "com.android.systemui", dynamic = false)))
    }

    @Test
    fun dynamic_on_mapped_app_applies_mapped_profile() {
        val d = ModeArbiter.decide(
            input(
                fg = "com.YoStarEN.AzurLane",
                map = mapOf("com.YoStarEN.AzurLane" to "game"),
                dynamic = true,
            ),
        )
        assertEquals(Decision.Apply("game", "app"), d)
    }
}
