package com.mifinetune.dynamic

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Best-effort re-apply after reboot: MIUI may block boot receivers unless the
 * app is allowed in Autostart — then this simply never fires.
 */
class DynamicProfileBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        if (DynamicProfileConfig.get(context).enabled) {
            DynamicProfileService.start(context)
        }
    }
}
