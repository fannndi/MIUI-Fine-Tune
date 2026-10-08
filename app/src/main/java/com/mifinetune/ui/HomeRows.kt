package com.mifinetune.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Apps
import androidx.compose.material.icons.filled.AutoMode
import androidx.compose.material.icons.filled.Balance
import androidx.compose.material.icons.filled.BatterySaver
import androidx.compose.material.icons.filled.MonitorHeart
import androidx.compose.material.icons.filled.PowerSettingsNew
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Sports
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.mifinetune.core.ApplyReport
import com.mifinetune.core.LockedKey
import com.mifinetune.core.Op
import com.mifinetune.core.Status
import com.mifinetune.dynamic.EnvSnapshot


// --- profile row -------------------------------------------------------------

private fun iconFor(id: String): ImageVector = when (id) {
    "powersave" -> Icons.Default.BatterySaver
    "balance" -> Icons.Default.Balance
    else -> Icons.Default.Sports
}

@Composable
internal fun ProfileRow(
    card: ProfileCard,
    active: Boolean,
    busy: Boolean,
    enabled: Boolean,
    onApply: () -> Unit,
    onDetail: () -> Unit,
) {
    val plan = card.plan
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled && !busy, onClick = onApply),
        colors = if (active) {
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.primaryContainer,
                contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
            )
        } else {
            CardDefaults.cardColors()
        },
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(
                modifier = Modifier.size(36.dp),
                shape = CircleShape,
                color = if (active) MaterialTheme.colorScheme.primary
                else MaterialTheme.colorScheme.primaryContainer,
            ) {
                Icon(
                    iconFor(card.profile.id),
                    contentDescription = null,
                    modifier = Modifier.padding(8.dp),
                    tint = if (active) MaterialTheme.colorScheme.onPrimary
                    else MaterialTheme.colorScheme.onPrimaryContainer,
                )
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        card.profile.label,
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                    if (active) {
                        Spacer(Modifier.width(8.dp))
                        SuggestionPill(
                            "ACTIVE",
                            MaterialTheme.colorScheme.primary,
                            MaterialTheme.colorScheme.onPrimary,
                        )
                    }
                }
                Text(
                    ProfileLabels.shortDesc(card.profile.id),
                    style = MaterialTheme.typography.labelSmall,
                    color = if (active) MaterialTheme.colorScheme.onPrimaryContainer
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                )
            }
            when {
                busy -> CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                plan != null && !plan.ok -> Text(
                    "Unavailable",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.error,
                )
                else -> TextButton(onClick = onDetail) { Text("Detail") }
            }
        }
    }
}

// --- apps entry + service ----------------------------------------------------

@Composable
internal fun AppsRow(mappedCount: Int, onClick: () -> Unit) {
    ElevatedCard(Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconCircle(Icons.Default.Apps)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text("Apps Profile", style = MaterialTheme.typography.titleMedium)
                Text(
                    if (mappedCount > 0) "Per-app rules · $mappedCount apps"
                    else "Per-app rules",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Icon(
                Icons.AutoMirrored.Filled.KeyboardArrowRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
internal fun ServiceRow(state: HomeUiState, onToggle: (Boolean) -> Unit) {
    val active = state.status?.active
    val statusText = when {
        state.serviceRunning && active != null -> {
            val reason = state.serviceReason
            val suffix = if (reason.isNullOrEmpty() || reason == "base") null else reason
            "Active · ${ProfileLabels.of(active)}" + (suffix?.let { " · $it" } ?: "")
        }
        state.serviceRunning -> "On · stock"
        state.serviceEnabled -> "Starting…"
        else -> "Off · stock"
    }
    ElevatedCard(Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconCircle(Icons.Default.PowerSettingsNew)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text("Service", style = MaterialTheme.typography.titleMedium)
                Text(
                    statusText,
                    style = MaterialTheme.typography.labelSmall,
                    color = if (state.serviceRunning) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(checked = state.serviceEnabled, onCheckedChange = onToggle)
        }
    }
}

/**
 * Dynamic Profile: the auto-switch switch. OFF keeps the universal profile
 * no matter which mapped app is opened; sleep / multi-window / MIUI-saver
 * rules still apply. Requires the Service to be on.
 */
@Composable
internal fun DynamicProfileRow(state: HomeUiState, onToggle: (Boolean) -> Unit) {
    val serviceOn = state.serviceEnabled
    val statusText = when {
        !serviceOn -> "Turn on Service first"
        state.dynamicEnabled -> "App profiles switch automatically"
        else -> "Universal profile only"
    }
    ElevatedCard(Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconCircle(Icons.Default.AutoMode)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text("Dynamic Profile", style = MaterialTheme.typography.titleMedium)
                Text(
                    statusText,
                    style = MaterialTheme.typography.labelSmall,
                    color = if (serviceOn && state.dynamicEnabled) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(
                checked = state.dynamicEnabled,
                onCheckedChange = onToggle,
                enabled = serviceOn,
            )
        }
    }
}

// --- diagnostics entry ---------------------------------------------------------

@Composable
internal fun DiagnosticsRow(env: EnvSnapshot?, onClick: () -> Unit) {
    val subtitle = if (env != null) {
        listOfNotNull(
            env.batteryPct?.let { "$it%" },
            env.cpuTempC?.let { String.format(java.util.Locale.US, "%.1f °C", it) },
            env.gpuBusyPct?.let { "GPU $it%" },
        ).joinToString(" · ").ifEmpty { "Daemon · battery · thermal · log" }
    } else {
        "Daemon · battery · thermal · log"
    }
    ElevatedCard(Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconCircle(Icons.Default.MonitorHeart)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text("Diagnostics", style = MaterialTheme.typography.titleMedium)
                Text(
                    subtitle,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Icon(
                Icons.AutoMirrored.Filled.KeyboardArrowRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

// --- dialogs -----------------------------------------------------------------

