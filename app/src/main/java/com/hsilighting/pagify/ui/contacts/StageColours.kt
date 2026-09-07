package com.hsilighting.pagify.ui.contacts

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.data.db.DealStage

/**
 * A colour per stage, so a list of deals can be read without reading it.
 *
 * The stage was a word in the second line of the row, in the same grey as the
 * company name and separated from it by a dot — which is to say it was not
 * visible at all. Down a day's worth of rows you could not see that four of them
 * were quoted and one was won without reading every one.
 *
 * **The hues run in the order the pipeline does** rather than being six colours
 * pulled out of a hat: grey for untouched, blue once contacted, amber for a
 * meeting in the diary, violet once a number is out, then the two endings in the
 * two colours endings have. Somebody who has never been told the scheme still
 * reads green as good and red as not.
 *
 * Not [MaterialTheme]'s scheme, which has three accent slots and needs six here,
 * and not one hue at six lightnesses, which would put the whole distinction on a
 * difference some people cannot see. These are picked in pairs against each
 * ground so the text keeps its contrast in both.
 */
internal data class StagePalette(val container: Color, val ink: Color)

@Composable
internal fun stagePalette(stage: DealStage): StagePalette {
    // Asked of the scheme in force rather than of the system setting: a theme
    // pinned light on a dark phone would otherwise get the dark pairs and the
    // badges would be six dark blocks on a white row.
    val dark = MaterialTheme.colorScheme.surface.luminance() < 0.5f
    return if (dark) {
        when (stage) {
            DealStage.New -> StagePalette(Color(0xFF2E343D), Color(0xFFC3CBD6))
            DealStage.Contacted -> StagePalette(Color(0xFF1B3350), Color(0xFFA9CBF3))
            DealStage.Meeting -> StagePalette(Color(0xFF4A3712), Color(0xFFF0C878))
            DealStage.Quoted -> StagePalette(Color(0xFF362A4D), Color(0xFFCDB8F0))
            DealStage.Won -> StagePalette(Color(0xFF1D3D28), Color(0xFFA6DCB6))
            DealStage.Lost -> StagePalette(Color(0xFF4A2320), Color(0xFFF2B4AE))
        }
    } else {
        when (stage) {
            DealStage.New -> StagePalette(Color(0xFFE4E7EC), Color(0xFF3F4854))
            DealStage.Contacted -> StagePalette(Color(0xFFDCE9FB), Color(0xFF1D4E86))
            DealStage.Meeting -> StagePalette(Color(0xFFFBEBD0), Color(0xFF7A5210))
            DealStage.Quoted -> StagePalette(Color(0xFFE8DFF7), Color(0xFF5A3B92))
            DealStage.Won -> StagePalette(Color(0xFFD8EFDC), Color(0xFF1E5B32))
            DealStage.Lost -> StagePalette(Color(0xFFF8DEDC), Color(0xFF8A2E27))
        }
    }
}

/**
 * The stage as a pill.
 *
 * Every stage gets one, New included. Hiding the quiet ones would leave the
 * right-hand edge of the list ragged and make "nothing has happened yet" look
 * the same as a row that had not loaded — so New is shown, in the colour of
 * nothing having happened yet.
 */
@Composable
internal fun StageBadge(stage: DealStage, modifier: Modifier = Modifier) {
    val palette = stagePalette(stage)
    Text(
        text = stage.label,
        style = MaterialTheme.typography.labelSmall,
        fontWeight = FontWeight.SemiBold,
        color = palette.ink,
        modifier = modifier
            .clip(RoundedCornerShape(6.dp))
            .background(palette.container)
            .padding(horizontal = 8.dp, vertical = 3.dp),
    )
}
