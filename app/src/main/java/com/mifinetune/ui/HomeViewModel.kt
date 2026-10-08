package com.mifinetune.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.mifinetune.dynamic.DaemonLink
import com.mifinetune.dynamic.DynamicProfileConfig
import com.mifinetune.dynamic.DynamicProfileService
import com.mifinetune.dynamic.DynamicProfileState
import com.mifinetune.dynamic.RestoredEvent
import com.mifinetune.core.ApplyReport
import com.mifinetune.core.LockedKey
import com.mifinetune.core.Plan
import com.mifinetune.core.Profile
import com.mifinetune.core.RootBridge
import com.mifinetune.core.Status
import com.mifinetune.core.Tuner
import com.mifinetune.core.parseProfiles
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONObject

/**
 * UI state machine: boot-time deploy + probe, profile rows with live plans,
 * apply/service-off with report dialogs, service status mirror.
 *
 * Responsibility: orchestrating [Tuner] calls and exposing [HomeUiState].
 * Non-goals: tuning logic (Rust core), service decisions (Rust daemon).
 */
class HomeViewModel(app: Application) : AndroidViewModel(app) {

    private val config = DynamicProfileConfig.get(app)

    private val _state = MutableStateFlow(HomeUiState())
    val state: StateFlow<HomeUiState> = _state.asStateFlow()

    init {
        observeService()
        refresh()
    }

    private fun observeService() {
        viewModelScope.launch {
            config.enabledFlow.collect { v -> _state.update { it.copy(serviceEnabled = v) } }
        }
        viewModelScope.launch {
            config.dynamicEnabledFlow.collect { v -> _state.update { it.copy(dynamicEnabled = v) } }
        }
        viewModelScope.launch {
            config.appMapFlow.collect { v -> _state.update { it.copy(mappedCount = v.size) } }
        }
        viewModelScope.launch {
            DynamicProfileState.running.collect { v -> _state.update { it.copy(serviceRunning = v) } }
        }
        viewModelScope.launch {
            DynamicProfileState.reason.collect { v -> _state.update { it.copy(serviceReason = v) } }
        }
        viewModelScope.launch {
            DynamicProfileState.appliedProfile.collect { id ->
                // the service switched profile in the background while this
                // screen is open -> keep status + plans fresh
                val cur = _state.value.status?.active
                if (id != null && id != cur && _state.value.busy == null) {
                    runCatching {
                        val st = Tuner.status()
                        st to refreshPlans()
                    }.onSuccess { (st, cards) ->
                        _state.update { it.copy(status = st, cards = cards) }
                    }
                }
            }
        }
        viewModelScope.launch {
            Tuner.guardActive.collect { v -> _state.update { it.copy(guardActive = v) } }
        }
        viewModelScope.launch {
            Tuner.driftFixed.collect { v -> _state.update { it.copy(driftFixed = v) } }
        }
    }

    /** Quiet status+plans refresh (on resume) — no spinner, no deploy. */
    fun refreshLight() {
        if (_state.value.loading || _state.value.busy != null) return
        viewModelScope.launch(Dispatchers.IO) {
            runCatching {
                val st = Tuner.status()
                st to refreshPlans()
            }.onSuccess { (st, cards) ->
                _state.update { it.copy(status = st, cards = cards) }
            }
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
            // start the service while the app is still foreground
            // (background FGS start would be rejected by the OS)
            maybeStartService()
            loadAll()
        }
    }

