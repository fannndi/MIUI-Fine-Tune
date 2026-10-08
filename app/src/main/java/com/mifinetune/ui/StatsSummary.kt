package com.mifinetune.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.mifinetune.dynamic.StatEntry

/**
 * Time-in-profile for the last 24 h, derived from the daemon's transition
 * history (each entry owns the span to the next one; the oldest spans are
 * clamped to the window). Display-only wall-clock math.
 */
internal fun timePerProfile(
    stats: List<StatEntry>,
    nowS: Long,
    windowS: Long = 24 * 3600,
): Map<String, Long> {
    if (stats.isEmpty()) return emptyMap()
    val from = nowS - windowS
    val out = mutableMapOf<String, Long>()
    for ((i, e) in stats.withIndex()) {
        val end = stats.getOrNull(i + 1)?.t ?: nowS
        val start = maxOf(e.t, from)
        val stop = minOf(end, nowS)
        if (stop > start) out[e.to] = (out[e.to] ?: 0L) + (stop - start)
    }
    return out
}

internal fun fmtDuration(secs: Long): String = when {
    secs >= 3600 -> "${secs / 3600}h ${(secs % 3600) / 60}m"
    secs >= 60 -> "${secs / 60}m"
    else -> "${secs}s"
}

@Composable
internal fun StatsSummaryCard(stats: List<StatEntry>) {
    val now = System.currentTimeMillis() / 1000
    val perProfile = timePerProfile(stats, now)

    ElevatedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(14.dp)) {
            Text(
                "Last 24 hours",
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
            )
            if (perProfile.isEmpty()) {
                Text("No history yet.", style = MaterialTheme.typography.bodySmall)
                return@Column
            }
            perProfile.entries.sortedByDescending { it.value }.forEach { (id, secs) ->
                Row(Modifier.fillMaxWidth().padding(vertical = 2.dp)) {
                    Text(
                        ProfileLabels.of(id),
                        Modifier.weight(1f),
                        style = MaterialTheme.typography.bodySmall,
                    )
                    Text(
                        fmtDuration(secs),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Text(
                "tracked ${fmtDuration(perProfile.values.sum())}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
