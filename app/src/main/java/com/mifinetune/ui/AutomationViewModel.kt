package com.mifinetune.ui

import android.app.Application
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.core.graphics.drawable.toBitmap
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.mifinetune.automation.AutomationConfig
import com.mifinetune.automation.AutomationService
import com.mifinetune.automation.AutomationState
import com.mifinetune.automation.ForegroundDetector
import com.mifinetune.core.Tuner
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** One launchable app for the per-app mapping list. */
data class AppEntry(
    val pkg: String,
    val label: String,
    val isGame: Boolean,
    val isSystem: Boolean,
    val icon: ImageBitmap?,
)

data class AutomationUiState(
    val apps: List<AppEntry> = emptyList(),
    val loadingApps: Boolean = true,
    val query: String = "",
    val onlyMapped: Boolean = false,
    val usageAccess: Boolean? = null,
    val rootGranted: Boolean? = null,
    val serviceRunning: Boolean = false,
    val starting: Boolean = false,
    val error: String? = null,
)

/**
 * State for the Automation screen: config (via [AutomationConfig] flows),
 * service control, the installed-apps list, and the MIUI setup checklist.
 *
 * Responsibility: UI orchestration only.
 * Non-goals: decisions/apply (service + arbiter + Tuner).
 */
class AutomationViewModel(app: Application) : AndroidViewModel(app) {

    private val config = AutomationConfig.get(app)

    val enabled = config.enabledFlow
    val defaultProfile = config.defaultProfileFlow
    val sleepEnabled = config.sleepEnabledFlow
    val sleepProfile = config.sleepProfileFlow
    val skipOnMusic = config.skipOnMusicFlow
    val skipOnCharging = config.skipOnChargingFlow
    val bootApply = config.bootApplyFlow
    val showSystemApps = config.showSystemAppsFlow
    val appMap = config.appMapFlow

    private val _state = MutableStateFlow(AutomationUiState())
    val state: StateFlow<AutomationUiState> = _state.asStateFlow()

    init {
        viewModelScope.launch {
            AutomationState.running.collect { r ->
                _state.update { it.copy(serviceRunning = r, starting = if (r) false else it.starting) }
            }
        }
        loadApps()
        checkSetup()
    }

    // --- service control --------------------------------------------------

    fun setEnabled(v: Boolean) {
        if (v) {
            _state.update { it.copy(starting = true, error = null) }
            runCatching {
                config.enabled = true
                AutomationService.start(getApplication())
            }.onFailure { e ->
                config.enabled = false
                _state.update { it.copy(starting = false, error = "Gagal menyalakan: ${e.message}") }
            }
        } else {
            config.enabled = false
            AutomationState.overrideProfile = null
            AutomationService.stop(getApplication())
        }
    }

    private fun refreshIfEnabled() {
        if (config.enabled) {
            runCatching { AutomationService.refresh(getApplication()) }
        }
    }

    // --- config setters ---------------------------------------------------

    fun setDefaultProfile(id: String) {
        config.defaultProfile = id
        refreshIfEnabled()
    }

    fun setSleepEnabled(v: Boolean) {
        config.sleepEnabled = v
        refreshIfEnabled()
    }

    fun setSleepProfile(id: String) {
        config.sleepProfile = id
        refreshIfEnabled()
    }

    fun setSkipOnMusic(v: Boolean) {
        config.skipOnMusic = v
        refreshIfEnabled()
    }

    fun setSkipOnCharging(v: Boolean) {
        config.skipOnCharging = v
        refreshIfEnabled()
    }

    fun setBootApply(v: Boolean) {
        config.bootApply = v
    }

    fun setShowSystemApps(v: Boolean) {
        config.showSystemApps = v
    }

    fun setAppProfile(pkg: String, profileId: String?) {
        config.setAppProfile(pkg, profileId)
        refreshIfEnabled()
    }

    fun setQuery(q: String) = _state.update { it.copy(query = q) }

    fun setOnlyMapped(v: Boolean) = _state.update { it.copy(onlyMapped = v) }

    // --- setup checklist --------------------------------------------------

    fun checkSetup() {
        viewModelScope.launch(Dispatchers.IO) {
            val root = runCatching { Tuner.bridge.isRoot() }.getOrDefault(false)
            val pkg = getApplication<Application>().packageName
            val usage = runCatching {
                Tuner.bridge.sh("appops get $pkg GET_USAGE_STATS").out.contains("allow")
            }.getOrDefault(false)
            _state.update { it.copy(rootGranted = root, usageAccess = usage) }
        }
    }

    fun grantUsageAccess() {
        viewModelScope.launch(Dispatchers.IO) {
            val detector = ForegroundDetector(getApplication(), Tuner.bridge)
            val ok = detector.ensureUsageAccess(Tuner.bridge)
            _state.update { it.copy(usageAccess = ok) }
        }
    }

    // --- apps -------------------------------------------------------------

    fun loadApps() {
        viewModelScope.launch {
            _state.update { it.copy(loadingApps = true) }
            val apps = withContext(Dispatchers.IO) {
                val app = getApplication<Application>()
                val pm = app.packageManager
                val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
                val seen = HashSet<String>()
                pm.queryIntentActivities(intent, 0)
                    .mapNotNull { ri ->
                        val ai = ri.activityInfo.applicationInfo
                        val pkg = ai.packageName
                        if (pkg == app.packageName || !seen.add(pkg)) return@mapNotNull null
                        AppEntry(
                            pkg = pkg,
                            label = runCatching { pm.getApplicationLabel(ai).toString() }
                                .getOrDefault(pkg),
                            isGame = ai.category == ApplicationInfo.CATEGORY_GAME,
                            isSystem = (ai.flags and ApplicationInfo.FLAG_SYSTEM) != 0,
                            icon = runCatching {
                                pm.getApplicationIcon(pkg).toBitmap(96, 96).asImageBitmap()
                            }.getOrNull(),
                        )
                    }
                    .sortedBy { it.label.lowercase() }
            }
            _state.update { it.copy(apps = apps, loadingApps = false) }
        }
    }

    fun clearError() = _state.update { it.copy(error = null) }

    /** Apps filtered for the current query/filters. */
    fun filteredApps(): List<AppEntry> {
        val s = _state.value
        val map = appMap.value
        return s.apps
            .filter { s.query.isBlank() || it.label.contains(s.query, true) || it.pkg.contains(s.query, true) }
            .filter { !s.onlyMapped || map.containsKey(it.pkg) }
            .filter { showSystemApps.value || !it.isSystem }
    }

}
