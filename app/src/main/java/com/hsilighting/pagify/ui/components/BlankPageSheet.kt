package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.FlowRowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.PageSize

/** Everything the reader chose about the paper. */
data class BlankSheet(
    val count: Int,
    val size: PageSize,
    /** ARGB, or null for paper left the colour paper already is. */
    val fill: Long?,
    val ruling: Ruling,
    /** Only meaningful when a whole document is being made. */
    val name: String,
)

/** What is printed on the paper before anything is written on it. */
enum class Ruling(val label: String, val code: Int) {
    None("Plain", 0),
    Lined("Lined", 1),
    Grid("Grid", 2),
    Dots("Dots", 3),
}

/**
 * A sheet of paper: how many, how big, which way up, what colour, and what is
 * already printed on it.
 *
 * Asked all at once because it is one decision — you are choosing paper, not
 * configuring five settings — and every part has an answer good enough that most
 * people will just press Add.
 *
 * The size defaults to the page it will follow, when there is one. A new sheet
 * that does not match its neighbours reads as a mistake in an otherwise uniform
 * document, and that is the common case; the standards are there for when it is
 * not.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun BlankPageSheet(
    /** The page the new sheet will follow, for the "same as this" option. */
    template: PageSize?,
    onAdd: (BlankSheet) -> Unit,
    onDismiss: () -> Unit,
    /**
     * True when this makes a document rather than adding to one. Only then are
     * "how many" and a file name questions at all — inserting into an open
     * document has an answer for both already.
     */
    newDocument: Boolean = false,
    /** The name offered when a document is being made. */
    suggestedName: String = "Notes",
) {
    val sizes = remember(template) { sheetSizes(template) }
    var chosen by remember(sizes) { mutableStateOf(sizes.first()) }
    var landscape by remember { mutableStateOf(false) }
    var fill by remember { mutableStateOf(SHEET_COLOURS.first().value) }
    var ruling by remember { mutableStateOf(Ruling.None) }
    var count by remember { mutableStateOf("1") }
    var name by remember(suggestedName) { mutableStateOf(suggestedName) }

    val pages = count.toIntOrNull()?.takeIf { it in 1..MAXIMUM_SHEETS }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (newDocument) "New document" else "Add a page") },
        text = {
            // Scrolls: with a name, a count, sizes, orientation, colour and
            // ruling this is taller than a short phone in landscape, and a dialog
            // that overflows hides its own buttons.
            Column(
                Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                if (newDocument) {
                    OutlinedTextField(
                        value = name,
                        onValueChange = { name = it },
                        label = { Text("Name") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    OutlinedTextField(
                        value = count,
                        onValueChange = { typed ->
                            // Digits only, and short: the field is a number, and a
                            // paste of something else should not become one.
                            count = typed.filter { it.isDigit() }.take(3)
                        },
                        label = { Text("Pages") },
                        singleLine = true,
                        isError = pages == null,
                        supportingText = if (pages == null) {
                            { Text("Between 1 and $MAXIMUM_SHEETS") }
                        } else {
                            null
                        },
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }

                Label("Size")
                // A flow row, not a row: five chips do not fit across a phone, and
                // a plain Row squeezes the last one until its label breaks across
                // two lines or disappears entirely.
                ChipRow {
                    sizes.forEach { sheet ->
                        Choice(sheet.label, sheet == chosen) { chosen = sheet }
                    }
                }

                ChipRow {
                    Choice("Portrait", !landscape) { landscape = false }
                    Choice("Landscape", landscape) { landscape = true }
                }

                Label("Colour")
                ChipRow {
                    SHEET_COLOURS.forEach { paper ->
                        Swatch(paper.value, paper.value == fill) { fill = paper.value }
                    }
                }

                Label("Ruling")
                ChipRow {
                    Ruling.entries.forEach { option ->
                        Choice(option.label, option == ruling) { ruling = option }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = pages != null,
                onClick = {
                    val size = chosen.size.let { if (landscape) it.turnedOnItsSide() else it }
                    onAdd(
                        BlankSheet(
                            count = pages ?: 1,
                            size = size,
                            // White is what an empty page already looks like, so it
                            // is sent as no fill at all rather than as a white
                            // rectangle covering the sheet.
                            fill = fill.takeIf { it != PAPER_WHITE },
                            ruling = ruling,
                            name = name.trim().ifEmpty { suggestedName },
                        ),
                    )
                },
            ) { Text(if (newDocument) "Create" else "Add") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ChipRow(content: @Composable FlowRowScope.() -> Unit) = FlowRow(
    horizontalArrangement = Arrangement.spacedBy(8.dp),
    verticalArrangement = Arrangement.spacedBy(8.dp),
    content = content,
)

@Composable
private fun Label(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun Choice(label: String, selected: Boolean, onClick: () -> Unit) {
    Box(
        Modifier
            .clip(RoundedCornerShape(10.dp))
            .background(
                if (selected) {
                    MaterialTheme.colorScheme.secondaryContainer
                } else {
                    MaterialTheme.colorScheme.surfaceVariant
                },
            )
            .clickable(onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 8.dp),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelLarge,
            textAlign = TextAlign.Center,
            // One line, always. A chip whose label wraps is taller than its
            // neighbours and reads as broken rather than as a long word.
            maxLines = 1,
            softWrap = false,
            color = if (selected) {
                MaterialTheme.colorScheme.onSecondaryContainer
            } else {
                MaterialTheme.colorScheme.onSurfaceVariant
            },
        )
    }
}

/**
 * A colour, shown as the paper itself.
 *
 * Outlined rather than filled with a tick: white paper on a white swatch needs an
 * edge to exist at all, and the same edge doing the selecting keeps the row
 * reading as a row of sheets.
 */
@Composable
private fun Swatch(colour: Long, selected: Boolean, onClick: () -> Unit) {
    Box(
        Modifier
            .size(36.dp)
            .clip(CircleShape)
            .background(Color(colour))
            .border(
                width = if (selected) 3.dp else 1.dp,
                color = if (selected) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.outlineVariant
                },
                shape = CircleShape,
            )
            .clickable(onClick = onClick),
    )
}

/** One offered sheet size. */
data class SheetSize(val label: String, val size: PageSize)

/**
 * The sizes on offer, with the page being followed first when there is one.
 *
 * Deduplicated against that page, so a document that is already A4 does not offer
 * A4 twice under two names.
 */
private fun sheetSizes(template: PageSize?): List<SheetSize> {
    val standards = listOf(
        SheetSize("A4", PageSize(A4_WIDE, A4_TALL)),
        SheetSize("A3", PageSize(A3_WIDE, A3_TALL)),
        SheetSize("Letter", PageSize(LETTER_WIDE, LETTER_TALL)),
        SheetSize("Square", PageSize(A4_WIDE, A4_WIDE)),
    )
    if (template == null) return standards

    val same = SheetSize("Same as this", template)
    return listOf(same) + standards.filterNot { it.size.matches(template) }
}

/** Within a point: paper sizes are quoted in millimetres and converted. */
private fun PageSize.matches(other: PageSize): Boolean =
    kotlin.math.abs(widthPoints - other.widthPoints) < 1f &&
        kotlin.math.abs(heightPoints - other.heightPoints) < 1f

private fun PageSize.turnedOnItsSide(): PageSize =
    if (widthPoints >= heightPoints) this else PageSize(heightPoints, widthPoints)

/** The papers on offer. White first, because it is what paper usually is. */
private val SHEET_COLOURS = listOf(
    Paper("White", PAPER_WHITE),
    Paper("Cream", 0xFFFFF6E0),
    Paper("Grey", 0xFFBFC3C7),
    Paper("Black", 0xFF101214),
    Paper("Blue", 0xFF1B3A5C),
)

private data class Paper(val name: String, val value: Long)

/**
 * As many sheets as one dialog should make in one press.
 *
 * Not a technical limit — the engine will build more. A cap, because "500" is far
 * more often a typo than a request, and a dotted sheet carries a couple of
 * thousand objects apiece.
 */
private const val MAXIMUM_SHEETS = 200

private const val PAPER_WHITE = 0xFFFFFFFF

private const val A4_WIDE = 595f
private const val A4_TALL = 842f
private const val A3_WIDE = 842f
private const val A3_TALL = 1191f
private const val LETTER_WIDE = 612f
private const val LETTER_TALL = 792f
