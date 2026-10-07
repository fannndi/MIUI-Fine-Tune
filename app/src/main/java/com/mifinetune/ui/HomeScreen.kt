package com.mifinetune.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AutoMode
import androidx.compose.material.icons.filled.Balance
import androidx.compose.material.icons.filled.BatterySaver
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Memory
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Sports
import androidx.compose.material3.AlertDialog
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
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.mifinetune.core.ApplyReport
import com.mifinetune.core.LockedKey
import com.mifinetune.core.Op
import com.mifinetune.core.Status

/**
 * Home: active-profile grid (tap = apply / temporary override), the automation
 * card (master switch + default + entries), and small diagnostics.
 * Simple by design — settings live in the dedicated sub-screens.
 */

private enum class HomeDest { HOME, APPS, SLEEP, SETUP }

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

    BackHandler(enabled = dest != HomeDest.HOME) { dest = HomeDest.HOME }

    when (dest) {
        HomeDest.APPS -> {
            val avm: AutomationViewModel = viewModel()
            AppsProfileScreen(vm = avm, onBack = { dest = HomeDest.HOME })
        }
        HomeDest.SLEEP -> {
            val avm: AutomationViewModel = viewModel()
            SleepScreen(vm = avm, onBack = { dest = HomeDest.HOME })
        }
        HomeDest.SETUP -> {
            val avm: AutomationViewModel = viewModel()
            SetupScreen(vm = avm, onBack = { dest = HomeDest.HOME })
        }
        HomeDest.HOME -> {
            Scaffold(
                topBar = {
                    TopAppBar(
                        title = {
                            Column {
                                Text("MiFineTune", style = MaterialTheme.typography.titleLarge)
                                Text(
                                    "MIUI-harmonized profiles",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        },
                        actions = {
                            IconButton(onClick = vm::refresh, enabled = state.canAct) {
                                Icon(Icons.Default.Refresh, contentDescription = "Refresh")
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
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        state.status?.let { st ->
                            item(key = "status") { StatusStrip(st, state) }
                        }
                        item(key = "profiles-header") {
                            SectionHeader(
                                "Profile aktif",
                                trailing = state.status?.active ?: "stock",
                            )
                        }
                        item(key = "grid") {
                            ProfileGrid(
                                state = state,
                                onApply = vm::apply,
                                onDetail = { detailProfileId = it },
                                onRestore = vm::askRestore,
                            )
                        }
                        item(key = "automation") {
                            AutomationCard(
                                state = state,
                                onToggle = vm::setAutomationEnabled,
                                onDefault = vm::setDefaultProfile,
                                onNavApps = { dest = HomeDest.APPS },
                                onNavSleep = { dest = HomeDest.SLEEP },
                                onNavSetup = { dest = HomeDest.SETUP },
                            )
                        }
                        item(key = "footnote") { FootNote(state) }
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
    if (state.confirmRestore) {
        AlertDialog(
            onDismissRequest = vm::cancelRestore,
            title = { Text("Kembalikan ke stock?") },
            text = {
                Text(
                    "Tulis balik semua nilai snapshot (kondisi sebelum apply pertama) " +
                        "dan matikan automasi. Profile aktif dihapus."
                )
            },
            confirmButton = {
                Button(onClick = vm::restore) { Text("Restore") }
            },
            dismissButton = {
                TextButton(onClick = vm::cancelRestore) { Text("Batal") }
            },
        )
    }
}

// --- status strip ------------------------------------------------------------

@Composable
private fun StatusStrip(st: Status, state: HomeUiState) {
    ElevatedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconCircle(Icons.Default.Memory)
                Spacer(Modifier.width(12.dp))
                Column {
                    Text(st.device.model, style = MaterialTheme.typography.titleMedium)
                    Text(
                        "MIUI ${st.device.rom} · ${st.device.kernel}",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SuggestionPill(
                    st.active ?: "stock",
                    if (st.active != null) MaterialTheme.colorScheme.primaryContainer
                    else MaterialTheme.colorScheme.surfaceVariant,
                    if (st.active != null) MaterialTheme.colorScheme.onPrimaryContainer
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (state.automationRunning) {
                    SuggestionPill(
                        "auto" + (state.automationReason?.let { " · $it" } ?: ""),
                        MaterialTheme.colorScheme.tertiaryContainer,
                        MaterialTheme.colorScheme.onTertiaryContainer,
                    )
                }
            }
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    FrameworkDot("thermal", st.framework.miThermald)
                    FrameworkDot("perf", st.framework.perfHal)
                    FrameworkDot("root", if (st.root) "granted" else null)
                }
                Text(
                    "${st.catalog.total} node · ${st.catalog.free} free · ${st.catalog.present} present",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (st.active != null && state.guardActive) {
                Text(
                    if (state.driftFixed > 0) "Drift guard: watching · corrected ${state.driftFixed}"
                    else "Drift guard: watching (verify tiap 15 dtk)",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.primary,
                )
            }
            if (state.packRom != null && state.packRom != st.device.rom) {
                Text(
                    "⚠ Profile pack diaudit untuk ${state.packRom}, device jalan ${st.device.rom} — " +
                        "jalankan tools/owner-map-audit.sh untuk kepastian.",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.secondary,
                )
            }
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

@Composable
private fun SectionHeader(title: String, trailing: String) {
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            title,
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
        )
        Text(
            trailing,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

// --- profile grid ------------------------------------------------------------

@Composable
private fun ProfileGrid(
    state: HomeUiState,
    onApply: (String) -> Unit,
    onDetail: (String) -> Unit,
    onRestore: () -> Unit,
) {
    val byId = state.cards.associateBy { it.profile.id }
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            CellFor("powersave", byId, state, onApply, onDetail)
            CellFor("balance", byId, state, onApply, onDetail)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            CellFor("game", byId, state, onApply, onDetail)
            StockCell(state, onRestore)
        }
    }
}

@Composable
private fun RowScope.CellFor(
    id: String,
    byId: Map<String, ProfileCard>,
    state: HomeUiState,
    onApply: (String) -> Unit,
    onDetail: (String) -> Unit,
) {
    val card = byId[id] ?: return
    val plan = card.plan
    val planLine = when {
        plan == null -> card.planError ?: "—"
        !plan.ok -> "ditolak"
        plan.pending == 0 -> "✓ in sync"
        else -> "${plan.pending} perubahan"
    }
    ProfileCell(
        modifier = Modifier.weight(1f),
        icon = when (id) {
            "powersave" -> Icons.Default.BatterySaver
            "balance" -> Icons.Default.Balance
            else -> Icons.Default.Sports
        },
        title = card.profile.label,
        desc = card.profile.desc,
        planLine = planLine,
        active = state.status?.active == id,
        isDefault = state.automationEnabled && state.defaultProfile == id,
        busy = state.busy == id,
        enabled = state.canAct && plan?.ok == true,
        onApply = { onApply(id) },
        onDetail = { onDetail(id) },
    )
}

@Composable
private fun RowScope.StockCell(state: HomeUiState, onRestore: () -> Unit) {
    val snapshot = state.status?.snapshot
    ProfileCell(
        modifier = Modifier.weight(1f),
        icon = Icons.Default.History,
        title = "Stock",
        desc = "Kembalikan nilai asli ROM",
        planLine = if (snapshot != null) "${snapshot.keys} nilai tersimpan" else "belum ada snapshot",
        active = false,
        isDefault = false,
        busy = state.busy == "restore",
        enabled = state.canAct && snapshot != null,
        onApply = onRestore,
        onDetail = null,
    )
}

@Composable
private fun ProfileCell(
    modifier: Modifier,
    icon: ImageVector,
    title: String,
    desc: String,
    planLine: String,
    active: Boolean,
    isDefault: Boolean,
    busy: Boolean,
    enabled: Boolean,
    onApply: () -> Unit,
    onDetail: (() -> Unit)?,
) {
    Card(
        modifier = modifier
            .height(150.dp)
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
        Column(
            Modifier
                .padding(12.dp)
                .fillMaxSize(),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Surface(
                    modifier = Modifier.size(32.dp),
                    shape = CircleShape,
                    color = if (active) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.primaryContainer,
                ) {
                    Icon(
                        icon,
                        contentDescription = null,
                        modifier = Modifier.padding(7.dp),
                        tint = if (active) MaterialTheme.colorScheme.onPrimary
                        else MaterialTheme.colorScheme.onPrimaryContainer,
                    )
                }
                Spacer(Modifier.width(8.dp))
                Text(
                    title,
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.weight(1f),
                    maxLines = 1,
                )
                if (busy) {
                    CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                } else if (onDetail != null) {
                    IconButton(
                        onClick = onDetail,
                        modifier = Modifier.size(26.dp),
                    ) {
                        Icon(
                            Icons.Default.Info,
                            contentDescription = "Detail",
                            modifier = Modifier.size(16.dp),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            Text(
                desc,
                style = MaterialTheme.typography.labelSmall,
                color = if (active) MaterialTheme.colorScheme.onPrimaryContainer
                else MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                minLines = 2,
            )
            Spacer(Modifier.weight(1f))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    planLine,
                    style = MaterialTheme.typography.labelSmall,
                    color = if (active) MaterialTheme.colorScheme.onPrimaryContainer
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(1f),
                    maxLines = 1,
                )
                if (active) {
                    SuggestionPill(
                        "AKTIF",
                        MaterialTheme.colorScheme.primary,
                        MaterialTheme.colorScheme.onPrimary,
                    )
                }
                if (isDefault) {
                    Spacer(Modifier.width(4.dp))
                    SuggestionPill(
                        "DEFAULT",
                        MaterialTheme.colorScheme.tertiaryContainer,
                        MaterialTheme.colorScheme.onTertiaryContainer,
                    )
                }
            }
        }
    }
}

// --- automation card ---------------------------------------------------------

@Composable
private fun AutomationCard(
    state: HomeUiState,
    onToggle: (Boolean) -> Unit,
    onDefault: (String) -> Unit,
    onNavApps: () -> Unit,
    onNavSleep: () -> Unit,
    onNavSetup: () -> Unit,
) {
    ElevatedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconCircle(Icons.Default.AutoMode)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text("Automasi", style = MaterialTheme.typography.titleMedium)
                    Text(
                        when {
                            state.automationRunning ->
                                "Aktif" + (state.automationReason?.let { " · $it" } ?: "")
                            state.automationEnabled -> "Menunggu service…"
                            else -> "Nonaktif"
                        },
                        style = MaterialTheme.typography.labelMedium,
                        color = if (state.automationRunning) MaterialTheme.colorScheme.primary
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(checked = state.automationEnabled, onCheckedChange = onToggle)
            }
            Text(
                "Layar mati → Sleep · app terpetakan → profile-nya · lainnya → Default harian.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (state.automationEnabled) {
                Text(
                    "Default harian",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                ProfileChips(
                    ids = listOf("powersave", "balance", "game"),
                    selected = state.defaultProfile,
                    onPick = onDefault,
                )
            }
            HorizontalDivider(Modifier.padding(vertical = 4.dp))
            NavRow(
                "Apps Profile",
                if (state.mappedCount > 0) "${state.mappedCount} app dipetakan" else "Belum ada app dipetakan",
                onNavApps,
            )
            NavRow(
                "Sleep (layar mati)",
                if (state.sleepEnabled) "aktif ±10 dtk setelah layar mati" else "nonaktif",
                onNavSleep,
            )
            NavRow("Izin & setup", "Root · akses penggunaan · autostart MIUI", onNavSetup)
        }
    }
}

@Composable
private fun FootNote(state: HomeUiState) {
    Text(
        "Tap kartu = apply langsung (override sementara saat automasi aktif). " +
            "Framework runtime nodes (thermal, perf locks, game cpuset, LMK, zRAM, charge) " +
            "tidak pernah ditulis oleh engine.",
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
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
                        "Semua sudah in sync — tidak ada yang ditulis.",
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Tutup") }
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
            TextButton(onClick = onDismiss) { Text("Tutup") }
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
            TextButton(onClick = onDismiss) { Text("Tutup") }
        },
    )
}
