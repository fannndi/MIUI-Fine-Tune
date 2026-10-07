package com.mifinetune.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.mifinetune.core.ApplyReport
import com.mifinetune.core.FtClient
import com.mifinetune.core.LockedKey
import com.mifinetune.core.Plan
import com.mifinetune.core.Profile
import com.mifinetune.core.RootBridge
import com.mifinetune.core.Status
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
 * apply/restore with report dialogs.
 *
 * Responsibility: orchestrating FtClient calls and exposing [HomeUiState].
 * Non-goals: tuning logic (Rust core owns it).
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
) {
    val canAct: Boolean get() = !loading && busy == null
}

data class LockedDetail(val title: String, val items: List<LockedKey>)

class HomeViewModel(app: Application) : AndroidViewModel(app) {

    private val bridge = RootBridge()
    private val client = FtClient(bridge)

    private val _state = MutableStateFlow(HomeUiState())
    val state: StateFlow<HomeUiState> = _state.asStateFlow()

    init {
        refresh()
    }

    fun refresh() {
        viewModelScope.launch {
            _state.update { it.copy(loading = true, error = null) }
            val deployError = bridge.deploy(getApplication())
            if (deployError != null) {
                _state.update { it.copy(loading = false, error = deployError) }
                return@launch
            }
            loadAll()
        }
    }

    private suspend fun loadAll() = withContext(Dispatchers.IO) {
        runCatching {
            val status = client.status()
            val profiles = loadBundledProfiles()
            val cards = profiles.map { p ->
                val res = runCatching { client.plan(p.id) }
                ProfileCard(
                    profile = p,
                    plan = res.getOrNull(),
                    planError = res.exceptionOrNull()?.message,
                )
            }
            _state.update {
                it.copy(loading = false, status = status, cards = cards, error = null)
            }
        }.onFailure { e ->
            _state.update {
                it.copy(
                    loading = false,
                    error = "Probe failed: ${e.message ?: e}" +
                        if (!bridge.isRoot()) " (root not granted?)" else "",
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

    fun apply(profileId: String) {
        if (_state.value.busy != null) return
        _state.update { it.copy(busy = profileId, error = null) }
        viewModelScope.launch(Dispatchers.IO) {
            runCatching {
                val report = client.apply(profileId)
                report to client.status()
            }.onSuccess { (report, status) ->
                val cards = _state.value.cards.map { c ->
                    c.copy(plan = runCatching { client.plan(c.profile.id) }.getOrNull())
                }
                _state.update {
                    it.copy(busy = null, report = report, status = status, cards = cards)
                }
            }.onFailure { e ->
                _state.update {
                    it.copy(busy = null, error = "Apply failed: ${e.message ?: e}")
                }
            }
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
                val report = client.restore()
                report to client.status()
            }.onSuccess { (report, status) ->
                val cards = _state.value.cards.map { c ->
                    c.copy(plan = runCatching { client.plan(c.profile.id) }.getOrNull())
                }
                _state.update {
                    it.copy(busy = null, report = report, status = status, cards = cards)
                }
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
