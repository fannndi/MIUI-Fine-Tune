package com.mifinetune.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

/**
 * Material 3 theme — calm pastel palette.
 *
 * Soft lavender primary, mint secondary, peach tertiary on a near-white
 * surface. Dark variant keeps the same hues at lower chroma for night use.
 * Static (no dynamic color): MIUI already owns the wallpaper palette.
 */

// --- pastel seeds -----------------------------------------------------------
private val Lavender40 = Color(0xFF6B5B8E)
private val Lavender80 = Color(0xFFD4C8F0)
private val Lavender90 = Color(0xFFEDE6FA)
private val LavenderContainerLight = Color(0xFFEDE6FA)
private val OnLavenderContainerLight = Color(0xFF2A1F45)
private val LavenderContainerDark = Color(0xFF3A2F5C)
private val OnLavenderContainerDark = Color(0xFFEDE6FA)

private val Mint40 = Color(0xFF4F7F6A)
private val Mint80 = Color(0xFFB8E0CC)
private val MintContainerLight = Color(0xFFD6F0E3)
private val OnMintContainerLight = Color(0xFF123325)
private val MintContainerDark = Color(0xFF234A3A)
private val OnMintContainerDark = Color(0xFFD6F0E3)

private val Peach40 = Color(0xFF9A6B5A)
private val Peach80 = Color(0xFFF2C9B6)
private val PeachContainerLight = Color(0xFFFCE2D6)
private val OnPeachContainerLight = Color(0xFF3A1F14)
private val PeachContainerDark = Color(0xFF5A3A2E)
private val OnPeachContainerDark = Color(0xFFFCE2D6)

private val NeutralLight = Color(0xFFF7F5FB)
private val NeutralDark = Color(0xFF1A1820)
private val SurfaceVariantLight = Color(0xFFE8E3F2)
private val SurfaceVariantDark = Color(0xFF2E2A38)
private val OutlineLight = Color(0xFFB7B0C9)
private val OutlineDark = Color(0xFF6E6880)

private val LightColors = lightColorScheme(
    primary = Lavender40,
    onPrimary = Color.White,
    primaryContainer = LavenderContainerLight,
    onPrimaryContainer = OnLavenderContainerLight,
    secondary = Mint40,
    onSecondary = Color.White,
    secondaryContainer = MintContainerLight,
    onSecondaryContainer = OnMintContainerLight,
    tertiary = Peach40,
    onTertiary = Color.White,
    tertiaryContainer = PeachContainerLight,
    onTertiaryContainer = OnPeachContainerLight,
    background = NeutralLight,
    onBackground = Color(0xFF1C1A24),
    surface = NeutralLight,
    onSurface = Color(0xFF1C1A24),
    surfaceVariant = SurfaceVariantLight,
    onSurfaceVariant = Color(0xFF474255),
    outline = OutlineLight,
    error = Color(0xFFB3261E),
    onError = Color.White,
)

private val DarkColors = darkColorScheme(
    primary = Lavender80,
    onPrimary = Color(0xFF2A1F45),
    primaryContainer = LavenderContainerDark,
    onPrimaryContainer = OnLavenderContainerDark,
    secondary = Mint80,
    onSecondary = Color(0xFF123325),
    secondaryContainer = MintContainerDark,
    onSecondaryContainer = OnMintContainerDark,
    tertiary = Peach80,
    onTertiary = Color(0xFF3A1F14),
    tertiaryContainer = PeachContainerDark,
    onTertiaryContainer = OnPeachContainerDark,
    background = NeutralDark,
    onBackground = Color(0xFFE8E3F2),
    surface = NeutralDark,
    onSurface = Color(0xFFE8E3F2),
    surfaceVariant = SurfaceVariantDark,
    onSurfaceVariant = Color(0xFFCBC4DB),
    outline = OutlineDark,
    error = Color(0xFFF2B8B5),
    onError = Color(0xFF601410),
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