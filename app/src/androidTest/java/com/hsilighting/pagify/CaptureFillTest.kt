package com.hsilighting.pagify

import com.hsilighting.pagify.core.PdfFont
import android.graphics.Bitmap
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.hsilighting.pagify.core.CaptureFill
import com.hsilighting.pagify.core.CaptureRequest
import com.hsilighting.pagify.core.CaptureTile
import com.hsilighting.pagify.ui.components.CaptureEditor
import com.hsilighting.pagify.ui.reader.CapturePreview
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The fill picker: is it there, and only where it means something?
 *
 * On a device this control can only be reached by drawing a ring, and a ring is
 * the one gesture the shell cannot inject — `input swipe` interpolates a straight
 * line, which the area check correctly refuses. So the editor is composed directly
 * with a capture that already has a ring on it.
 */
@RunWith(AndroidJUnit4::class)
class CaptureFillTest {

    @get:Rule
    val rule = createComposeRule()

    private var chosen: CaptureFill? = null

    private fun preview(ring: List<Offset>): CapturePreview {
        val request = CaptureRequest(
            tiles = listOf(
                CaptureTile(pageIndex = 0, crop = Rect(0f, 0f, 100f, 100f), dest = Rect(0f, 0f, 100f, 100f)),
            ),
            width = 100f,
            height = 100f,
            background = 0xFFFFFFFFL,
            originPage = 0,
            mask = ring,
        )
        val bitmap = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888)
        return CapturePreview(request, ByteArray(64), "test.png", bitmap.asImageBitmap())
    }

    private fun showEditor(ring: List<Offset>) {
        chosen = null
        rule.setContent {
            CaptureEditor(
                preview = preview(ring),
                isCapturing = false,
                markup = emptyList(),
                markupTool = com.hsilighting.pagify.core.MarkupTool.Pen,
                markupColor = 0xFFFF0000L,
                markupSize = 4f,
                markupStyle = com.hsilighting.pagify.core.MarkupStyle.SOLID,
                onMarkupStyle = {},
                fill = CaptureFill.PAGE,
                onFillChange = { chosen = it },
                onScaleChange = {},
                onFormatChange = {},
                onMarkupTool = {},
                onMarkupColor = {},
                onMarkupSize = { _, _ -> },
                onCommitMarkup = {},
                // Added when text markup landed and never wired here, which is
                // what had the whole instrumented suite failing to compile. The
                // values are inert on purpose: this test is about the fills.
                markupArmed = false,
                selectedMarkup = null,
                onDisarmMarkup = {},
                textFont = PdfFont.HELVETICA,
                textSizePoints = 12f,
                textCurveDegrees = 0f,
                onTextFont = {},
                onTextSize = {},
                onTextCurve = {},
                onMoveMarkup = { _, _ -> },
                onSelectMarkup = {},
                onScaleMarkup = {},
                onRewriteMarkup = { _, _ -> },
                onEraseMarkup = {},
                onRecogniseMarkup = {},
                onUndoMarkup = {},
                onSaveToGallery = {},
                onShare = {},
                onCopy = {},
                onDismiss = {},
            )
        }
    }

    /** A square ring, which is what a lasso capture arrives with. */
    private fun ring() = listOf(
        Offset(10f, 10f),
        Offset(90f, 10f),
        Offset(90f, 90f),
        Offset(10f, 90f),
    )

    @Test
    fun aRingedCaptureOffersTheFills() {
        showEditor(ring())

        rule.onNodeWithText("Around it").assertIsDisplayed()
        CaptureFill.entries.forEach { fill ->
            rule.onNodeWithText(fill.label).assertIsDisplayed()
        }
    }

    @Test
    fun choosingAFillReportsIt() {
        showEditor(ring())

        rule.onNodeWithText(CaptureFill.TRANSPARENT.label).performClick()
        rule.waitForIdle()

        assertEquals(CaptureFill.TRANSPARENT, chosen)
    }

    @Test
    fun aBoxCaptureHasNoOutsideAndSoNoFill() {
        // The control would have nothing to act on: a box capture is all page.
        showEditor(emptyList())

        rule.onAllNodesWithText("Around it").assertCountEquals(0)
    }
}
