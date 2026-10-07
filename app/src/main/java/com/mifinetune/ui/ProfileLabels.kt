package com.mifinetune.ui

/** Shared display labels for profile ids (rows, pickers, notifications). */
object ProfileLabels {
    private val LABELS = mapOf(
        "sleep" to "Sleep",
        "powersave" to "Power Save",
        "balance" to "Balance",
        "game" to "Game",
    )

    private val SHORT = mapOf(
        "powersave" to "Battery-first",
        "balance" to "Everyday balanced",
        "game" to "Performance first",
        "sleep" to "Screen-off",
    )

    fun of(id: String?): String = id?.let { LABELS[it] ?: it } ?: "Default"

    fun shortDesc(id: String): String = SHORT[id] ?: ""
}
