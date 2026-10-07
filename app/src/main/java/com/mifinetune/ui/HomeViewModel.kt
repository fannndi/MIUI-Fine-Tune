package com.mifinetune.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.mifinetune.automation.AutomationConfig
import com.mifinetune.automation.AutomationService
import com.mifinetune.automation.AutomationState
import com.mifinetune.core.ApplyReport
import com.mifinetune.core.LockedKey
import com.mifinetune.core.Plan
import com.mifinetune.core.Profile
import com.mifinetune.core.RootBridge
import com.mifinetune.core.Status
import com.mifinetune.core.Tuner
import com.mifinetune.core.parseProfiles
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * UI state machine: boot-time deploy + probe, profile cards with live plans,
 * apply/restore with report dialogs, automation status mirror.
 *
 * Responsibility: orchestrating [Tuner] calls and exposing [HomeUiState].
 * Non-goals: tuning logic (Rust core), automation decisions (ModeArbiter).
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
    /** Profile id (or "restore") while a root action runs. */
    val busy: String? = null,
    val error: String? = null,
    val report: ApplyReport? = null,
    val confirmRestore: Boolean = false,
    val lockedDetail: LockedDetail? = null,
    /** ROM the bundled profile pack was built/audited against. */
    val packRom: String? = null,
    /** Drift guard: true while the periodic verify loop runs. */
    val guardActive: Boolean = false,
    /** Number of drifted keys corrected by the guard since app start. */
    val driftFixed: Int = 0,
    // --- automation mirror ---
    val automationEnabled: Boolean = false,
    val automationRunning: Boolean = false,
    val automationReason: String? = null,
    val defaultProfile: String = AutomationConfig.DEFAULT_PROFILE,
    val sleepEnabled: Boolean = true,
    val mappedCount: Int = 0,
) {
    val canAct: Boolean get() = !loading && busy == null
}

data class LockedDetail(val title: String, val items: List<LockedKey>)

class HomeViewModel(app: Application) : AndroidViewModel(app) {

    private val config = AutomationConfig.get(app)

    private val _state = MutableStateFlow(HomeUiState())
    val state: StateFlow<HomeUiState> = _state.asStateFlow()

    init {
        observeAutomation()
        refresh()
    }

    private fun observeAutomation() {
        viewModelScope.launch {
            config.enabledFlow.collect { v -> _state.update { it.copy(automationEnabled = v) } }
        }
        viewModelScope.launch {
            config.defaultProfileFlow.collect { v -> _state.update { it.copy(defaultProfile = v) } }
        }
        viewModelScope.launch {
            config.sleepEnabledFlow.collect { v -> _state.update { it.copy(sleepEnabled = v) } }
        }
        viewModelScope.launch {
            config.appMapFlow.collect { v -> _state.update { it.copy(mappedCount = v.size) } }
        }
        viewModelScope.launch {
            AutomationState.running.collect { v -> _state.update { it.copy(automationRunning = v) } }
        }
        viewModelScope.launch {
            AutomationState.reason.collect { v -> _state.update { it.copy(automationReason = v) } }
        }
        viewModelScope.launch {
            Tuner.guardActive.collect { v -> _state.update { it.copy(guardActive = v) } }
        }
        viewModelScope.launch {
            Tuner.driftFixed.collect { v -> _state.update { it.copy(driftFixed = v) } }
        }
    }

    fun refresh() {
        viewModelScope.launch {
            _state.update { it.copy(loading = true, error = null) }
            val deployError = Tuner.deploy(getApplication())
            if (deployError != null) {
                _state.update { it.copy(loading = false, error = deployError) }
                return@launch
            }
            // start the automation service while the app is still foreground
            // (background FGS start would be rejected by the OS)
            maybeStartService()
            loadAll()
        }
    }

    private suspend fun loadAll() = withContext(Dispatchers.IO) {
        runCatching {
            val status = Tuner.status()
            val profiles = loadBundledProfiles().filter { !it.hidden }
            val cards = profiles.map { p -> planCard(p) }
            val packRom = runCatching {
                getApplication<Application>().assets
                    .open(RootBridge.ASSET_PROFILES)
                    .bufferedReader().use { r -> JSONObject(r.readText()).optString("rom") }
                    .ifEmpty { null }
            }.getOrNull()
            _state.update {
                it.copy(
                    loading = false,
                    status = status,
                    cards = cards,
                    packRom = packRom,
                    error = null,
                )
            }
            if (status.active != null) Tuner.ensureGuard(viewModelScope)
        }.onFailure { e ->
            _state.update {
                it.copy(
                    loading = false,
                    error = "Probe failed: ${e.message ?: e}" +
                        if (!Tuner.bridge.isRoot()) " (root not granted?)" else "",
                )
            }
        }
    }

