package com.mifinetune.dynamic

import android.app.KeyguardManager
import android.content.Context
import android.os.PowerManager

/**
 * Screen/keyguard context readers — Android-only values the daemon cannot
 * read itself (no binder access from its root shell context).
 *
 * Responsibility: turning Android services into plain values.
 * Non-goals: decisions (Rust daemon), lifecycle (DynamicProfileService).
 *
 * The foreground/multi-window watchers that used to live here moved into
 * the Rust daemon (`core/src/daemon/watcher.rs`, logcat -v epoch).
 */
class DeviceContextReader(private val context: Context) {

    private val keyguard =
        context.getSystemService(Context.KEYGUARD_SERVICE) as KeyguardManager
    private val power =
        context.getSystemService(Context.POWER_SERVICE) as PowerManager

    val screenOn: Boolean get() = power.isInteractive
    val keyguardLocked: Boolean get() = keyguard.isKeyguardLocked
}
