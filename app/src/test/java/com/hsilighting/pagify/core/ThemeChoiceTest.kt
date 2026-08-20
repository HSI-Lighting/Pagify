package com.hsilighting.pagify.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Which way round the theme goes.
 *
 * An inverted branch here shows the right thing on a phone in light mode and the
 * wrong thing on every other, which is the kind of bug that ships.
 */
class ThemeChoiceTest {

    @Test
    fun `following the system means following the system`() {
        assertTrue(ThemeChoice.SYSTEM.isDark(systemIsDark = true))
        assertFalse(ThemeChoice.SYSTEM.isDark(systemIsDark = false))
    }

    @Test
    fun `a chosen theme overrides the phone`() {
        // The whole point of the setting: a phone in dark mode still draws light
        // when light is what was asked for.
        assertFalse(ThemeChoice.LIGHT.isDark(systemIsDark = true))
        assertTrue(ThemeChoice.DARK.isDark(systemIsDark = false))
    }

    @Test
    fun `a stored choice reads back`() {
        ThemeChoice.entries.forEach { choice ->
            assertEquals(choice, themeChoiceFrom(choice.name))
        }
    }

    @Test
    fun `anything unreadable follows the phone`() {
        // A settings file from a newer build, or half-written. Following the phone
        // is the answer that is never jarring.
        assertEquals(ThemeChoice.SYSTEM, themeChoiceFrom(null))
        assertEquals(ThemeChoice.SYSTEM, themeChoiceFrom(""))
        assertEquals(ThemeChoice.SYSTEM, themeChoiceFrom("MIDNIGHT"))
        assertEquals(ThemeChoice.SYSTEM, themeChoiceFrom("dark"))
    }
}
