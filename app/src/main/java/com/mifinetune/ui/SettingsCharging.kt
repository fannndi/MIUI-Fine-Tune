package com.mifinetune.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

/**
 * Charging card (Settings): the charge limit and the per-app bypass floor.
 * The daemon owns the decisions; these are its config knobs. Slider values
 * commit on release (config writes must not fire per drag pixel).
 */
@Composable
internal fun ChargingCard(
    chargeLimit: Boolean,
    chargeLimitPct: Int,
    bypassFloor: Int,
    onChargeLimit: (Boolean) -> Unit,
    onChargeLimitPct: (Int) -> Unit,
    onBypassFloor: (Int) -> Unit,
) {
    var chargePct by remember { mutableFloatStateOf(chargeLimitPct.toFloat()) }
    LaunchedEffect(chargeLimitPct) { chargePct = chargeLimitPct.toFloat() }
    var floor by remember { mutableFloatStateOf(bypassFloor.toFloat()) }
    LaunchedEffect(bypassFloor) { floor = bypassFloor.toFloat() }

    ElevatedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Charging", style = MaterialTheme.typography.titleMedium)

            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Charge limit", style = MaterialTheme.typography.bodyMedium)
                    Text(
                        "Pause charging at ${chargePct.roundToInt()}% " +
                            "(resumes 5% lower; the stock switch returns on exit)",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(checked = chargeLimit, onCheckedChange = onChargeLimit)
            }
            if (chargeLimit) {
                Slider(
                    value = chargePct,
                    onValueChange = { chargePct = it },
                    onValueChangeFinished = { onChargeLimitPct(chargePct.roundToInt()) },
                    valueRange = 60f..95f,
                    steps = 6,
                )
            }

            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Bypass floor", style = MaterialTheme.typography.bodyMedium)
                    Text(
                        "Per-app bypass charging releases at ${floor.roundToInt()}% " +
                            "and re-engages 5% above (protects the battery while gaming)",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Slider(
                value = floor,
                onValueChange = { floor = it },
                onValueChangeFinished = { onBypassFloor(floor.roundToInt()) },
                valueRange = 15f..50f,
                steps = 6,
            )
        }
    }
}
