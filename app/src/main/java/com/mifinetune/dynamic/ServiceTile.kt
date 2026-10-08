package com.mifinetune.dynamic

import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import com.mifinetune.ui.ProfileLabels

/**
 * Quick Settings tile: shows the service state and toggles it with the same
 * semantics as the UI switch — turning off runs the restore path first.
 *
 * Responsibility: a status/toggle surface. Non-goals: decisions.
 */
class ServiceTile : TileService() {

    companion object {
        /**
         * Ask the system to re-bind the tile so its subtitle updates
         * immediately after a profile switch (no-op when the tile is absent).
         */
        fun refresh(context: android.content.Context) {
            runCatching {
                requestListeningState(
                    context,
                    android.content.ComponentName(context, ServiceTile::class.java),
                )
            }
        }
    }

    override fun onStartListening() = refreshTile()

    override fun onClick() {
        if (DynamicProfileState.running.value) {
            // same path as the notification action: restore, then stop
            startService(
                Intent(this, DynamicProfileService::class.java)
                    .setAction(DynamicProfileService.ACTION_STOP),
            )
        } else {
            DynamicProfileConfig.get(this).enabled = true
            DynamicProfileService.start(this)
        }
        refreshTile()
    }

    private fun refreshTile() {
        val t = qsTile ?: return
        val running = DynamicProfileState.running.value
        t.state = if (running) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        t.label = "MiFineTune"
        if (Build.VERSION.SDK_INT >= 29) {
            val active = DynamicProfileState.appliedProfile.value
            t.subtitle = when {
                !running -> "Off"
                active != null -> ProfileLabels.of(active)
                else -> "On"
            }
        }
        t.updateTile()
    }
}
