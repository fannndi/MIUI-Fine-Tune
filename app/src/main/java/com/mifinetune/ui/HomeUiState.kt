package com.mifinetune.ui

import com.mifinetune.core.ApplyReport
import com.mifinetune.core.LockedKey
import com.mifinetune.core.Plan
import com.mifinetune.core.Profile
import com.mifinetune.core.Status

/**
 * UI state shapes for the Home screen (kept separate from the
 * ViewModel so the state contract is greppable on its own).
 */

data class ProfileCard(
    val profile: Profile,
    val plan: Plan?,
    val planError: String? = null,
)

data class HomeUiState(
    val loading: Boolean = true,
    val status: Status? = null,
    val cards: List<ProfileCard> = emptyList(),
    /** Profile id (or "service-off") while a root action runs. */
    val busy: String? = null,
    val error: String? = null,
    val report: ApplyReport? = null,
    val confirmServiceOff: Boolean = false,
    val lockedDetail: LockedDetail? = null,
    /** ROM the bundled profile pack was built/audited against. */
    val packRom: String? = null,
    /** Drift guard: true while the periodic verify loop runs. */
    val guardActive: Boolean = false,
    /** Number of drifted keys corrected by the guard since app start. */
    val driftFixed: Int = 0,
    // --- service mirror ---
    val serviceEnabled: Boolean = false,
    val serviceRunning: Boolean = false,
    val serviceReason: String? = null,
    /** Dynamic Profile switch (mapped apps auto-override the base). */
    val dynamicEnabled: Boolean = true,
    val mappedCount: Int = 0,
) {
    val canAct: Boolean get() = !loading && busy == null
}

data class LockedDetail(val title: String, val items: List<LockedKey>)
