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

@Composable
internal fun ReportDialog(report: ApplyReport, onDismiss: () -> Unit) {
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
internal fun ProfileDetailDialog(card: ProfileCard, onDismiss: () -> Unit) {
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
internal fun LockedDialog(detail: LockedDetail, onDismiss: () -> Unit) {
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