    private suspend fun loadAll() = withContext(Dispatchers.IO) {
        runCatching {
            val status = Tuner.status()
            val profiles = loadBundledProfiles().filter { !it.hidden }
            val cards = coroutineScope { profiles.map { async { planCard(it) } }.awaitAll() }
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
            // manual-only drift protection (same rule as apply(); the
            // service owns its own periodic re-assert)
            if (status.active != null && !config.enabled) {
                Tuner.ensureGuard(viewModelScope)
            }
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

    private suspend fun refreshPlans(): List<ProfileCard> {
        val cards = _state.value.cards
        return coroutineScope { cards.map { async { planCard(it.profile) } }.awaitAll() }
    }

    /** Manual tap: apply now and remember it as the universal base. */
    fun apply(profileId: String) {
        if (_state.value.busy != null) return
        _state.update { it.copy(busy = profileId, error = null) }
        viewModelScope.launch(Dispatchers.IO) {
            if (DaemonLink.connected()) applyViaDaemon(profileId) else applyViaCli(profileId)
        }
    }

    /**
     * Service on: the daemon is the single writer — write the base, ask it
     * to apply now, wait for the applied event (it carries real counters).
     */
    private suspend fun applyViaDaemon(profileId: String) {
        config.baseProfile = profileId
        val seq = DynamicProfileState.currentAppliedSeq()
        val sent = DaemonLink.send(JSONObject().put("cmd", "set_base").put("profile", profileId))
        val ev = if (sent) {
            withTimeoutOrNull(15_000) {
                DynamicProfileState.appliedEvents.filterNotNull().first { it.seq > seq }
            }
        } else {
            null
        }
        val status = runCatching { Tuner.status() }.getOrNull()
        val cards = refreshPlans()
        val report = ev?.let {
            ApplyReport(
                mode = "apply",
                profile = it.profile,
                wrote = it.wrote,
                unchanged = 0,
                verified = it.verified,
                failed = it.failed,
                locked = emptyList(),
                results = emptyList(),
                ok = it.ok,
                snapshotCreated = false,
                active = if (it.ok) it.profile else null,
            )
        }
        _state.update {
            it.copy(
                busy = null,
                report = report,
                status = status,
                cards = cards,
                error = if (ev == null) "Daemon did not confirm the apply" else null,
            )
        }
    }

    /** Service off: direct CLI path (no daemon is running). */
    private suspend fun applyViaCli(profileId: String) {
        runCatching {
            val report = Tuner.apply(profileId)
            report to Tuner.status()
        }.onSuccess { (report, status) ->
            if (report.ok) config.baseProfile = profileId
            // mirror into the shared dynamic profile state so the Service row
            // shows the manual decision with a fresh reason (the service
            // will overwrite both on its next own decision)
            if (report.ok) {
                DynamicProfileState.appliedProfile.value = profileId
                DynamicProfileState.reason.value = null
            }
            val cards = refreshPlans()
            _state.update {
                it.copy(busy = null, report = report, status = status, cards = cards)
            }
            // manual-only drift protection: when the service is off, the
            // periodic guard keeps the hand-chosen profile intact (same
            // profile as state.active — no arbitration conflicts). With
            // service on, the service loop owns drift handling.
            if (status.active != null && !config.enabled) {
                Tuner.ensureGuard(viewModelScope)
            }
        }.onFailure { e ->
            _state.update {
                it.copy(busy = null, error = "Apply failed: ${e.message ?: e}")
            }
        }
    }

    /** Switch toggle: ON starts the service; OFF restores stock and stops. */
    fun onServiceToggle(v: Boolean) {
        if (v) {
            setServiceEnabled(true)
        } else if (_state.value.status?.snapshot != null) {
            _state.update { it.copy(confirmServiceOff = true) }
        } else {
            setServiceEnabled(false)
        }
    }

    /**
     * Dynamic Profile toggle: pure config write. The running service
     * observes the flow and re-evaluates instantly; while the service is
     * off nothing happens (the value is picked up at next start).
     */
    fun onDynamicToggle(v: Boolean) {
        config.dynamicEnabled = v
    }

    fun confirmServiceOff() {
        _state.update { it.copy(confirmServiceOff = false) }
        setServiceEnabled(false)
    }

    fun cancelServiceOff() {
        _state.update { it.copy(confirmServiceOff = false) }
    }

    private fun setServiceEnabled(v: Boolean) {
        if (v) {
            runCatching {
                config.enabled = true
                DynamicProfileService.start(getApplication())
            }.onFailure { e ->
                config.enabled = false
                _state.update { it.copy(error = "Failed to start service: ${e.message}") }
            }
            return
        }
        // OFF = no intervention: restore stock, then stop everything.
        // Daemon connected -> it is the single writer (restore via IPC);
        // otherwise the CLI path (no daemon running).
        viewModelScope.launch(Dispatchers.IO) {
            _state.update { it.copy(busy = "service-off", error = null) }
            config.enabled = false
            val report = if (DaemonLink.connected()) restoreViaDaemon() else restoreViaCli()
            runCatching { DynamicProfileService.stop(getApplication()) }
            Tuner.stopGuard()
            val status = runCatching { Tuner.status() }.getOrNull()
            val cards = refreshPlans()
            _state.update {
                it.copy(busy = null, report = report, status = status, cards = cards)
            }
        }
    }

    /** Service on: restore through the daemon, wait for the real counters. */
    private suspend fun restoreViaDaemon(): ApplyReport? {
        var ev = sendRestoreAndWait()
        if (ev?.ok == false) {
            // transient race with MIUI/thermal writes -> one retry
            delay(1_500)
            ev = sendRestoreAndWait() ?: ev
        }
        return ev?.let {
            ApplyReport(
                mode = "restore",
                profile = null,
                wrote = it.wrote,
                unchanged = 0,
                verified = it.verified,
                failed = it.failed,
                locked = emptyList(),
                results = emptyList(),
                ok = it.ok,
                snapshotCreated = false,
                active = null,
            )
        }
    }

    private suspend fun sendRestoreAndWait(): RestoredEvent? {
        val seq = DynamicProfileState.currentRestoredSeq()
        DaemonLink.send(JSONObject().put("cmd", "restore"))
        return withTimeoutOrNull(15_000) {
            DynamicProfileState.restoredEvents.filterNotNull().first { it.seq > seq }
        }
    }

    /** Service off: direct CLI path with one transient retry. */
    private suspend fun restoreViaCli(): ApplyReport? {
        var report = runCatching { Tuner.restore() }.getOrNull()
        if (report?.ok == false) {
            delay(1_500)
            report = runCatching { Tuner.restore() }.getOrNull() ?: report
        }
        return report
    }

    /** Bring the service back if it is enabled but gone (update, MIUI kill). */
    private fun maybeStartService() {
        if (config.enabled && !DynamicProfileState.running.value) {
            runCatching { DynamicProfileService.start(getApplication()) }
                .onFailure { e ->
                    android.util.Log.w("MiFineTune", "service start failed: $e")
                }
        }
    }

    fun dismissReport() = _state.update { it.copy(report = null) }

    fun showLocked(title: String, items: List<LockedKey>) =
        _state.update { it.copy(lockedDetail = LockedDetail(title, items)) }

    fun dismissLocked() = _state.update { it.copy(lockedDetail = null) }

    fun clearError() = _state.update { it.copy(error = null) }
}
