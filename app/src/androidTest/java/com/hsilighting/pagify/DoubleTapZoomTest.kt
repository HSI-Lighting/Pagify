package com.hsilighting.pagify

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.doubleClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.hsilighting.pagify.ui.components.doubleTapToZoom
import com.hsilighting.pagify.ui.components.pinchToZoom
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Does a double tap on the reader reach the reader?
 *
 * Reported from a phone: double-tap to zoom does nothing, most of the time. The
 * handler is there and its own recording shows it firing occasionally, so the
 * question is not "is it wired up" but "does the gesture arrive" — and that
 * depends on the modifiers and the scroll container around it, which is the one
 * thing neither a unit test nor `adb` can reproduce. Two injected `input tap`
 * calls are 300 ms apart at best, which is the double-tap timeout itself.
 *
 * The first test is the control: the same handler with nothing around it. If that
 * passes and the second fails, the reader's own arrangement is what is eating it.
 */
@RunWith(AndroidJUnit4::class)
class DoubleTapZoomTest {

    @get:Rule
    val rule = createComposeRule()

    private var doubleTaps = 0

    @Test
    fun aDoubleTapOnItsOwnIsReceived() {
        doubleTaps = 0
        rule.setContent {
            Box(
                Modifier
                    .fillMaxSize()
                    .testTag(TAG)
                    .pointerInput(Unit) {
                        detectTapGestures(onDoubleTap = { doubleTaps++ })
                    },
            )
        }

        rule.onNodeWithTag(TAG).performTouchInput { doubleClick(Offset(200f, 400f)) }
        rule.waitForIdle()

        assertEquals(1, doubleTaps)
    }

    @Test
    fun aDoubleTapOverTheScrollingListIsReceived() {
        doubleTaps = 0
        // The reader's own arrangement: the zoom handlers sit on a Box that wraps
        // the scrolling list of pages, so every tap reaches the list first.
        rule.setContent {
            Box(
                Modifier
                    .fillMaxSize()
                    .testTag(TAG)
                    .pinchToZoom { _, _ -> }
                    .pointerInput(Unit) {
                        detectTapGestures(onDoubleTap = { doubleTaps++ })
                    },
            ) {
                LazyColumn(Modifier.fillMaxSize()) {
                    items((0 until 20).toList()) { index ->
                        Box(
                            Modifier
                                .fillMaxWidth()
                                .height(400.dp)
                                .background(if (index % 2 == 0) Color.White else Color.LightGray),
                        )
                    }
                }
            }
        }

        rule.onNodeWithTag(TAG).performTouchInput { doubleClick(Offset(200f, 400f)) }
        rule.waitForIdle()

        assertEquals(1, doubleTaps)
    }

    @Test
    fun aDoubleTapOverAPageThatHandlesTapsIsReceived() {
        doubleTaps = 0
        var pageTaps = 0
        // The arrangement that actually ships. Every page in the list carries a tap
        // handler even with no tool armed — it is what opens a note and what clears
        // a text selection — and a child handles the Main pass first. `detectTapGestures`
        // consumes what it handles, so the zoom handler on the parent never sees a
        // down at all, and double-tap to zoom is dead on every page.
        rule.setContent {
            Box(
                Modifier
                    .fillMaxSize()
                    .testTag(TAG)
                    .pinchToZoom { _, _ -> }
                    .doubleTapToZoom { doubleTaps++ },
            ) {
                LazyColumn(Modifier.fillMaxSize()) {
                    items((0 until 20).toList()) { index ->
                        Box(
                            Modifier
                                .fillMaxWidth()
                                .height(400.dp)
                                .background(if (index % 2 == 0) Color.White else Color.LightGray)
                                .pointerInput(Unit) {
                                    detectTapGestures(onTap = { pageTaps++ })
                                },
                        )
                    }
                }
            }
        }

        rule.onNodeWithTag(TAG).performTouchInput { doubleClick(Offset(200f, 400f)) }
        rule.waitForIdle()

        assertEquals("the page swallowed the double tap", 1, doubleTaps)
    }

    private companion object {
        const val TAG = "reader-surface"
    }
}
