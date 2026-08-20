package com.hsilighting.pagify

import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.hsilighting.pagify.core.RecentDocument
import com.hsilighting.pagify.ui.library.LibraryScreen
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The app's front door.
 *
 * The list's ordering and its file are tested off-device; this is about the screen
 * doing what a row promises — that tapping one opens that document and not its
 * neighbour, which is the kind of thing an index bug gets subtly wrong.
 */
@RunWith(AndroidJUnit4::class)
class LibraryScreenTest {

    @get:Rule
    val rule = createComposeRule()

    private var opened: RecentDocument? = null
    private var picked = 0

    private val documents = listOf(
        RecentDocument("content://x/1", "Site survey.pdf", 2_500_000L, 12, 1_698_140_000_000L),
        RecentDocument("content://x/2", "Invoice 88.pdf", 156_000L, 2, 1_697_000_000_000L),
    )

    private fun show(documents: List<RecentDocument>) {
        opened = null
        picked = 0
        rule.setContent {
            LibraryScreen(
                documents = documents,
                onOpen = { opened = it },
                onForget = {},
                onPickDocument = { picked++ },
            )
        }
    }

    @Test
    fun everyDocumentIsListedWithWhatItIs() {
        show(documents)

        rule.onNodeWithText("Site survey.pdf").assertIsDisplayed()
        rule.onNodeWithText("Invoice 88.pdf").assertIsDisplayed()
        // Date, pages and size, in the row's own subtitle.
        rule.onAllNodesWithText("12 pages", substring = true).assertCountEquals(1)
        rule.onAllNodesWithText("2.4 MB", substring = true).assertCountEquals(1)
    }

    @Test
    fun tappingARowOpensThatDocument() {
        show(documents)

        rule.onNodeWithText("Invoice 88.pdf").performClick()
        rule.waitForIdle()

        assertEquals(documents[1], opened)
    }

    @Test
    fun searchingNarrowsTheList() {
        show(documents)

        rule.onNodeWithText("Search files…").performTextInput("invoice")
        rule.waitForIdle()

        rule.onNodeWithText("Invoice 88.pdf").assertIsDisplayed()
        rule.onAllNodesWithText("Site survey.pdf").assertCountEquals(0)
    }

    @Test
    fun anEmptyLibrarySaysSoAndOffersAWayOut() {
        show(emptyList())

        rule.onNodeWithText("Nothing here yet").assertIsDisplayed()
        rule.onNodeWithText("Open a PDF").performClick()
        rule.waitForIdle()

        assertEquals(1, picked)
    }
}
