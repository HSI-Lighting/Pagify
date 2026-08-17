package com.hsilighting.pagify.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

private val LightColors = lightColorScheme(
    primary = Color(0xFF3F5F90),
    onPrimary = Color.White,
    secondary = Color(0xFF565F71),
    // A reader spends most of its time showing white pages, so the chrome around
    // them is kept slightly grey — a pure-white background would make page edges
    // disappear.
    background = Color(0xFFF4F4F7),
    surface = Color(0xFFFDFBFF),
)

private val DarkColors = darkColorScheme(
    primary = Color(0xFFA8C7FA),
    onPrimary = Color(0xFF0A305F),
    secondary = Color(0xFFBEC6DC),
    background = Color(0xFF121316),
    surface = Color(0xFF1A1B1F),
)

@Composable
fun PagifyTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    /** Material You, where the platform supports it. */
    dynamicColor: Boolean = true,
    content: @Composable () -> Unit,
) {
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        darkTheme -> DarkColors
        else -> LightColors
    }

    MaterialTheme(colorScheme = colorScheme, content = content)
}
