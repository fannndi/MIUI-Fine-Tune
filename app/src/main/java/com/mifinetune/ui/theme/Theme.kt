package com.mifinetune.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

/**
 * Material 3 theme. Static seed colors (no dynamic color: MIUI already owns
 * the wallpaper palette and its own system theming).
 */

private val Teal10 = Color(0xFF00201E)
private val Teal20 = Color(0xFF003734)
private val Teal30 = Color(0xFF00504B)
private val Teal40 = Color(0xFF006A66)
private val Teal80 = Color(0xFF4CDAD3)
private val Teal90 = Color(0xFF6FF7F0)
private val TealContainerLight = Color(0xFF6FF7F0)
private val OnTealContainerLight = Color(0xFF00201E)
private val TealContainerDark = Color(0xFF00504B)
private val OnTealContainerDark = Color(0xFF6FF7F0)

private val NeutralLight = Color(0xFFFAFDFC)
private val NeutralDark = Color(0xFF0E1514)
private val Amber40 = Color(0xFF7A5900)
private val Amber80 = Color(0xFFEFBE4B)

private val LightColors = lightColorScheme(
    primary = Teal40,
    onPrimary = Color.White,
    primaryContainer = TealContainerLight,
    onPrimaryContainer = OnTealContainerLight,
    secondary = Amber40,
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFFFDF9B),
    onSecondaryContainer = Color(0xFF261A00),
    background = NeutralLight,
    surface = NeutralLight,
    surfaceVariant = Color(0xFFDBE5E3),
    onSurfaceVariant = Color(0xFF3F4947),
    error = Color(0xFFBA1A1A),
)

private val DarkColors = darkColorScheme(
    primary = Teal80,
    onPrimary = Teal20,
    primaryContainer = TealContainerDark,
    onPrimaryContainer = OnTealContainerDark,
    secondary = Amber80,
    onSecondary = Color(0xFF412F00),
    secondaryContainer = Color(0xFF5C4400),
    onSecondaryContainer = Color(0xFFFFDF9B),
    background = NeutralDark,
    surface = NeutralDark,
    surfaceVariant = Color(0xFF3F4947),
    onSurfaceVariant = Color(0xFFBEC9C7),
    error = Color(0xFFFFB4AB),
)

@Composable
fun MiFineTuneTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    MaterialTheme(
        colorScheme = if (darkTheme) DarkColors else LightColors,
        content = content,
    )
}