    private fun loadBundledProfiles(): List<Profile> {
        val json = getApplication<Application>().assets
            .open(RootBridge.ASSET_PROFILES)
            .bufferedReader().use { it.readText() }
        return parseProfiles(JSONObject(json)).profiles
    }

    private suspend fun planCard(p: Profile): ProfileCard {
        val res = runCatching { Tuner.plan(p.id) }
        return ProfileCard(
            profile = p,
            plan = res.getOrNull(),
            planError = res.exceptionOrNull()?.message,
        )
    }

    private suspend fun refreshPlans(): List<ProfileCard> =
        _state.value.cards.map { planCard(it.profile) }

    fun apply(profileId: String) {
        if (_state.value.busy != null) return
        _state.update { it.copy(busy = profileId, error = null) }
        viewModelScope.launch(Dispatchers.IO) {
            runCatching {
                val report = Tuner.apply(profileId)
                report to Tuner.status()
            }.onSuccess { (report, status) ->
                // Manual tap while automation runs = temporary override, it
                // does not change the daily default (consumed at next trigger).
                if (config.enabled) AutomationState.overrideProfile = profileId
                val cards = refreshPlans()
                _state.update {
                    it.copy(busy = null, report = report, status = status, cards = cards)
                }
                if (status.active != null) Tuner.ensureGuard(viewModelScope)
            }.onFailure { e ->
                _state.update {
                    it.copy(busy = null, error = "Apply failed: ${e.message ?: e}")
                }
            }
        }
    }

    fun setDefaultProfile(id: String) {
        config.defaultProfile = id
        if (config.enabled) {
            runCatching { AutomationService.refresh(getApplication()) }
        }
    }

    /** Bring the service back if automation is on but the service is gone. */
    private fun maybeStartService() {
        if (config.enabled && !AutomationState.running.value) {
            runCatching { AutomationService.start(getApplication()) }
                .onFailure { e ->
                    android.util.Log.w("MiFineTune", "automation service start failed: $e")
                }
        }
    }

    fun setAutomationEnabled(v: Boolean) {
        if (v) {
            runCatching {
                config.enabled = true
                AutomationService.start(getApplication())
            }.onFailure { e ->
                config.enabled = false
                _state.update { it.copy(error = "Gagal menyalakan automasi: ${e.message}") }
            }
        } else {
            config.enabled = false
            AutomationState.overrideProfile = null
            AutomationService.stop(getApplication())
        }
    }

    fun askRestore() {
        _state.update { it.copy(confirmRestore = true) }
    }

    fun cancelRestore() {
        _state.update { it.copy(confirmRestore = false) }
    }

    fun restore() {
        if (_state.value.busy != null) return
        _state.update { it.copy(busy = "restore", confirmRestore = false, error = null) }
        viewModelScope.launch(Dispatchers.IO) {
            runCatching {
                val report = Tuner.restore()
                report to Tuner.status()
            }.onSuccess { (report, status) ->
                if (report.ok) {
                    // Restore = explicit "stop touching my phone": pause automation.
                    config.enabled = false
                    AutomationState.overrideProfile = null
                    runCatching { AutomationService.stop(getApplication()) }
                }
                val cards = refreshPlans()
                _state.update {
                    it.copy(busy = null, report = report, status = status, cards = cards)
                }
                Tuner.stopGuard()
            }.onFailure { e ->
                _state.update {
                    it.copy(busy = null, error = "Restore failed: ${e.message ?: e}")
                }
            }
        }
    }

    fun dismissReport() = _state.update { it.copy(report = null) }

    fun showLocked(title: String, items: List<LockedKey>) =
        _state.update { it.copy(lockedDetail = LockedDetail(title, items)) }

    fun dismissLocked() = _state.update { it.copy(lockedDetail = null) }

    fun clearError() = _state.update { it.copy(error = null) }
}
