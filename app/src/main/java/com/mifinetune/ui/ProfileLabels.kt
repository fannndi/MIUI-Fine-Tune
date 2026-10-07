package com.mifinetune.ui

/** Shared display labels for profile ids (chips, pickers, notifications). */
object ProfileLabels {
    private val MAP = mapOf(
        "sleep" to "Sleep",
        "powersave" to "Power Save",
        "balance" to "Balance",
        "game" to "Game",
    )

    fun of(id: String?): String = id?.let { MAP[it] ?: it } ?: "Default"
}
