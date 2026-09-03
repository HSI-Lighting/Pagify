package com.hsilighting.pagify.ui.contacts

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.util.Log
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.automirrored.filled.Undo
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.net.toUri
import com.hsilighting.pagify.core.CardFieldKind
import com.hsilighting.pagify.core.CardReading
import com.hsilighting.pagify.core.ReadField
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlin.math.min

/**
 * One card waiting to be checked, as the screen needs it.
 *
 * Deliberately not the view model's own review state, which also carries where
 * the cards will be filed and which have been kept so far. The screen has no use
 * for either, and a screen that can see them is a screen that can be tempted to
 * act on them.
 */
data class CardReviewState(
    val imageUri: String,
    val reading: CardReading,
    val position: Int,
    val total: Int,
)

/**
 * Checking what was read, against the card it was read from.
 *
 * The photograph fills the screen dimmed, each thing that was read is ringed on
 * it, and the values sit in a translucent panel against the card — close enough
 * to compare without covering what they describe.
 *
 * The values are numbered to match the rings rather than drawn at them. Drawing
 * them in place was the first attempt and the reason for this one: the lines of a
 * business card are a few millimetres apart and readable type is not, so four
 * labelled values landed on top of each other and on the card's own text. It was
 * less legible than the card it was explaining.
 *
 * Only the printed path gets here. A card read from a QR is exact and has no
 * regions; asking somebody to confirm a value that cannot be wrong is a step for
 * its own sake.
 */
@Composable
fun CardReviewSheet(
    imageUri: String,
    reading: CardReading,
    /** Which of how many, when one photograph held several cards. */
    position: Int,
    total: Int,
    /** From settings: this is read at arm's length, in an event's lighting. */
    textScale: Float,
    /** What survived the review, in the order it is shown. */
    onSave: (List<ReadField>) -> Unit,
    onSkip: () -> Unit,
) {
    val context = LocalContext.current
    var photo by remember(imageUri) { mutableStateOf<Photo?>(null) }

    // The working copy. Keyed on the reading, so moving to the next card in a
    // photograph starts from that card's own fields rather than the last one's.
    // Every field the card produced, and which of them have been swiped away.
    // Kept as a set of indices rather than two lists so a removal is reversible:
    // the field is still here, it is simply not being saved.
    var all by remember(reading) { mutableStateOf(reading.fields) }
    var removed by remember(reading) { mutableStateOf(emptySet<Int>()) }
    var correcting by remember(reading) { mutableStateOf<Int?>(null) }

    val shown = all.withIndex().filter { it.index !in removed }

    LaunchedEffect(imageUri) {
        photo = withContext(Dispatchers.IO) {
            runCatching { loadPhoto(context, imageUri.toUri()) }
                .onFailure { Log.e("CardReview", "the photograph could not be opened", it) }
                .getOrNull()
        }
    }

    Dialog(
        onDismissRequest = onSkip,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            Modifier
                .fillMaxSize()
                .background(Color(0xFF0B0B0B)),
        ) {
            val loaded = photo
            if (loaded == null) {
                Text(
                    "Opening the photograph…",
                    color = Color.White.copy(alpha = 0.7f),
                    modifier = Modifier.align(Alignment.Center),
                )
            } else {
                CardAndPanel(
                    photo = loaded,
                    shown = shown,
                    removed = removed.map { all[it] },
                    heading = if (total > 1) {
                        "Card ${position + 1} of $total"
                    } else {
                        "Check the card"
                    },
                    textScale = textScale,
                    onEdit = { correcting = it },
                    onDrop = { at -> removed = removed + at },
                    onRestore = { field -> removed = removed - all.indexOf(field) },
                )
            }

            Row(
                Modifier
                    .align(Alignment.BottomCenter)
                    .fillMaxWidth()
                    .background(Color(0xFF0B0B0B))
                    .padding(20.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TextButton(onClick = onSkip, modifier = Modifier.weight(1f)) {
                    // "Skip", not "Cancel": with several cards in one photograph
                    // this passes over one and goes on to the next.
                    Text(if (total > 1) "Skip this one" else "Discard")
                }
                Button(
                    onClick = { onSave(shown.map { it.value }) },
                    enabled = shown.isNotEmpty(),
                    modifier = Modifier.weight(1f),
                ) { Text("Save") }
            }
        }
    }

    correcting?.let { at ->
        all.getOrNull(at)?.let { field ->
            CorrectField(
                field = field,
                onDone = { corrected ->
                    all = all.toMutableList().also {
                        // A corrected number is no longer the one the engine
                        // normalised, so that form goes: exporting a dialable
                        // number that disagrees with what is on screen is worse
                        // than exporting none.
                        it[at] = field.copy(value = corrected, normalised = null)
                    }
                    correcting = null
                },
                onDismiss = { correcting = null },
            )
        }
    }
}

