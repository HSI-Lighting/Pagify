package com.hsilighting.pagify.core

/**
 * Light, dark, or whatever the phone is doing.
 *
 * [SYSTEM] is the default because a reader is one app among many and following
 * the phone is what most people expect — but only most: a document is a white
 * page whatever the app around it does, and someone reading in bed with the phone
 * in dark mode may still want the chrome light, or the other way about. That is
 * the whole reason this is a setting rather than a rule.
 */
enum class ThemeChoice(val label: String) {
    SYSTEM("System"),
    LIGHT("Light"),
    DARK("Dark"),
}

/**
 * Whether to draw dark, given what the phone is set to.
 *
 * Pure, and separate from the composable that asks it, because it is the one
 * piece of this that can be wrong in a way nobody sees until they are in the
 * dark: an inverted branch shows the right thing on a phone in light mode and the
 * wrong thing everywhere else.
 */
fun ThemeChoice.isDark(systemIsDark: Boolean): Boolean = when (this) {
    ThemeChoice.SYSTEM -> systemIsDark
    ThemeChoice.LIGHT -> false
    ThemeChoice.DARK -> true
}

/**
 * Read a stored choice back, falling back to following the phone.
 *
 * Lenient because the alternative is worse: a settings file written by a newer
 * build, or half-written, should leave the app looking like the phone rather than
 * refusing to start.
 */
fun themeChoiceFrom(stored: String?): ThemeChoice =
    ThemeChoice.entries.firstOrNull { it.name == stored } ?: ThemeChoice.SYSTEM
