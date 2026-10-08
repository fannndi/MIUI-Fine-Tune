package com.mifinetune.ui

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.mifinetune.core.Status

/**
 * Settings: permissions/background checklist and a small diagnostics block.
 * Turning the service off (on Home) is the restore-to-stock control.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    status: Status?,
    onBack: () -> Unit,
    vm: DynamicProfileViewModel? = null,
) {
    val context = LocalContext.current
    val version = runCatching {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName
    }.getOrNull() ?: "?"
    val syncPerf = vm?.syncMiuiPerf?.collectAsStateWithLifecycle()?.value
    val syncSaver = vm?.syncSaver?.collectAsStateWithLifecycle()?.value
    val syncRefresh = vm?.syncRefresh?.collectAsStateWithLifecycle()?.value
    val gmodeChecker = vm?.gameModeChecker?.collectAsStateWithLifecycle()?.value
    val bridgeLog = vm?.bridgeLog?.collectAsStateWithLifecycle()?.value ?: emptyList()
    val guardBattery = vm?.guardBattery?.collectAsStateWithLifecycle()?.value
    val batteryFloor = vm?.batteryFloor?.collectAsStateWithLifecycle()?.value
    val guardThermal = vm?.guardThermal?.collectAsStateWithLifecycle()?.value
    val thermalCeiling = vm?.thermalCeiling?.collectAsStateWithLifecycle()?.value

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (vm != null && guardBattery != null && batteryFloor != null &&
                guardThermal != null && thermalCeiling != null
            ) {
                GuardsCard(
                    guardBattery = guardBattery,
                    batteryFloor = batteryFloor,
                    guardThermal = guardThermal,
                    thermalCeiling = thermalCeiling,
                    onGuardBattery = vm::setGuardBattery,
                    onBatteryFloor = vm::setBatteryFloor,
                    onGuardThermal = vm::setGuardThermal,
                    onThermalCeiling = vm::setThermalCeiling,
                )
            }

            ElevatedCard(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text("Background", style = MaterialTheme.typography.titleMedium)
                    SetupRow(
                        title = "Root",
                        subtitle = "Required — tuning goes through su",
                        ok = status?.root,
                    )
                    SetupRow(
                        title = "MIUI Autostart",
                        subtitle = "Keeps the service alive",
                        ok = null,
                        actionLabel = "Open",
                        onAction = { openMiuiAutostart(context) },
                    )
                    SetupRow(
                        title = "Battery saver: no restrictions",
                        subtitle = "App info → Battery saver → No restrictions",
                        ok = null,
                        actionLabel = "Open",
                        onAction = { openAppSettings(context) },
                    )
                }
            }

            if (vm != null) {
                ElevatedCard(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("MIUI bridge", style = MaterialTheme.typography.titleMedium)
                        if (syncPerf != null) {
                            Row(
                                Modifier.fillMaxWidth(),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Column(Modifier.weight(1f)) {
                                    Text("Sync MIUI Performance mode", style = MaterialTheme.typography.bodyMedium)
                                    Text(
                                        "Mapped game in front → mirror key written; MIUI's " +
                                            "hidden sheet reads a restricted property and may not flip",
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                Switch(checked = syncPerf, onCheckedChange = vm::setSyncMiuiPerf)
                            }
                        }
                        if (syncSaver != null) {
                            Row(
                                Modifier.fillMaxWidth(),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Column(Modifier.weight(1f)) {
                                    Text("Sync MIUI Battery saver", style = MaterialTheme.typography.bodyMedium)
                                    Text(
                                        "Frugal-mapped app in front → MIUI Battery saver follows " +
                                            "(restores your own state on exit)",
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                Switch(checked = syncSaver, onCheckedChange = vm::setSyncSaver)
                            }
                        }
                        if (syncRefresh != null) {
                            Row(
                                Modifier.fillMaxWidth(),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Column(Modifier.weight(1f)) {
                                    Text("Refresh rate follow", style = MaterialTheme.typography.bodyMedium)
                                    Text(
                                        "Mapped game → 120 Hz, Power Save app → 60 Hz; " +
                                            "your own value returns when neither is in front",
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                Switch(checked = syncRefresh, onCheckedChange = vm::setSyncRefresh)
                            }
                        }
                        if (gmodeChecker != null) {
                            Row(
                                Modifier.fillMaxWidth(),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Column(Modifier.weight(1f)) {
                                    Text("Game-mode checker", style = MaterialTheme.typography.bodyMedium)
                                    Text(
                                        "Warn when MIUI Game Booster still boosts a mapped game",
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                Switch(checked = gmodeChecker, onCheckedChange = vm::setGameModeChecker)
                            }
                        }
                        Text(
                            "Ultra battery saver: MiFineTune retires automatically " +
                                "(restore + stop) — that mode belongs to MIUI.",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (bridgeLog.isNotEmpty()) {
                            Text("Timeline", style = MaterialTheme.typography.labelLarge)
                            Text(
                                bridgeLog.takeLast(8).joinToString("\n"),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }

            ElevatedCard(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Diagnostics", style = MaterialTheme.typography.titleMedium)
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        FrameworkDot("thermal", status?.framework?.miThermald)
                        FrameworkDot("perf", status?.framework?.perfHal)
                        FrameworkDot("root", if (status?.root == true) "granted" else null)
                    }
                    status?.let { st ->
                        Text(
                            "${st.catalog.total} nodes · ${st.catalog.free} free · " +
                                "${st.catalog.present} present",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Text(
                        "MiFineTune $version · surya",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Spacer(Modifier.size(16.dp))
        }
    }
}

@Composable
private fun SetupRow(
    title: String,
    subtitle: String,
    ok: Boolean?,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Surface(
            modifier = Modifier.size(8.dp),
            shape = CircleShape,
            color = when (ok) {
                true -> MaterialTheme.colorScheme.primary
                false -> MaterialTheme.colorScheme.error
                null -> MaterialTheme.colorScheme.outline
            },
        ) {}
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyMedium)
            Text(
                subtitle,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (actionLabel != null && onAction != null) {
            TextButton(onClick = onAction) { Text(actionLabel) }
        } else if (ok == true) {
            Text(
                "OK",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.primary,
            )
        }
    }
}

@Composable
private fun FrameworkDot(label: String, value: String?) {
    val ok = value == "running" || value == "granted"
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        Surface(
            modifier = Modifier.size(8.dp),
            shape = CircleShape,
            color = if (ok) MaterialTheme.colorScheme.primary
            else MaterialTheme.colorScheme.error,
        ) {}
        Text(
            label,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

private fun openMiuiAutostart(context: Context) {
    val candidates = listOf(
        Intent().setClassName(
            "com.miui.securitycenter",
            "com.miui.permcenter.autostart.AutoStartManagementActivity",
        ),
        Intent("miui.intent.action.OP_AUTO_START").addCategory(Intent.CATEGORY_DEFAULT),
    )
    for (intent in candidates) {
        val started = runCatching {
            context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            true
        }.getOrDefault(false)
        if (started) return
    }
    openAppSettings(context)
}

private fun openAppSettings(context: Context) {
    runCatching {
        context.startActivity(
            Intent(
                Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                Uri.fromParts("package", context.packageName, null),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }
}
