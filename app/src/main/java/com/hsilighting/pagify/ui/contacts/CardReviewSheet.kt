package com.hsilighting.pagify.ui.contacts

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.util.Log
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
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
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.net.toUri
import com.hsilighting.pagify.core.CardReading
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
 * The photograph is dimmed, each thing that was read is ringed on it, and the
 * four values are set out below in type big enough to check at arm's length.
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
    onSave: () -> Unit,
    onSkip: () -> Unit,
) {
    val context = LocalContext.current
    var photo by remember(imageUri) { mutableStateOf<Photo?>(null) }

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
        Column(
            Modifier
                .fillMaxSize()
                .background(Color(0xFF101010)),
        ) {
            Text(
                text = if (total > 1) "Card ${position + 1} of $total" else "Check the card",
                style = MaterialTheme.typography.titleMedium,
                color = Color.White,
                modifier = Modifier.padding(start = 20.dp, top = 20.dp, bottom = 4.dp),
            )
            Text(
                text = "Read from the photograph — tap Save to keep it, or edit it after.",
                style = MaterialTheme.typography.bodySmall,
                color = Color.White.copy(alpha = 0.7f),
                modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 12.dp),
            )

            Box(
                Modifier
                    .weight(1f)
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp),
                contentAlignment = Alignment.Center,
            ) {
                val loaded = photo
                if (loaded == null) {
                    Text("Opening the photograph…", color = Color.White.copy(alpha = 0.7f))
                } else {
                    HighlightedPhoto(loaded, reading)
                }
            }


            ReadFields(reading)

            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(20.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TextButton(onClick = onSkip, modifier = Modifier.weight(1f)) {
                    // "Skip", not "Cancel": with several cards in one photograph
                    // this passes over one and goes on to the next.
                    Text(if (total > 1) "Skip this one" else "Discard")
                }
                Button(onClick = onSave, modifier = Modifier.weight(1f)) { Text("Save") }
            }
        }
    }
}
/**
 * The photograph, dimmed, with a marker round each thing that was read.
 *
 * The values themselves are **not** drawn on the picture, and the first version
 * that did was the argument against it: four labelled values placed at their own
 * lines landed on top of each other and on the card's own text, because the lines
 * of a business card are a few millimetres apart and readable type is not. It was
 * less legible than the card.
 *
 * So the picture carries numbered markers and nothing else, and the numbers are
 * repeated beside the values below. The link is explicit rather than spatial,
 * which is what survives a card whose lines are close together.
 *
 * The markers are placed into the rectangle the image actually occupies, not the
 * container's — `ContentScale.Fit` leaves bars on one axis, and a marker measured
 * against the container drifts further from its words the more the two aspect
 * ratios differ.
 */
@Composable
private fun HighlightedPhoto(photo: Photo, reading: CardReading) {
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val boxWidth = constraints.maxWidth.toFloat()
        val boxHeight = constraints.maxHeight.toFloat()
        val bitmap = photo.bitmap
        val shown = min(boxWidth / bitmap.width, boxHeight / bitmap.height)
        val shownWidth = bitmap.width * shown
        val shownHeight = bitmap.height * shown
        // Regions are in the full photograph and the bitmap is a smaller copy of
        // it: this is the factor that takes one to the other, and it is not
        // `shown`. Deriving it from the decoded bitmap instead would move every
        // marker by the downsampling factor.
        val scale = if (photo.sourceWidth > 0) shownWidth / photo.sourceWidth else shown
        val offsetX = (boxWidth - shownWidth) / 2f
        val offsetY = (boxHeight - shownHeight) / 2f

        val density = LocalDensity.current
        val accent = MaterialTheme.colorScheme.primary

        Image(
            bitmap = bitmap.asImageBitmap(),
            contentDescription = "The card that was photographed",
            contentScale = ContentScale.Fit,
            modifier = Modifier.fillMaxSize(),
        )

        // Dim everything, then let the markers sit above it. Heavy enough that
        // the picture reads as background; a light scrim leaves both competing.
        Box(
            Modifier
                .fillMaxSize()
                .background(Color.Black.copy(alpha = 0.55f)),
        )

        reading.highlights.forEachIndexed { index, field ->
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
                //
                // Tucked back inside when the line runs close to the frame edge,
                // where a badge beyond it would simply be clipped away.
                val badgeAfter = (offsetX + region.right * scale).toDp() + 4.dp
                val fits = badgeAfter + BADGE < (offsetX + shownWidth).toDp()

                Box(
                    Modifier
                        .offset(
                            x = if (fits) badgeAfter else badgeAfter - BADGE - 8.dp,
                            y = (offsetY + region.top * scale).toDp() +
                                (((region.height * scale).toDp() - BADGE) / 2f),
                        )
                        .size(BADGE)
                        .clip(CircleShape)
                        .background(accent),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = "${index + 1}",
                        color = MaterialTheme.colorScheme.onPrimary,
                        fontSize = 12.sp,
                        fontWeight = FontWeight.Bold,
                    )
                }
            }
        }
    }
}

/**
 * What was read, in type big enough to check at arm's length.
 *
 * Numbered to match the markers on the photograph. This is where the values
 * actually get read — the picture above says *where they came from*, which is the
 * question somebody asks only when one of them looks wrong.
 */
@Composable
private fun ReadFields(reading: CardReading) {
    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 20.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (reading.highlights.isEmpty()) {
            Text(
                "Nothing could be read off this card.",
                color = Color.White.copy(alpha = 0.7f),
            )
            return@Column
        }

        reading.highlights.forEachIndexed { index, field ->
            Row(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    Modifier
                        .size(BADGE)
                        .clip(CircleShape)
                        .background(MaterialTheme.colorScheme.primary),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = "${index + 1}",
                        color = MaterialTheme.colorScheme.onPrimary,
                        fontSize = 12.sp,
                        fontWeight = FontWeight.Bold,
                    )
                }
                Column(Modifier.weight(1f)) {
                    Text(
                        text = field.label.uppercase(),
                        color = MaterialTheme.colorScheme.primary,
                        fontSize = 11.sp,
                        fontWeight = FontWeight.Bold,
                        letterSpacing = 1.sp,
                    )
                    Text(
                        text = field.value,
                        color = Color.White,
                        fontSize = 22.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                }
            }
        }
    }
}

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
 * Two things have to agree with the recogniser or the highlights land in the
 * wrong place, and both are silent when wrong:
 *
 * **Size.** A phone camera makes 12 megapixels or more and the review shows it a
 * screen's width wide, so it is decoded small — but the regions are in the full
 * image's pixels, so the full size is carried alongside rather than assumed.
 *
 * **Rotation.** `InputImage.fromFilePath` applies the EXIF orientation, so ML Kit
 * measured an upright image. `BitmapFactory` ignores EXIF entirely. A portrait
 * photo from the camera app is stored landscape with a "rotate 90" tag, so
 * without this the picture appears sideways and every region is against the wrong
 * axis — a highlight would sit at the far edge of the card from its words.
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

/** The numbered marker, sized once so the photo and the list below agree. */
private val BADGE = 22.dp
