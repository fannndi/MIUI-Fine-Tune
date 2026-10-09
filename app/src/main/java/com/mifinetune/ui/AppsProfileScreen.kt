package com.mifinetune.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.mifinetune.dynamic.AppProfileEntry

/**
 * Apps Profile: search + filter, tap an app, open its detail screen (Device
 * Profile mapping + the software layer). Nothing else.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppsProfileScreen(vm: DynamicProfileViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val appMap by vm.appMap.collectAsStateWithLifecycle()
    val profiles by vm.appProfiles.collectAsStateWithLifecycle()
    var selected by remember { mutableStateOf<AppEntry?>(null) }
    var filter by remember { mutableStateOf(AppFilter.ALL) }

    selected?.let { app ->
        // the detail edits the extended entry; a legacy app_map mapping is
        // shown (and carried over on the first save) instead of "Default"
        val merged = profiles[app.pkg] ?: appMap[app.pkg]?.let { AppProfileEntry(profile = it) }
        AppProfileDetailScreen(
            app = app,
            entry = merged,
            onSave = { vm.setAppEntry(app.pkg, it) },
            onBack = { selected = null },
        )
        return
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Apps Profile") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            OutlinedTextField(
                value = state.query,
                onValueChange = vm::setQuery,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                singleLine = true,
                placeholder = { Text("Search apps…") },
                leadingIcon = { Icon(Icons.Default.Search, contentDescription = null) },
            )
            Row(
                Modifier.padding(horizontal = 16.dp, vertical = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                AppFilter.entries.forEach { f ->
                    FilterChip(
                        selected = filter == f,
                        onClick = { filter = f },
                        label = { Text(f.label) },
                        modifier = Modifier.padding(end = 8.dp),
                    )
                }
            }
            if (state.loading) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(Modifier.size(28.dp), strokeWidth = 3.dp)
                }
            } else {
                val filtered = vm.filteredApps().filter { app ->
                    when (filter) {
                        AppFilter.ALL -> true
                        AppFilter.GAMES -> app.isGame
                        AppFilter.CONFIGURED -> profiles.containsKey(app.pkg)
                    }
                }
                val suggestions = if (state.query.isBlank() && filter != AppFilter.CONFIGURED) {
                    state.apps.filter { it.isGame && !profiles.containsKey(it.pkg) }.take(5)
                } else {
                    emptyList()
                }
                LazyColumn(
                    Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp),
                ) {
                    if (suggestions.isNotEmpty()) {
                        item(key = "suggestions") {
                            SuggestedGames(suggestions) { app ->
                                vm.setAppProfile(app.pkg, "game")
                            }
                        }
                    }
                    items(filtered, key = { it.pkg }) { app ->
                        AppRow(
                            app = app,
                            mapped = appMap[app.pkg],
                            configured = profiles[app.pkg],
                            onClick = { selected = app },
                        )
                    }
                }
            }
        }
    }
}

private enum class AppFilter(val label: String) {
    ALL("All"),
    GAMES("Games"),
    CONFIGURED("Configured"),
}

@Composable
private fun SuggestedGames(games: List<AppEntry>, onMap: (AppEntry) -> Unit) {
    androidx.compose.material3.ElevatedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(14.dp)) {
            Text(
                "Suggested games",
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                "Game-category apps with no mapping yet",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            games.forEach { app ->
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(top = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        app.label,
                        Modifier.weight(1f),
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 1,
                    )
                    androidx.compose.material3.TextButton(onClick = { onMap(app) }) {
                        Text("Set Game")
                    }
                }
            }
        }
    }
}

@Composable
private fun AppRow(
    app: AppEntry,
    mapped: String?,
    configured: AppProfileEntry?,
    onClick: () -> Unit,
) {
    val extras = buildList {
        if (configured?.bypassCharge == true) add("Bypass")
        if (configured?.dnd != null) add("DND")
        configured?.refreshHz?.let { add("$it Hz") }
    }
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .clickable(onClick = onClick)
            .padding(vertical = 8.dp, horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (app.icon != null) {
            Image(
                bitmap = app.icon,
                contentDescription = null,
                modifier = Modifier
                    .size(40.dp)
                    .clip(RoundedCornerShape(8.dp)),
            )
        } else {
            Box(Modifier.size(40.dp))
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    app.label,
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.Medium,
                    maxLines = 1,
                )
                if (app.isGame) {
                    Spacer(Modifier.width(6.dp))
                    Surface(
                        color = MaterialTheme.colorScheme.tertiaryContainer,
                        shape = MaterialTheme.shapes.small,
                    ) {
                        Text(
                            "GAME",
                            Modifier.padding(horizontal = 5.dp, vertical = 1.dp),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onTertiaryContainer,
                        )
                    }
                }
            }
            if (extras.isNotEmpty()) {
                Text(
                    extras.joinToString(" · "),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        Spacer(Modifier.width(8.dp))
        Surface(
            color = if (mapped != null) MaterialTheme.colorScheme.primaryContainer
            else MaterialTheme.colorScheme.surfaceVariant,
            shape = MaterialTheme.shapes.small,
        ) {
            Text(
                ProfileLabels.of(mapped),
                Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
                style = MaterialTheme.typography.labelMedium,
                color = if (mapped != null) MaterialTheme.colorScheme.onPrimaryContainer
                else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
