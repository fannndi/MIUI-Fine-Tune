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
import com.mifinetune.dynamic.DynamicProfileState

/**
 * Home: compact profile rows (tap = apply + base), the Apps Profile entry and
 * the Service switch. Everything else lives in Settings.
 */

private enum class HomeDest { HOME, APPS, SETTINGS, DIAGNOSTICS }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(vm: HomeViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val env by DynamicProfileState.env.collectAsStateWithLifecycle()
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
            val avm: DynamicProfileViewModel = viewModel()
            AppsProfileScreen(vm = avm, onBack = { dest = HomeDest.HOME })
        }
        HomeDest.SETTINGS -> {
            val avm: DynamicProfileViewModel = viewModel()
            SettingsScreen(
                status = state.status,
                onBack = { dest = HomeDest.HOME },
                vm = avm,
            )
        }
        HomeDest.DIAGNOSTICS -> {
            DiagnosticsScreen(onBack = { dest = HomeDest.HOME })
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
                        item(key = "dynamic") {
                            DynamicProfileRow(
                                state = state,
                                onToggle = vm::onDynamicToggle,
                            )
                        }
                        item(key = "diagnostics") {
                            DiagnosticsRow(
                                env = env,
                                onClick = { dest = HomeDest.DIAGNOSTICS },
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
