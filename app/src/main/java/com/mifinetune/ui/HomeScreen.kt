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

/**
 * Home: compact profile rows (tap = apply + base), the Apps Profile entry and
 * the Service switch. Everything else lives in Settings.
 */

private enum class HomeDest { HOME, APPS, SETTINGS }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(vm: HomeViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }
    var dest by remember { mutableStateOf(HomeDest.HOME) }
    var detailProfileId by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(state.error) {
        state.error?.let {
            snackbarHostState.showSnackbar(it)
            vm.clearError()
        }
    }

    // keep the status fresh when the app comes back to the foreground (the
    // service may have switched profiles while we were away)
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { vm.refreshLight() }

    BackHandler(enabled = dest != HomeDest.HOME) { dest = HomeDest.HOME }

    when (dest) {
        HomeDest.APPS -> {
            val avm: AutomationViewModel = viewModel()
            AppsProfileScreen(vm = avm, onBack = { dest = HomeDest.HOME })
        }
        HomeDest.SETTINGS -> {
            val avm: AutomationViewModel = viewModel()
            SettingsScreen(
                status = state.status,
                onBack = { dest = HomeDest.HOME },
                vm = avm,
            )
        }
        HomeDest.HOME -> {
            Scaffold(
                topBar = {
                    TopAppBar(
                        title = { Text("MiFineTune", style = MaterialTheme.typography.titleLarge) },
                        actions = {
                            IconButton(onClick = { dest = HomeDest.SETTINGS }) {
                                Icon(Icons.Default.Settings, contentDescription = "Settings")
                            }
                        },
                        colors = TopAppBarDefaults.topAppBarColors(
                            containerColor = MaterialTheme.colorScheme.surface,
                        ),
                    )
                },
                snackbarHost = { SnackbarHost(snackbarHostState) },
            ) { padding ->
                Column(
                    Modifier
                        .fillMaxSize()
                        .padding(padding),
                ) {
                    if (state.loading || state.busy != null) {
                        LinearProgressIndicator(Modifier.fillMaxWidth())
                    }
                    LazyColumn(
                        modifier = Modifier.fillMaxSize(),
                        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 32.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        items(state.cards, key = { it.profile.id }) { card ->
                            ProfileRow(
                                card = card,
                                active = state.status?.active == card.profile.id,
                                busy = state.busy == card.profile.id,
                                enabled = state.canAct,
                                onApply = { vm.apply(card.profile.id) },
                                onDetail = { detailProfileId = card.profile.id },
                            )
                        }
                        item(key = "apps") {
                            AppsRow(
                                mappedCount = state.mappedCount,
                                onClick = { dest = HomeDest.APPS },
                            )
                        }
                        item(key = "service") {
                            ServiceRow(
                                state = state,
                                onToggle = vm::onServiceToggle,
                            )
                        }
                    }
                }
            }
        }
    }

    state.report?.let { report ->
        ReportDialog(report = report, onDismiss = vm::dismissReport)
    }
    state.lockedDetail?.let { detail ->
        LockedDialog(detail = detail, onDismiss = vm::dismissLocked)
    }
    detailProfileId?.let { id ->
        state.cards.firstOrNull { it.profile.id == id }?.let { card ->
            ProfileDetailDialog(card = card, onDismiss = { detailProfileId = null })
        } ?: run { detailProfileId = null }
    }
    if (state.confirmServiceOff) {
        AlertDialog(
            onDismissRequest = vm::cancelServiceOff,
            title = { Text("Turn off service?") },
            text = {
                Text(
                    "All tuned values will be written back to stock and the " +
                        "service will stop. Nothing will be modified until you " +
                        "turn it on again."
                )
            },
            confirmButton = {
                Button(onClick = vm::confirmServiceOff) { Text("Turn off") }
            },
            dismissButton = {
                TextButton(onClick = vm::cancelServiceOff) { Text("Cancel") }
            },
        )
    }
}

// --- profile row -------------------------------------------------------------

private fun iconFor(id: String): ImageVector = when (id) {
    "powersave" -> Icons.Default.BatterySaver
    "balance" -> Icons.Default.Balance
    else -> Icons.Default.Sports
}

