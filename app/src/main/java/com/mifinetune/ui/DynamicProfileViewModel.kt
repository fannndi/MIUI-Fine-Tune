package com.mifinetune.ui

import android.app.Application
import android.content.Intent
import android.content.pm.ApplicationInfo
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.core.graphics.drawable.toBitmap
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.mifinetune.dynamic.DynamicProfileConfig
import com.mifinetune.dynamic.DynamicProfileState
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
    val icon: ImageBitmap?,
)

data class DynamicProfileUiState(
    val apps: List<AppEntry> = emptyList(),
    val loading: Boolean = true,
    val query: String = "",
    val rootGranted: Boolean? = null,
)

/**
 * Controller for the Apps Profile and Settings screens: installed-app list,
 * per-app mapping, root status. Kept separate from [HomeViewModel] so the
 * (heavy) icon loading happens only when the user opens the apps screen.
 */
class DynamicProfileViewModel(app: Application) : AndroidViewModel(app) {

    private val config = DynamicProfileConfig.get(app)

    val appMap: StateFlow<Map<String, String>> = config.appMapFlow

    /** MIUI bridge switches (Settings page). */
    val syncMiuiPerf: StateFlow<Boolean> = config.syncMiuiPerfFlow
    val syncSaver: StateFlow<Boolean> = config.syncSaverFlow
    val gameModeChecker: StateFlow<Boolean> = config.gameModeCheckerFlow
    val bridgeLog: StateFlow<List<String>> = DynamicProfileState.bridgeLog

    /** Adaptive guards (Settings page). */
    val guardBattery: StateFlow<Boolean> = config.guardBatteryFlow
    val batteryFloor: StateFlow<Int> = config.batteryFloorFlow
    val guardThermal: StateFlow<Boolean> = config.guardThermalFlow
    val thermalCeiling: StateFlow<Float> = config.thermalCeilingFlow
    val maintenance: StateFlow<Boolean> = config.maintenanceFlow
    val jankBoost: StateFlow<Boolean> = config.jankBoostFlow
    val chargeLimit: StateFlow<Boolean> = config.chargeLimitFlow
    val chargeLimitPct: StateFlow<Int> = config.chargeLimitPctFlow

    private val _state = MutableStateFlow(DynamicProfileUiState())
    val state: StateFlow<DynamicProfileUiState> = _state.asStateFlow()

    init {
        loadApps()
        checkRoot()
    }

    fun setQuery(q: String) = _state.update { it.copy(query = q) }

    fun setAppProfile(pkg: String, profileId: String?) {
        config.setAppProfile(pkg, profileId)
    }

    fun setSyncMiuiPerf(v: Boolean) {
        config.syncMiuiPerf = v
    }

    fun setSyncSaver(v: Boolean) {
        config.syncSaver = v
    }


    fun setGameModeChecker(v: Boolean) {
        config.gameModeChecker = v
    }

    fun setGuardBattery(v: Boolean) {
        config.guardBattery = v
    }

    fun setBatteryFloor(v: Int) {
        config.batteryFloor = v
    }

    fun setGuardThermal(v: Boolean) {
        config.guardThermal = v
    }

    fun setThermalCeiling(v: Float) {
        config.thermalCeiling = v
    }

    fun setMaintenance(v: Boolean) {
        config.maintenance = v
    }

    fun setJankBoost(v: Boolean) {
        config.jankBoost = v
    }

    fun setChargeLimit(v: Boolean) {
        config.chargeLimit = v
    }

    fun setChargeLimitPct(v: Int) {
        config.chargeLimitPct = v
    }

    fun checkRoot() {
        viewModelScope.launch(Dispatchers.IO) {
            val root = runCatching { Tuner.bridge.isRoot() }.getOrDefault(false)
            _state.update { it.copy(rootGranted = root) }
        }
    }

    fun filteredApps(): List<AppEntry> {
        val s = _state.value
        return s.apps
            .filter { s.query.isBlank() || it.label.contains(s.query, true) || it.pkg.contains(s.query, true) }
    }

    private fun loadApps() {
        viewModelScope.launch {
            _state.update { it.copy(loading = true) }
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
                        val isSystem = (ai.flags and ApplicationInfo.FLAG_SYSTEM) != 0
                        if (isSystem) return@mapNotNull null
                        AppEntry(
                            pkg = pkg,
                            label = runCatching { pm.getApplicationLabel(ai).toString() }
                                .getOrDefault(pkg),
                            isGame = ai.category == ApplicationInfo.CATEGORY_GAME,
                            icon = runCatching {
                                pm.getApplicationIcon(pkg).toBitmap(96, 96).asImageBitmap()
                            }.getOrNull(),
                        )
                    }
                    .sortedBy { it.label.lowercase() }
            }
            _state.update { it.copy(apps = apps, loading = false) }
        }
    }
}
