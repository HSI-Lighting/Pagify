package com.hsilighting.pagify

import android.graphics.Bitmap
import com.hsilighting.pagify.core.CardFieldKind
import com.hsilighting.pagify.core.CardRegion
import com.hsilighting.pagify.core.ReadField
import com.hsilighting.pagify.ui.contacts.Photo
import com.hsilighting.pagify.ui.contacts.cropAround
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * `cropAround`: does it show the card, or the desk it was lying on?
 *
 * A `Bitmap` is needed to build a `Photo`, which is why this runs on-device
 * rather than as a JVM unit test — the stub `android.jar` throws for the calls
 * a real one makes even though nothing here draws anything.
 */
class CardCropTest {

    private fun photo(width: Int, height: Int) =
        Photo(Bitmap.createBitmap(4, 4, Bitmap.Config.ARGB_8888), width, height)

    private fun field(region: CardRegion) =
        IndexedValue(0, ReadField(CardFieldKind.NAME, "x", region))

    private fun region(left: Float, top: Float, right: Float, bottom: Float) =
        CardRegion(left, top, right, bottom)

    /** A card's worth of fields, clustered the way a real card's fields are. */
    private fun theCard() = listOf(
        field(region(400f, 380f, 900f, 430f)),   // name
        field(region(400f, 440f, 850f, 480f)),   // title
        field(region(400f, 500f, 950f, 540f)),   // company
        field(region(400f, 560f, 700f, 600f)),   // phone
    )

    @Test
    fun aTightClusterOfFieldsCropsCloseToThem() {
        val crop = cropAround(theCard(), photo(2000, 2000))

        // The card's own extent is x:400..950, y:380..600 — width 550, height 220.
        // cropAround pads by 12% of that extent on each side (CROP_MARGIN), so the
        // crop should be close to, not far past, the card's own footprint.
        assertTrue("crop too tight: width ${crop.width}", crop.width >= 550f)
        assertTrue("crop too wide for a clean card: width ${crop.width}", crop.width < 550f * 1.5f)
        assertTrue("crop too tight: height ${crop.height}", crop.height >= 220f)
        assertTrue("crop too tall for a clean card: height ${crop.height}", crop.height < 220f * 1.5f)
    }

    @Test
    fun noRegionsFallsBackToTheWholePhoto() {
        val noRegion = listOf(IndexedValue(0, ReadField(CardFieldKind.NOTES, "from a QR", region = null)))

        val crop = cropAround(noRegion, photo(1234, 5678))

        assertTrue(crop.width == 1234f && crop.height == 5678f)
    }

    /**
     * **Documents a known, not-yet-fixed gap — see the comment on `cropAround`.**
     *
     * `split_cards` has a confirmed failure where a fragment from outside a card
     * is attributed to it (`real_cards.rs`,
     * `a_single_card_is_not_split_in_two`). If that ever reaches this function —
     * one of the card's `ReadField`s carrying a region that actually belongs to
     * something else in the photograph — `cropAround`'s unconditional min/max
     * envelope stretches to cover it.
     *
     * This is not a hypothetical distance: the stray region sits where "a card
     * beside something that is not a card" would put unrelated text, a few card
     * widths away, not a token's width away.
     *
     * Not asserted as a bug to fix — asserted as what happens today, so a change
     * to `cropAround` is visible whichever direction it goes. See the function's
     * doc comment for why it stays this way until Phase B or a captured card
     * settles it.
     */
    @Test
    fun aStrayRegionFarFromTheCardBalloonsTheCrop() {
        val withStray = theCard() + field(region(1700f, 1750f, 1900f, 1800f))

        val real = cropAround(theCard(), photo(2000, 2000))
        val withOutlier = cropAround(withStray, photo(2000, 2000))

        assertTrue(
            "the stray field should have widened the crop, but width went from " +
                "${real.width} to ${withOutlier.width}",
            withOutlier.width > real.width * 2f,
        )
        assertTrue(
            "the stray field should have widened the crop, but height went from " +
                "${real.height} to ${withOutlier.height}",
            withOutlier.height > real.height * 2f,
        )

        // What that means on screen: the true card's own footprint as a fraction
        // of the area now being shown. A healthy crop keeps this close to 1; here
        // it demonstrates how small the real card gets squeezed to.
        val cardArea = real.width * real.height
        val shownArea = withOutlier.width * withOutlier.height
        assertTrue(
            "expected the real card to be squeezed to a small fraction of the " +
                "shown area; it was ${cardArea / shownArea}",
            cardArea / shownArea < 0.25f,
        )
    }
}