@Composable
private fun ProfileRow(
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
private fun AppsRow(mappedCount: Int, onClick: () -> Unit) {
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
private fun ServiceRow(state: HomeUiState, onToggle: (Boolean) -> Unit) {
    val active = state.status?.active
    val statusText = when {
        state.automationRunning && active != null -> {
            val reason = state.automationReason
            val suffix = if (reason.isNullOrEmpty() || reason == "base") null else reason
            "Active · ${ProfileLabels.of(active)}" + (suffix?.let { " · $it" } ?: "")
        }
        state.automationRunning -> "On · stock"
        state.automationEnabled -> "Starting…"
        else -> "Off · stock"
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
                Text("Service", style = MaterialTheme.typography.titleMedium)
                Text(
                    statusText,
                    style = MaterialTheme.typography.labelSmall,
                    color = if (state.automationRunning) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(checked = state.automationEnabled, onCheckedChange = onToggle)
        }
    }
}

// --- dialogs -----------------------------------------------------------------

@Composable
private fun ReportDialog(report: ApplyReport, onDismiss: () -> Unit) {
    val title = when (report.mode) {
        "restore" -> "Restore ${if (report.ok) "complete" else "with errors"}"
        else -> "Apply ${report.profile ?: ""} ${if (report.ok) "complete" else "with errors"}"
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(
                Modifier
                    .fillMaxWidth()
                    .height(320.dp)
                    .verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Text(
                    "wrote ${report.wrote} · verified ${report.verified} · " +
                        "unchanged ${report.unchanged} · failed ${report.failed} · " +
                        "locked ${report.locked.size}",
                    style = MaterialTheme.typography.labelMedium,
                )
                if (report.snapshotCreated) {
                    Text(
                        "Stock snapshot created before first write.",
                        style = MaterialTheme.typography.labelSmall,
                    )
                }
                report.results.forEach { r -> ResultLine(r) }
                report.locked.forEach { l ->
                    Text(
                        "LOCKED ${l.key}\n    ${l.reason}",
                        style = MaterialTheme.typography.labelSmall,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.secondary,
                    )
                }
                if (report.results.isEmpty() && report.locked.isEmpty()) {
                    Text(
                        "Everything already in sync — nothing to write.",
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Close") }
        },
    )
}

@Composable
private fun ProfileDetailDialog(card: ProfileCard, onDismiss: () -> Unit) {
    val plan = card.plan
    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Column {
                Text("${card.profile.label} — detail")
                Text(
                    card.profile.desc,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        text = {
            Column(
                Modifier
                    .fillMaxWidth()
                    .height(400.dp)
                    .verticalScroll(rememberScrollState()),
            ) {
                if (plan == null) {
                    Text(
                        card.planError ?: "Plan unavailable",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                } else if (!plan.ok) {
                    plan.errors.forEach {
                        Text(it, style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.error)
                    }
                } else {
                    Text(
                        "${plan.ops.size} params · ${plan.pending} to write · " +
                            "${plan.inSync} in sync · ${plan.locked.size} locked",
                        style = MaterialTheme.typography.labelMedium,
                        modifier = Modifier.padding(bottom = 8.dp),
                    )
                    plan.ops.forEach { op ->
                        OpDetailRow(op)
                        HorizontalDivider(Modifier.padding(vertical = 2.dp))
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Close") }
        },
    )
}

@Composable
private fun OpDetailRow(op: Op) {
    Column(Modifier.padding(vertical = 6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                op.key,
                style = MaterialTheme.typography.labelMedium,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.weight(1f),
            )
            TierBadge(op.tier)
        }
        when (op.status.kind) {
            "locked" -> Text(
                "⚠ ${op.status.reason ?: "locked"}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.secondary,
            )
            "unchanged" -> Text(
                "✓ in sync: ${op.resolved}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.primary,
            )
            else -> Text(
                "→ ${op.resolved}" + (op.current?.let { "   (now: $it)" } ?: ""),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun TierBadge(tier: String) {
    val container = when (tier) {
        "free" -> MaterialTheme.colorScheme.tertiaryContainer
        else -> MaterialTheme.colorScheme.secondaryContainer
    }
    val onContainer = when (tier) {
        "free" -> MaterialTheme.colorScheme.onTertiaryContainer
        else -> MaterialTheme.colorScheme.onSecondaryContainer
    }
    Surface(color = container, shape = MaterialTheme.shapes.small) {
        Text(
            tier,
            Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelSmall,
            color = onContainer,
        )
    }
}

@Composable
private fun ResultLine(r: com.mifinetune.core.WriteResult) {
    val color = when {
        r.error != null -> MaterialTheme.colorScheme.error
        r.verified -> MaterialTheme.colorScheme.primary
        else -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    val mark = when {
        r.error != null -> "✗"
        r.verified -> "✓"
        else -> "·"
    }
    Text(
        "$mark ${r.key} → ${r.resolved}" + (r.error?.let { "\n    $it" } ?: ""),
        style = MaterialTheme.typography.labelSmall,
        fontFamily = FontFamily.Monospace,
        color = color,
    )
}

@Composable
private fun LockedDialog(detail: LockedDetail, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(detail.title) },
        text = {
            Column(
                Modifier
                    .fillMaxWidth()
                    .height(300.dp)
                    .verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                detail.items.forEach { item ->
                    Column {
                        Text(
                            item.key,
                            style = MaterialTheme.typography.labelMedium,
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.SemiBold,
                        )
                        Text(
                            item.reason,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Close") }
        },
    )
}
