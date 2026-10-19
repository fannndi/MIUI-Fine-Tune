package com.mifinetune.dynamic

import android.content.Context
import android.os.Build
import android.os.PowerManager
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Android 10+ thermal status via PowerManager.addThermalStatusChangedListener.
 * Exposes a live [StateFlow] of the current thermal status code so the UI can
 * react without polling sysfs. No-op below API 29.
 */
class ThermalStatusMonitor(context: Context) {
    private val pm = context.getSystemService(Context.POWER_SERVICE) as? PowerManager
    private val _status = MutableStateFlow(pm?.currentThermalStatus ?: 0)
    val status: StateFlow<Int> = _status.asStateFlow()

    private val listener = if (Build.VERSION.SDK_INT >= 29) {
        PowerManager.OnThermalStatusChangedListener { s -> _status.value = s }
    } else null

    init {
        if (Build.VERSION.SDK_INT >= 29) {
            listener?.let { pm?.addThermalStatusChangedListener(it) }
        }
    }

    fun label(code: Int): String = when (code) {
        0 -> "none"
        1 -> "light"
        2 -> "moderate"
        3 -> "severe"
        4 -> "critical"
        5 -> "emergency"
        6 -> "shutdown"
        else -> "—"
    }

    fun close() {
        if (Build.VERSION.SDK_INT >= 29) {
            listener?.let { pm?.removeThermalStatusChangedListener(it) }
        }
    }
}