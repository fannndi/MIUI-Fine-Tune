package com.mifinetune.automation

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Best-effort re-apply after reboot: MIUI may block boot receivers unless the
 * app is allowed in Autostart — then this simply never fires.
 */
class AutomationBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        val config = AutomationConfig.get(context)
        if (config.enabled && config.bootApply) {
            AutomationService.start(context)
        }
    }
}
