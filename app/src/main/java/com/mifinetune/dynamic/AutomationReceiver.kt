package com.mifinetune.dynamic

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import org.json.JSONObject

/**
 * Automation API (Tasker / MacroDroid / `adb shell am broadcast`).
 *
 * Deliberately tiny and safe: it only routes the same actions the UI has —
 * change the base Device Profile, raise a manual boost window, and set the
 * charge limit. It never starts/stops the Service. All decisions stay in
 * the Rust daemon; this receiver only writes app-owned config + hints.
 *
 * Actions (explicit component recommended):
 *   am broadcast -a com.mifinetune.action.SET_PROFILE \
 *       -n com.mifinetune/.dynamic.AutomationReceiver --es profile game
 *   am broadcast -a com.mifinetune.action.BOOST \
 *       -n com.mifinetune/.dynamic.AutomationReceiver
 *   am broadcast -a com.mifinetune.action.SET_CHARGE_LIMIT \
 *       -n com.mifinetune/.dynamic.AutomationReceiver --ez enabled true --ei pct 80
 */
class AutomationReceiver : BroadcastReceiver() {

    companion object {
        private const val TAG = "MiFineTune"
        const val ACTION_SET_PROFILE = "com.mifinetune.action.SET_PROFILE"
        const val ACTION_BOOST = "com.mifinetune.action.BOOST"
        const val ACTION_SET_CHARGE_LIMIT = "com.mifinetune.action.SET_CHARGE_LIMIT"

        /** The base-card profiles the UI exposes (sleep/boost stay automatic). */
        private val PROFILES = setOf("powersave", "balance", "game")
    }

    override fun onReceive(context: Context, intent: Intent) {
        val config = DynamicProfileConfig.get(context)
        when (intent.action) {
            ACTION_SET_PROFILE -> {
                val profile = intent.getStringExtra("profile")?.trim()
                if (profile == null || profile !in PROFILES) {
                    Log.w(TAG, "automation: bad profile '${intent.getStringExtra("profile")}'")
                    return
                }
                Log.d(TAG, "automation: set profile $profile")
                config.baseProfile = profile
                // immediate apply when the service (daemon) is running;
                // otherwise the base takes effect at the next service start
                DaemonLink.send(JSONObject().put("cmd", "set_base").put("profile", profile))
            }

            ACTION_BOOST -> {
                Log.d(TAG, "automation: boost")
                DaemonLink.send(JSONObject().put("cmd", "boost"))
            }

            ACTION_SET_CHARGE_LIMIT -> {
                if (!intent.hasExtra("enabled")) return
                val enabled = intent.getBooleanExtra("enabled", config.chargeLimit)
                Log.d(TAG, "automation: charge limit $enabled")
                config.chargeLimit = enabled
                if (enabled && intent.hasExtra("pct")) {
                    config.chargeLimitPct = intent.getIntExtra("pct", config.chargeLimitPct)
                }
            }
        }
    }
}