/**
 * The photograph with its markers, and the panel of values against it.
 *
 * The panel is placed relative to the card rather than at a fixed height: below
 * the read text when there is room under it, over the picture otherwise. A panel
 * pinned to the bottom of the screen is a long way from a card sitting high in
 * the frame, and comparing the two is the whole point.
 *
 * Markers and panel share one coordinate system, so both are computed here.
 */
@Composable
private fun CardAndPanel(
    photo: Photo,
    shown: List<IndexedValue<ReadField>>,
    removed: List<ReadField>,
    heading: String,
    textScale: Float,
    onEdit: (Int) -> Unit,
    onDrop: (Int) -> Unit,
    onRestore: (ReadField) -> Unit,
) {
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val boxWidth = constraints.maxWidth.toFloat()
        val boxHeight = constraints.maxHeight.toFloat()
        val bitmap = photo.bitmap

        // The photograph gets the space the panel does not. Fitting it to the
        // whole box and laying the panel over it put the panel across the card,
        // which is the one thing this screen must not do: the values are there to
        // be compared against the card, and a card nobody can see is a list.
        val density = LocalDensity.current
        val panelMax = boxHeight * PANEL_SHARE
        val forPhoto = boxHeight - panelMax - with(density) { BOTTOM_BAR.toPx() }

        val fit = min(boxWidth / bitmap.width, forPhoto / bitmap.height)
        val shownWidth = bitmap.width * fit
        val shownHeight = bitmap.height * fit
        // Regions are in the full photograph and the bitmap is a smaller copy of
        // it: this is the factor between them, and it is not `shown`. Deriving it
        // from the decoded bitmap would move every marker by the sampling factor.
        val scale = if (photo.sourceWidth > 0) shownWidth / photo.sourceWidth else fit
        val offsetX = (boxWidth - shownWidth) / 2f
        val offsetY = (forPhoto - shownHeight) / 2f

        val accent = MaterialTheme.colorScheme.primary
        val badge = BADGE * textScale

        Image(
            bitmap = bitmap.asImageBitmap(),
            contentDescription = "The card that was photographed",
            contentScale = ContentScale.Fit,
            modifier = Modifier
                .fillMaxWidth()
                .height(with(density) { forPhoto.toDp() }),
        )

        // Dim everything, so the markers and the panel read as foreground. A
        // light scrim leaves the picture and the values competing.
        Box(
            Modifier
                .fillMaxSize()
                .background(Color.Black.copy(alpha = 0.62f)),
        )

        shown.forEachIndexed { position, entry ->
            val field = entry.value
            val region = field.region ?: return@forEachIndexed

            with(density) {
                Box(
                    Modifier
                        .offset(
                            x = (offsetX + region.left * scale).toDp(),
                            y = (offsetY + region.top * scale).toDp(),
                        )
                        .size(
                            width = (region.width * scale).toDp(),
                            height = (region.height * scale).toDp(),
                        )
                        .border(2.dp, accent, RoundedCornerShape(3.dp))
                        .background(accent.copy(alpha = 0.22f), RoundedCornerShape(3.dp)),
                )
                // The number goes *after* the line, not before it. On the left
                // corner it covered the first letter of every value, and a name
                // reading "aseen Anwar" is exactly what makes somebody distrust
                // the check they are being asked to make.
                val badgeAfter = (offsetX + region.right * scale).toDp() + 4.dp
                val fits = badgeAfter + badge < (offsetX + shownWidth).toDp()

                Box(
                    Modifier
                        .offset(
                            x = if (fits) badgeAfter else badgeAfter - badge - 8.dp,
                            y = (offsetY + region.top * scale).toDp() +
                                (((region.height * scale).toDp() - badge) / 2f),
                        )
                        .size(badge)
                        .clip(CircleShape)
                        .background(accent),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = "${position + 1}",
                        color = MaterialTheme.colorScheme.onPrimary,
                        fontSize = (12 * textScale).sp,
                        fontWeight = FontWeight.Bold,
                    )
                }
            }
        }

        // Where the read text actually sits, so the panel can go beside it rather
        // than at some fixed place on the screen.
        val cardBottom = shown.mapNotNull { it.value.region }
            .maxOfOrNull { offsetY + it.bottom * scale }
            ?: (offsetY + shownHeight)

        with(density) {
            // Sized to the share reserved for it above, so it sits directly
            // under the photograph rather than over it. Hanging it in the gap
            // below the card was the first attempt and showed two fields out of
            // six — on an ordinary card the text runs most of the way down, so
            // that gap is small.
            Surface(
                color = Color.Black.copy(alpha = 0.72f),
                shape = RoundedCornerShape(16.dp),
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .padding(start = 12.dp, end = 12.dp, bottom = BOTTOM_BAR)
                    .fillMaxWidth()
                    .heightIn(max = panelMax.toDp()),
            ) {
                Column(Modifier.padding(vertical = 12.dp)) {
                    Text(
                        text = heading,
                        color = Color.White,
                        fontSize = (15 * textScale).sp,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 2.dp),
                    )
                    SwipeInstruction(textScale)
                    // The list yields space to what follows rather than taking
                    // all of it: it scrolls, so it would otherwise push the
                    // removed strip out of the panel and there would be no way
                    // back from a swipe after all.
                    ReadFields(
                        shown = shown,
                        textScale = textScale,
                        onEdit = onEdit,
                        onDrop = onDrop,
                        modifier = Modifier.weight(1f, fill = false),
                    )
                    if (removed.isNotEmpty()) Removed(removed, textScale, onRestore)
                }
            }
        }
    }
}

