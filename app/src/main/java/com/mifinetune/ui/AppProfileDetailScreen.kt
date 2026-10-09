package com.mifinetune.ui

import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.KeyboardArrowRight
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import com.mifinetune.dynamic.AppProfileEntry
import kotlinx.coroutines.launch

/**
 * Apps Profile detail (software layer): the Device-Profile mapping plus the
 * audited per-app software surfaces (bypass charging, DND).
 *
 * Edits are a local draft; the explicit Save button is the only writer (the
 * user asked for a visible confirmation). Leaving with unsaved changes asks
 * before discarding.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppProfileDetailScreen(
    app: AppEntry,
    entry: AppProfileEntry?,
    onSave: (AppProfileEntry?) -> Unit,
    onBack: () -> Unit,
) {
    val context = LocalContext.current
    val initial = entry ?: AppProfileEntry()
    var draft by remember(app.pkg) { mutableStateOf(initial) }
    val dirty = draft != initial
    var dndGranted by remember { mutableStateOf(isDndGranted(context)) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        dndGranted = isDndGranted(context)
    }

    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    var confirmDiscard by remember { mutableStateOf(false) }
    var picker by remember { mutableStateOf<String?>(null) }

    fun leave() {
        if (dirty) confirmDiscard = true else onBack()
    }

    BackHandler { leave() }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("App Profile") },
                navigationIcon = {
                    IconButton(onClick = { leave() }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
        ) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (app.icon != null) {
                    Image(
                        bitmap = app.icon,
                        contentDescription = null,
                        modifier = Modifier
                            .size(48.dp)
                            .clip(RoundedCornerShape(10.dp)),
                    )
                } else {
                    Spacer(Modifier.size(48.dp))
                }
                Spacer(Modifier.width(12.dp))
                Column {
                    Text(app.label, style = MaterialTheme.typography.titleMedium)
                    Text(
                        app.pkg,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            SectionLabel("Device Profile (hardware)")
            PickerRow(
                title = "Power profile",
                value = ProfileLabels.of(draft.profile),
                detail = "Hardware parameters used while this app is in front",
                onClick = { picker = "profile" },
            )

            SectionLabel("Software")
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text("Bypass charging", style = MaterialTheme.typography.bodyLarge)
                    Text(
                        "Charger input is suspended while this app runs " +
                            "(phone runs on battery, less heat); releases at the " +
                            "bypass floor or when the app leaves",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(
                    checked = draft.bypassCharge,
                    onCheckedChange = { draft = draft.copy(bypassCharge = it) },
                )
            }
            HorizontalDivider()
            PickerRow(
                title = "Do Not Disturb",
                value = dndLabel(draft.dnd),
                detail = "Through Android's official DND access (restored on exit)",
                onClick = { picker = "dnd" },
            )
            HorizontalDivider()
            PickerRow(
                title = "Refresh rate",
                value = refreshLabel(draft.refreshHz),
                detail = "While this app is in front — captures your MIUI value " +
                    "and restores it on exit; Default = MIUI keeps control",
                onClick = { picker = "refresh" },
            )
            if (draft.dnd != null && !dndGranted) {
                Surface(
                    color = MaterialTheme.colorScheme.errorContainer,
                    shape = RoundedCornerShape(12.dp),
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 10.dp),
                ) {
                    Column(Modifier.padding(12.dp)) {
                        Text(
                            "Do Not Disturb access is not granted",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onErrorContainer,
                        )
                        Text(
                            "Grant it once in system settings so MiFineTune can " +
                                "switch DND for this app.",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onErrorContainer,
                        )
                        OutlinedButton(
                            onClick = {
                                runCatching {
                                    context.startActivity(
                                        Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS)
                                            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                                    )
                                }
                            },
                            modifier = Modifier.padding(top = 6.dp),
                        ) {
                            Text("Grant access")
                        }
                    }
                }
            }

            if (dirty) {
                Text(
                    "Unsaved changes",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(top = 14.dp),
                )
            }
            Button(
                onClick = {
                    onSave(if (draft.isEmpty) null else draft)
                    scope.launch { snackbar.showSnackbar("App profile saved") }
                },
                enabled = dirty,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 8.dp),
            ) {
                Text("Save app profile")
            }
            TextButton(
                onClick = { draft = AppProfileEntry() },
                modifier = Modifier.padding(vertical = 8.dp),
            ) {
                Text("Reset app profile")
            }
            Spacer(Modifier.height(24.dp))
        }
    }

    picker?.let { which ->
        val (title, options) = when (which) {
            "profile" -> "Power profile" to listOf(
                null to "Default (follow base)",
                "powersave" to "Power Save",
                "balance" to "Balance",
                "game" to "Game",
            )
            "refresh" -> "Refresh rate" to listOf(
                null to "Default (MIUI)",
                "120" to "120 Hz",
                "90" to "90 Hz",
                "60" to "60 Hz",
                "30" to "30 Hz",
            )
            else -> "Do Not Disturb" to listOf(
                null to "Off",
                "priority" to "Priority only",
                "total" to "Total silence",
            )
        }
        ModalBottomSheet(onDismissRequest = { picker = null }) {
            Column(Modifier.padding(bottom = 24.dp)) {
                Text(
                    title,
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
                )
                val selected = when (which) {
                    "profile" -> draft.profile
                    "refresh" -> draft.refreshHz?.toString()
                    else -> draft.dnd
                }
                options.forEach { (value, label) ->
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .clickable {
                                picker = null
                                draft = when (which) {
                                    "profile" -> draft.copy(profile = value)
                                    "refresh" -> draft.copy(refreshHz = value?.toIntOrNull())
                                    else -> draft.copy(dnd = value)
                                }
                            }
                            .padding(horizontal = 20.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = value == selected, onClick = null)
                        Spacer(Modifier.width(8.dp))
                        Text(label, style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        }
    }

    if (confirmDiscard) {
        AlertDialog(
            onDismissRequest = { confirmDiscard = false },
            title = { Text("Discard changes?") },
            text = { Text("Your changes to this app profile are not saved yet.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmDiscard = false
                        onBack()
                    },
                ) {
                    Text("Discard")
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmDiscard = false }) {
                    Text("Keep editing")
                }
            },
        )
    }
}

@Composable
private fun SectionLabel(text: String) {
    Text(
        text.uppercase(),
        style = MaterialTheme.typography.labelMedium,
        fontWeight = FontWeight.SemiBold,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(top = 16.dp, bottom = 2.dp),
    )
}

@Composable
private fun PickerRow(
    title: String,
    value: String,
    detail: String,
    onClick: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(
                detail,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Text(
            value,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.primary,
        )
        Icon(
            Icons.Default.KeyboardArrowRight,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

private fun dndLabel(value: String?): String = when (value) {
    "priority" -> "Priority"
    "total" -> "Total silence"
    else -> "Off"
}

private fun refreshLabel(value: Int?): String = when (value) {
    120 -> "120 Hz"
    90 -> "90 Hz"
    60 -> "60 Hz"
    30 -> "30 Hz"
    else -> "Default"
}

internal fun isDndGranted(context: Context): Boolean = runCatching {
    (context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager)
        .isNotificationPolicyAccessGranted
}.getOrDefault(false)
