package com.mifinetune.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.mifinetune.dynamic.DaemonLink
import com.mifinetune.dynamic.DiagInfo
import com.mifinetune.dynamic.DynamicProfileState
import com.mifinetune.dynamic.EnvSnapshot
import com.mifinetune.dynamic.StatEntry
import org.json.JSONObject
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Diagnostics: live daemon health, env telemetry, transition history and the
 * relayed daemon log — everything the user needs to verify behaviour without
 * a cable. All data comes from the Rust daemon (thin-client rule).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DiagnosticsScreen(onBack: () -> Unit) {
    val running by DynamicProfileState.running.collectAsStateWithLifecycle()
    val diag by DynamicProfileState.diag.collectAsStateWithLifecycle()
    val env by DynamicProfileState.env.collectAsStateWithLifecycle()
    val stats by DynamicProfileState.stats.collectAsStateWithLifecycle()
    val logs by DynamicProfileState.logs.collectAsStateWithLifecycle()

    // fresh data on open (the service seeds diag/stats once at connect)
    LaunchedEffect(Unit) {
        DaemonLink.send(JSONObject().put("cmd", "diag"))
        DaemonLink.send(JSONObject().put("cmd", "stats"))
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Diagnostics") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    IconButton(onClick = {
                        DaemonLink.send(JSONObject().put("cmd", "diag"))
                        DaemonLink.send(JSONObject().put("cmd", "stats"))
                    }) {
                        Icon(Icons.Default.Refresh, contentDescription = "Refresh")
                    }
                },
            )
        },
    ) { padding ->
        LazyColumn(
            Modifier
                .fillMaxSize()
                .padding(padding),
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 32.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (!running) {
                item(key = "off") {
                    SectionCard("Service is off") {
                        Text(
                            "Turn on the Service to see live daemon data.",
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                }
            }
            item(key = "daemon") { DaemonCard(diag) }
            item(key = "env") { EnvCard(env) }
            item(key = "stats") { TransitionsCard(stats) }
            item(key = "log") { LogCard(logs) }
        }
    }
}

// --- cards -------------------------------------------------------------------

@Composable
private fun SectionCard(title: String, content: @Composable () -> Unit) {
    ElevatedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(14.dp)) {
            Text(
                title,
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
            )
            Spacer(Modifier.width(8.dp))
            content()
        }
    }
}

@Composable
private fun DaemonCard(diag: DiagInfo?) {
    SectionCard("Daemon") {
        if (diag == null) {
            Text("Waiting for a `diag` reply…", style = MaterialTheme.typography.bodySmall)
            return@SectionCard
        }
        KV("PID", "${diag.pid} · up ${fmtUptime(diag.uptimeS)}")
        KV("Active", (diag.active ?: "stock") + (diag.reason?.let { " · $it" } ?: ""))
        KV("Foreground", diag.foreground ?: "—")
        val screen = when {
            !diag.screenOn -> "off"
            diag.locked -> "on · locked"
            else -> "on · unlocked"
        }
        KV("Screen", screen + if (diag.multiWindow) " · multi-window" else "")
        KV(
            "Watchers",
            "foreground ${alive(diag.watcherFg)} · multi-window ${alive(diag.watcherMw)}",
        )
        KV(
            "MIUI holds",
            "perf ${if (diag.perfHeld) "held" else "—"} · saver ${if (diag.saverHeld) "held" else "—"}",
        )
        KV(
            "Config",
            "service ${if (diag.configEnabled) "on" else "off"} · " +
                "dynamic ${if (diag.configDynamic) "on" else "off"} · base ${diag.baseProfile}",
        )
    }
}

@Composable
private fun EnvCard(env: EnvSnapshot?) {
    SectionCard("Environment") {
        if (env == null) {
            Text("Waiting for the first sample (30 s cadence)…", style = MaterialTheme.typography.bodySmall)
            return@SectionCard
        }
        KV(
            "Battery",
            (env.batteryPct?.let { "$it%" } ?: "—") +
                (env.charging?.let { if (it) " · charging" else " · discharging" } ?: ""),
        )
        KV("Battery temp", fmtTemp(env.batteryTempC))
        KV("CPU / GPU temp", "${fmtTemp(env.cpuTempC)} · ${fmtTemp(env.gpuTempC)}")
        KV("GPU busy", env.gpuBusyPct?.let { "$it%" } ?: "—")
    }
}

@Composable
private fun TransitionsCard(stats: List<StatEntry>) {
    SectionCard("Transitions (${stats.size})") {
        if (stats.isEmpty()) {
            Text("No switches recorded yet.", style = MaterialTheme.typography.bodySmall)
            return@SectionCard
        }
        // newest first, bounded to keep the screen light
        stats.asReversed().take(20).forEachIndexed { i, e ->
            if (i > 0) HorizontalDivider(Modifier.padding(vertical = 6.dp))
            val from = e.from ?: "stock"
            Text(
                "${fmtTime(e.t)}   $from → ${e.to}  (${e.reason})",
                style = MaterialTheme.typography.bodySmall,
                fontWeight = FontWeight.Medium,
            )
            val meta = listOfNotNull(
                e.batteryPct?.let { "$it%" },
                e.tempC?.let { fmtTemp(it) },
            ).joinToString(" · ")
            if (meta.isNotEmpty()) {
                Text(
                    meta,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        if (stats.size > 20) {
            Text(
                "… ${stats.size - 20} older entries",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun LogCard(logs: List<String>) {
    SectionCard("Daemon log (${logs.size})") {
        if (logs.isEmpty()) {
            Text("No log lines yet.", style = MaterialTheme.typography.bodySmall)
            return@SectionCard
        }
        Column(
            Modifier
                .fillMaxWidth()
                .heightIn(max = 320.dp),
        ) {
            logs.takeLast(60).forEach { line ->
                Text(
                    line,
                    style = MaterialTheme.typography.labelSmall,
                    fontFamily = FontFamily.Monospace,
                )
            }
        }
    }
}

// --- bits --------------------------------------------------------------------

@Composable
private fun KV(label: String, value: String) {
    Row(Modifier.fillMaxWidth().padding(vertical = 2.dp)) {
        Text(
            label,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.width(110.dp),
        )
        Text(value, style = MaterialTheme.typography.bodySmall)
    }
}

private fun alive(ok: Boolean) = if (ok) "alive" else "down"

private fun fmtTemp(t: Float?): String =
    t?.let { String.format(Locale.US, "%.1f °C", it) } ?: "—"

private fun fmtUptime(s: Long): String = when {
    s < 60 -> "${s}s"
    s < 3600 -> "${s / 60}m ${s % 60}s"
    else -> "${s / 3600}h ${(s % 3600) / 60}m"
}

private fun fmtTime(t: Long): String =
    SimpleDateFormat("HH:mm", Locale.US).format(Date(t * 1000))