/**
 * A line saying the rows can be swiped.
 *
 * Spelled out because a swipe is invisible: nothing about a row suggests it
 * moves, and a correction somebody does not know they can make is a correction
 * they do not make. The icons carry the meaning; the words say which way.
 */
@Composable
private fun SwipeInstruction(textScale: Float) {
    Row(
        Modifier.padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            Icons.Filled.Edit,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier.size((14 * textScale).dp),
        )
        Text(
            text = "Swipe right to correct",
            color = Color.White.copy(alpha = 0.75f),
            fontSize = (11 * textScale).sp,
            modifier = Modifier.padding(start = 4.dp, end = 12.dp),
        )
        Icon(
            Icons.Filled.Delete,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.error,
            modifier = Modifier.size((14 * textScale).dp),
        )
        Text(
            text = "left to remove",
            color = Color.White.copy(alpha = 0.75f),
            fontSize = (11 * textScale).sp,
            modifier = Modifier.padding(start = 4.dp),
        )
    }
}

/**
 * What was read, big enough to check at arm's length — and correctable in place.
 *
 * **Swipe left to drop a field, right to correct it.** Both directions on the
 * same row, because both answers to "that is wrong" are common: the recogniser
 * either read something that is not there, or read something real and got a
 * character of it wrong. Making one a swipe and the other a menu would say those
 * are different kinds of act.
 *
 * A dropped field is genuinely absent from what is saved rather than hidden — see
 * `contactFrom`. The raw text is untouched either way, so a value removed here is
 * still findable by search afterwards.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ReadFields(
    shown: List<IndexedValue<ReadField>>,
    textScale: Float,
    onEdit: (Int) -> Unit,
    onDrop: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier.verticalScroll(rememberScrollState())) {
        if (shown.isEmpty()) {
            Text(
                "Nothing left to save from this card.",
                color = Color.White.copy(alpha = 0.7f),
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )
            return@Column
        }

        shown.forEachIndexed { position, entry ->
            val field = entry.value
            // Keyed on the underlying index: without it, dropping a row leaves
            // the dismiss state attached to the row that slid up into its place,
            // which then draws itself already swiped away.
            key(entry.index, field.value) {
                val dismiss = rememberSwipeToDismissBoxState(
                    confirmValueChange = { value ->
                        when (value) {
                            SwipeToDismissBoxValue.EndToStart -> {
                                onDrop(entry.index)
                                true
                            }
                            SwipeToDismissBoxValue.StartToEnd -> {
                                // Editing is not a dismissal — the row stays and
                                // the dialog opens over it, so `false` puts the
                                // row back where it was.
                                onEdit(entry.index)
                                false
                            }
                            else -> false
                        }
                    },
                )

                SwipeToDismissBox(
                    state = dismiss,
                    backgroundContent = { SwipeHint(dismiss.dismissDirection, textScale) },
                ) {
                    FieldRow(position + 1, field, textScale)
                }
            }
        }
    }
}

/** What shows behind a row as it is swiped, so the gesture says what it will do. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SwipeHint(direction: SwipeToDismissBoxValue, textScale: Float) {
    val editing = direction == SwipeToDismissBoxValue.StartToEnd
    val colour = if (editing) {
        MaterialTheme.colorScheme.primary
    } else {
        MaterialTheme.colorScheme.error
    }

    Row(
        Modifier
            .fillMaxSize()
            .background(colour.copy(alpha = 0.3f))
            .padding(horizontal = 20.dp),
        horizontalArrangement = if (editing) Arrangement.Start else Arrangement.End,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = if (editing) Icons.Filled.Edit else Icons.Filled.Delete,
            contentDescription = null,
            tint = colour,
        )
        Text(
            text = if (editing) "Correct" else "Remove",
            color = colour,
            fontSize = (13 * textScale).sp,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.padding(start = 8.dp),
        )
    }
}

@Composable
private fun FieldRow(number: Int, field: ReadField, textScale: Float) {
    Row(
        Modifier
            .fillMaxWidth()
            // Opaque, so a row sliding across does not show the rows underneath
            // it through the panel's translucency.
            .background(Color(0xFF1A1A1A))
            .padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier
                .size(BADGE * textScale)
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.primary),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = "$number",
                color = MaterialTheme.colorScheme.onPrimary,
                fontSize = (12 * textScale).sp,
                fontWeight = FontWeight.Bold,
            )
        }
        Column(Modifier.weight(1f)) {
            Text(
                text = field.kind.label.uppercase() +
                    (
                        field.phoneKind?.takeIf { field.kind == CardFieldKind.PHONE }
                            ?.let { " · ${it.replaceFirstChar(Char::uppercase)}" } ?: ""
                        ),
                color = MaterialTheme.colorScheme.primary,
                fontSize = (13 * textScale).sp,
                fontWeight = FontWeight.Bold,
                letterSpacing = 1.sp,
            )
            Text(
                text = field.value,
                color = Color.White,
                fontSize = (19 * textScale).sp,
                fontWeight = FontWeight.SemiBold,
            )
        }
    }
}

/** Correcting one value the recogniser got wrong. */
@Composable
private fun CorrectField(
    field: ReadField,
    onDone: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var value by remember(field) { mutableStateOf(field.value) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Correct the ${field.kind.label.lowercase()}") },
        text = {
            OutlinedTextField(
                value = value,
                onValueChange = { value = it },
                label = { Text(field.kind.label) },
                singleLine = field.kind != CardFieldKind.NOTES,
                modifier = Modifier.fillMaxWidth(),
            )
        },
        confirmButton = {
            TextButton(
                onClick = { onDone(value.trim()) },
                enabled = value.isNotBlank(),
            ) { Text("Save") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** Room kept clear at the foot of the screen for Skip and Save. */
private val BOTTOM_BAR = 92.dp

/**
 * The photograph as the screen shows it, and as the recogniser saw it.
 *
 * Both are needed. Regions are in the coordinates ML Kit worked in, which is the
 * photograph at full size and turned the right way up; the bitmap here is smaller
 * than that on purpose. Mapping one to the other needs the size of both, and
 * getting it wrong puts every highlight somewhere plausible and false.
 */
private class Photo(val bitmap: Bitmap, val sourceWidth: Int, val sourceHeight: Int)

/**
 * Decode the photograph at something a screen can use, turned as the camera held
 * it.
 *
 * Two things have to agree with the recogniser or the markers land in the wrong
 * place, and both are silent when wrong:
 *
 * **Size.** A phone camera makes 12 megapixels or more and the review shows it a
 * screen's width wide, so it is decoded small — but the regions are in the full
 * image's pixels, so the full size is carried alongside rather than assumed.
 *
 * **Rotation.** `InputImage.fromFilePath` applies the EXIF orientation, so ML Kit
 * measured an upright image. `BitmapFactory` ignores EXIF entirely. A portrait
 * photo from the camera app is stored landscape with a "rotate 90" tag, so
 * without this the picture appears sideways and every region is against the wrong
 * axis — a marker would sit at the far edge of the card from its words.
 */
private fun loadPhoto(context: android.content.Context, uri: Uri): Photo? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    context.contentResolver.openInputStream(uri)?.use {
        BitmapFactory.decodeStream(it, null, bounds)
    }
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null

    var sample = 1
    while (maxOf(bounds.outWidth, bounds.outHeight) / sample > TARGET_LONG_EDGE) sample *= 2

    val decoded = context.contentResolver.openInputStream(uri)?.use {
        BitmapFactory.decodeStream(it, null, BitmapFactory.Options().apply { inSampleSize = sample })
    } ?: return null

    val quarterTurns = context.contentResolver.openInputStream(uri)?.use { stream ->
        runCatching {
            when (
                android.media.ExifInterface(stream)
                    .getAttributeInt(
                        android.media.ExifInterface.TAG_ORIENTATION,
                        android.media.ExifInterface.ORIENTATION_NORMAL,
                    )
            ) {
                android.media.ExifInterface.ORIENTATION_ROTATE_90 -> 90f
                android.media.ExifInterface.ORIENTATION_ROTATE_180 -> 180f
                android.media.ExifInterface.ORIENTATION_ROTATE_270 -> 270f
                else -> 0f
            }
        }.getOrDefault(0f)
    } ?: 0f

    val upright = if (quarterTurns == 0f) {
        decoded
    } else {
        val turn = android.graphics.Matrix().apply { postRotate(quarterTurns) }
        Bitmap.createBitmap(decoded, 0, 0, decoded.width, decoded.height, turn, true)
            .also { if (it != decoded) decoded.recycle() }
    }

    // The full-size dimensions *after* the same turn, which is the space the
    // recogniser reported its boxes in.
    val sideways = quarterTurns == 90f || quarterTurns == 270f
    return Photo(
        bitmap = upright,
        sourceWidth = if (sideways) bounds.outHeight else bounds.outWidth,
        sourceHeight = if (sideways) bounds.outWidth else bounds.outHeight,
    )
}

/** Pixels the long edge is decoded to. See [loadPhoto] on why size is carried. */
private const val TARGET_LONG_EDGE = 1600

/** The numbered marker, sized once so the photo and the panel agree. */
private val BADGE = 22.dp

/** How much of the screen the panel may take, leaving the rest for the card. */
private const val PANEL_SHARE = 0.45f

/**
 * What has been swiped away, and the way back.
 *
 * A destructive gesture with no undo is a trap: the swipe is quick, easy to make
 * by accident on a scrolling list, and there is no second chance before Save.
 * Rather than a snackbar that times out — and which sits badly inside a dialog —
 * the removals stay listed until the card is saved. Nothing is lost until the
 * whole screen is.
 */
@Composable
private fun Removed(
    removed: List<ReadField>,
    textScale: Float,
    onRestore: (ReadField) -> Unit,
) {
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        Text(
            text = "REMOVED — TAP TO PUT BACK",
            color = Color.White.copy(alpha = 0.55f),
            fontSize = (10 * textScale).sp,
            fontWeight = FontWeight.Bold,
            letterSpacing = 1.sp,
            modifier = Modifier.padding(bottom = 6.dp),
        )
        removed.forEach { field ->
            Surface(
                color = Color.White.copy(alpha = 0.08f),
                shape = RoundedCornerShape(8.dp),
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(bottom = 6.dp)
                    .clickable { onRestore(field) },
            ) {
                Row(
                    Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(
                        Icons.AutoMirrored.Filled.Undo,
                        contentDescription = "Put back the ${field.kind.label.lowercase()}",
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size((16 * textScale).dp),
                    )
                    Text(
                        text = "${field.kind.label}: ${field.value}",
                        color = Color.White.copy(alpha = 0.75f),
                        fontSize = (13 * textScale).sp,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(start = 8.dp),
                    )
                }
            }
        }
    }
}
