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
 * Adaptive guards card (Settings): battery floor and thermal ceiling.
 * The daemon owns the decisions; these are its config knobs. Slider values
 * commit on release (config writes must not fire per drag pixel).
 */
@Composable
internal fun GuardsCard(
    guardBattery: Boolean,
    batteryFloor: Int,
    guardThermal: Boolean,
    thermalCeiling: Float,
    onGuardBattery: (Boolean) -> Unit,
    onBatteryFloor: (Int) -> Unit,
    onGuardThermal: (Boolean) -> Unit,
    onThermalCeiling: (Float) -> Unit,
) {
    var floor by remember { mutableFloatStateOf(batteryFloor.toFloat()) }
    LaunchedEffect(batteryFloor) { floor = batteryFloor.toFloat() }
    var ceiling by remember { mutableFloatStateOf(thermalCeiling) }
    LaunchedEffect(thermalCeiling) { ceiling = thermalCeiling }

    ElevatedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Adaptive guards", style = MaterialTheme.typography.titleMedium)

            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Battery guard", style = MaterialTheme.typography.bodyMedium)
                    Text(
                        "Below ${floor.roundToInt()}% and not charging → Power Save " +
                            "(mapped apps included)",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(checked = guardBattery, onCheckedChange = onGuardBattery)
            }
            if (guardBattery) {
                Slider(
                    value = floor,
                    onValueChange = { floor = it },
                    onValueChangeFinished = { onBatteryFloor(floor.roundToInt()) },
                    valueRange = 10f..40f,
                    steps = 5,
                )
            }

            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Thermal guard", style = MaterialTheme.typography.bodyMedium)
                    Text(
                        "Game steps down to Balance at ${ceiling.roundToInt()} °C " +
                            "(releases 5 °C lower; never fights the kernel)",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(checked = guardThermal, onCheckedChange = onGuardThermal)
            }
            if (guardThermal) {
                Slider(
                    value = ceiling,
                    onValueChange = { ceiling = it },
                    onValueChangeFinished = { onThermalCeiling(ceiling) },
                    valueRange = 65f..85f,
                    steps = 3,
                )
            }
        }
    }
}
