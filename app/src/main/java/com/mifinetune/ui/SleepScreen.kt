package com.mifinetune.ui

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
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Bedtime
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle

/** Sleep (screen off) — small dedicated page. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SleepScreen(vm: AutomationViewModel, onBack: () -> Unit) {
    val sleepEnabled by vm.sleepEnabled.collectAsStateWithLifecycle()
    val sleepProfile by vm.sleepProfile.collectAsStateWithLifecycle()
    val skipMusic by vm.skipOnMusic.collectAsStateWithLifecycle()
    val skipCharging by vm.skipOnCharging.collectAsStateWithLifecycle()

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Sleep (layar mati)") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Kembali")
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
            ElevatedCard(Modifier.fillMaxWidth()) {
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    IconCircle(Icons.Default.Bedtime)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        Text(
                            "Sleep saat layar mati",
                            style = MaterialTheme.typography.titleMedium,
                        )
                        Text(
                            "Aktif ±10 dtk setelah layar mati.",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Switch(checked = sleepEnabled, onCheckedChange = vm::setSleepEnabled)
                }
            }

            if (sleepEnabled) {
                ElevatedCard(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("Profile saat layar mati", style = MaterialTheme.typography.titleSmall)
                        ProfileChips(
                            ids = listOf("sleep", "powersave", "balance", "game"),
                            selected = sleepProfile,
                            onPick = vm::setSleepProfile,
                        )
                    }
                }
                ElevatedCard(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp)) {
                        ToggleRow("Lewati saat memutar musik", skipMusic, vm::setSkipOnMusic)
                        HorizontalDivider()
                        ToggleRow("Lewati saat charging", skipCharging, vm::setSkipOnCharging)
                    }
                }
            }

            Text(
                "Sleep hanya menyetel frekuensi, core_ctl, GPU, jadwal CPU, dan I/O. " +
                    "Tidak menyentuh jaringan, LMK, swap, atau cpuset — telpon dan " +
                    "notifikasi (WA, FCM) tetap masuk normal.",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.size(16.dp))
        }
    }
}
