package com.mifinetune.ui

import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.mifinetune.dynamic.DynamicProfileConfig

/**
 * Backup card: export/import `config.json` through the system file picker
 * (no storage permission needed). Import is validated + clamped in
 * [DynamicProfileConfig.importJson] before anything is written.
 */
@Composable
internal fun BackupCard(config: DynamicProfileConfig) {
    val context = LocalContext.current
    val export = rememberLauncherForActivityResult(
        ActivityResultContracts.CreateDocument("application/json"),
    ) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        val ok = runCatching {
            context.contentResolver.openOutputStream(uri)?.use {
                it.write(config.exportJson().toByteArray())
            }
            true
        }.getOrDefault(false)
        Toast.makeText(
            context,
            if (ok) "Backup exported" else "Export failed",
            Toast.LENGTH_SHORT,
        ).show()
    }
    val import = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        val raw = runCatching {
            context.contentResolver.openInputStream(uri)?.bufferedReader()
                ?.use { r -> r.readText() }
        }.getOrNull()
        val ok = raw != null && config.importJson(raw)
        Toast.makeText(
            context,
            if (ok) "Backup imported" else "Not a MiFineTune backup",
            Toast.LENGTH_SHORT,
        ).show()
    }

    ElevatedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("Backup", style = MaterialTheme.typography.titleMedium)
            Text(
                "Export or restore settings and app mappings as JSON",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Row {
                TextButton(onClick = { export.launch("mifinetune-backup.json") }) {
                    Text("Export")
                }
                TextButton(onClick = { import.launch(arrayOf("application/json", "text/plain", "*/*")) }) {
                    Text("Import")
                }
            }
        }
    }
}
