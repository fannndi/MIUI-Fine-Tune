package com.mifinetune.ui

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
import androidx.compose.material.icons.filled.Balance
import androidx.compose.material.icons.filled.BatterySaver
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Memory
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Speed
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
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.mifinetune.core.ApplyReport
import com.mifinetune.core.LockedKey
import com.mifinetune.core.Status

/**
 * Single-screen Material 3 UI: status + three profile cards + dialogs.
 * No tuning logic here — everything goes through [HomeViewModel].
 */

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(vm: HomeViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }

    LaunchedEffect(state.error) {
        state.error?.let {
            snackbarHostState.showSnackbar(it)
            vm.clearError()
        }
    }

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
                    IconButton(
                        onClick = vm::askRestore,
                        enabled = state.canAct && state.status?.snapshot != null,
                    ) {
                        Icon(Icons.Default.History, contentDescription = "Restore stock")
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
                if (state.loading) {
                    item { StatusShimmer() }
                } else {
                    state.status?.let { st ->
                        item(key = "status") { StatusCard(st) }
                    }
                    item(key = "section") {
                        SectionHeader("Profiles", trailing = sectionSummary(state))
                    }
                    items(state.cards, key = { it.profile.id }) { card ->
                        ProfileCardView(
                            card = card,
                            active = state.status?.active == card.profile.id,
                            busy = state.busy == card.profile.id,
                            enabled = state.canAct,
                            onApply = { vm.apply(card.profile.id) },
                            onLockedClick = { title, items ->
                                vm.showLocked(title, items)
                            },
                        )
                    }
                    item(key = "footnote") { FootNote(state) }
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
    if (state.confirmRestore) {
        AlertDialog(
            onDismissRequest = vm::cancelRestore,
            title = { Text("Restore stock?") },
            text = {
                Text(
                    "Write every snapshotted parameter back to the values the ROM " +
                        "had before the first apply. The active profile is cleared."
                )
            },
            confirmButton = {
                Button(onClick = vm::restore) { Text("Restore") }
            },
            dismissButton = {
                TextButton(onClick = vm::cancelRestore) { Text("Cancel") }
            },
        )
    }
}

private fun sectionSummary(state: HomeUiState): String {
    val active = state.status?.active
    return if (active != null) "active: $active" else "stock"
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

@Composable
private fun StatusShimmer() {
    ElevatedCard(Modifier.fillMaxWidth()) {
        Row(
            Modifier.padding(16.dp),
            horizontalArrangement = Arrangement.Center,
        ) {
            CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
            Spacer(Modifier.width(12.dp))
            Text("Probing device…", style = MaterialTheme.typography.bodyMedium)
        }
    }
}

@Composable
private fun StatusCard(st: Status) {
    ElevatedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconCircle(Icons.Default.Memory)
                Spacer(Modifier.width(12.dp))
                Column {
                    Text(st.device.model, style = MaterialTheme.typography.titleMedium)
                    Text(
                        "MIUI ${st.device.rom} · SoC ${st.device.socId} · ${st.device.kernel}",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            HorizontalDivider()

            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Active profile", style = MaterialTheme.typography.bodyMedium)
                val active = st.active
                if (active == null) {
                    SuggestionPill("stock (restored)", MaterialTheme.colorScheme.surfaceVariant)
                } else {
                    SuggestionPill(active, MaterialTheme.colorScheme.primaryContainer,
                        MaterialTheme.colorScheme.onPrimaryContainer)
                }
            }

            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Framework (untouched)", style = MaterialTheme.typography.bodyMedium)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FrameworkDot("thermal", st.framework.miThermald)
                    FrameworkDot("perf", st.framework.perfHal)
                    FrameworkDot("root", if (st.root) "granted" else null)
                }
            }

            Text(
                "Catalog: ${st.catalog.total} nodes · ${st.catalog.free} free · " +
                    "${st.catalog.baseline} boot-baseline · ${st.catalog.present} present",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
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
        Text(label, style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun SuggestionPill(text: String, container: androidx.compose.ui.graphics.Color,
                           onContainer: androidx.compose.ui.graphics.Color =
                               MaterialTheme.colorScheme.onSurfaceVariant) {
    Surface(color = container, shape = MaterialTheme.shapes.small) {
        Text(
            text,
            Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
            style = MaterialTheme.typography.labelMedium,
            color = onContainer,
        )
    }
}

@Composable
private fun IconCircle(icon: ImageVector) {
    Surface(
        modifier = Modifier.size(40.dp),
        shape = CircleShape,
        color = MaterialTheme.colorScheme.primaryContainer,
    ) {
        Icon(
            icon,
            contentDescription = null,
            modifier = Modifier.padding(8.dp),
            tint = MaterialTheme.colorScheme.onPrimaryContainer,
        )
    }
}

private fun iconFor(id: String): ImageVector = when (id) {
    "powersave" -> Icons.Default.BatterySaver
    "balance" -> Icons.Default.Balance
    "game" -> Icons.Default.Sports
    else -> Icons.Default.Speed
}

@Composable
private fun ProfileCardView(
    card: ProfileCard,
    active: Boolean,
    busy: Boolean,
    enabled: Boolean,
    onApply: () -> Unit,
    onLockedClick: (String, List<LockedKey>) -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = if (active) CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.primaryContainer,
            contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
        ) else CardDefaults.cardColors(),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconCircle(iconFor(card.profile.id))
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
                        card.profile.desc,
                        style = MaterialTheme.typography.bodySmall,
                        color = if (active) MaterialTheme.colorScheme.onPrimaryContainer
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            val plan = card.plan
            when {
                plan == null -> Text(
                    card.planError ?: "Plan unavailable",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.error,
                )
                !plan.ok -> Text(
                    "Rejected: ${plan.errors.joinToString()}",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.error,
                )
                else -> Text(
                    "${plan.ops.size} params · ${plan.pending} to write · " +
                        "${plan.inSync} in sync" +
                        if (plan.locked.isNotEmpty()) " · ${plan.locked.size} locked" else "",
                    style = MaterialTheme.typography.labelSmall,
                    color = if (active) MaterialTheme.colorScheme.onPrimaryContainer
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            val locked = plan?.locked.orEmpty()
            if (locked.isNotEmpty()) {
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    locked.take(3).forEach { op ->
                        AssistChip(
                            onClick = {
                                onLockedClick(
                                    "${card.profile.label}: locked parameters",
                                    locked.map { LockedKey(it.key, it.status.reason ?: "locked") },
                                )
                            },
                            label = { Text(op.key, style = MaterialTheme.typography.labelSmall) },
                            leadingIcon = {
                                Icon(
                                    Icons.Default.Warning,
                                    contentDescription = null,
                                    Modifier.size(14.dp),
                                )
                            },
                            modifier = Modifier.height(28.dp),
                        )
                    }
                    if (locked.size > 3) {
                        AssistChip(
                            onClick = {
                                onLockedClick(
                                    "${card.profile.label}: locked parameters",
                                    locked.map { LockedKey(it.key, it.status.reason ?: "locked") },
                                )
                            },
                            label = { Text("+${locked.size - 3}", style = MaterialTheme.typography.labelSmall) },
                            modifier = Modifier.height(28.dp),
                        )
                    }
                }
            }

            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (busy) {
                    CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.width(10.dp))
                }
                val planOk = plan?.ok == true
                if (active) {
                    OutlinedButton(onClick = onApply, enabled = enabled && !busy && planOk) {
                        Text(if (busy) "Applying…" else "Re-apply")
                    }
                } else {
                    Button(onClick = onApply, enabled = enabled && !busy && planOk) {
                        Text(if (busy) "Applying…" else "Apply")
                    }
                }
            }
        }
    }
}

@Composable
private fun FootNote(state: HomeUiState) {
    val snapshot = state.status?.snapshot
    Text(
        if (snapshot != null)
            "Stock snapshot: ${snapshot.keys} keys captured — Restore returns exactly these values. " +
                "Framework runtime nodes (thermal, perf locks, game cpuset, LMK, zRAM, charge) are " +
                "rejected by the core validator and never written."
        else
            "No snapshot yet — applying a profile captures stock values first. " +
                "Framework runtime nodes (thermal, perf locks, game cpuset, LMK, zRAM, charge) are " +
                "rejected by the core validator and never written.",
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

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
                report.results.forEach { r ->
                    ResultLine(r)
                }
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
