//! Getting a rendered page into the shape a recogniser wants.
//!
//! Three steps, and their order matters. Greyscale first because everything
//! after works on one channel; deskew second because contrast measured on a
//! tilted page is measured across lines rather than along them; contrast last
//! so it operates on the pixels the recogniser will actually see.
//!
//! Deskew earns its place ahead of anything else here. A two-degree tilt —
//! which is nothing, and is what a page photographed on a desk looks like —
//! measurably degrades line detection, because a detector looking for
//! horizontal runs of ink finds each line smeared across several rows.

use crate::ocr::GreyImage;

/// RGBA → greyscale, using the standard luma weights.
///
/// Not a plain average: the eye is far more sensitive to green than to blue,
/// and averaging turns coloured text — a heading, a table rule, a watermark —
/// into a grey that no longer matches its contrast against the paper.
pub fn to_grey(pixels: &[u8], width: u32, height: u32) -> GreyImage {
    let count = width as usize * height as usize;
    let mut data = Vec::with_capacity(count);

    for i in 0..count {
        let at = i * 4;
        if at + 2 >= pixels.len() {
            data.push(0xFF);
            continue;
        }
        let (r, g, b) = (pixels[at] as f32, pixels[at + 1] as f32, pixels[at + 2] as f32);
        data.push((0.299 * r + 0.587 * g + 0.114 * b).round().clamp(0.0, 255.0) as u8);
    }

    GreyImage { width, height, data }
}

/// How far the text on this page is off the horizontal, in radians.
///
/// By projection profile: rotate the page through a range of candidate angles
/// and, for each, sum the ink in every row. When the page is straight those
/// sums are extreme — dense rows where the lines are, empty rows between them —
/// so the *variance* of the profile peaks at the correct angle. Tilt it and the
/// lines smear across rows, flattening the profile.
///
/// Cheap because it never actually rotates the image: it samples along tilted
/// rows instead.
pub fn estimate_skew(image: &GreyImage) -> f32 {
    if image.is_empty() || image.height < 8 {
        return 0.0;
    }

    // ±5°, which covers a page put down carelessly on a scanner or held in a
    // hand. Beyond that it is not skew, it is the wrong orientation, and
    // guessing a rotation from a skew estimate would be worse than not trying.
    const LIMIT_DEGREES: f32 = 5.0;
    const STEPS: i32 = 41;

    let mut best_angle = 0.0f32;
    let mut best_score = f32::MIN;

    for step in 0..STEPS {
        let degrees = -LIMIT_DEGREES + (step as f32) * (2.0 * LIMIT_DEGREES / (STEPS - 1) as f32);
        let score = profile_variance(image, degrees.to_radians());
        if score > best_score {
            best_score = score;
            best_angle = degrees.to_radians();
        }
    }

    best_angle
}

/// The variance of the row-ink profile when the page is read at `angle`.
fn profile_variance(image: &GreyImage, angle: f32) -> f32 {
    let slope = angle.tan();
    let mut rows = vec![0f32; image.height as usize];

    // Every fourth column is plenty: the profile is a statistic, and sampling
    // it costs a quarter as much for an answer that does not move.
    let step = 4u32.max(image.width / 512);

    for y in 0..image.height {
        let mut ink = 0f32;
        let mut x = 0u32;
        while x < image.width {
            let shifted = y as f32 + slope * x as f32;
            if shifted >= 0.0 && shifted < image.height as f32 {
                ink += 255.0 - image.get(x, shifted as u32) as f32;
            }
            x += step;
        }
        rows[y as usize] = ink;
    }

    let mean = rows.iter().sum::<f32>() / rows.len() as f32;
    rows.iter().map(|r| (r - mean).powi(2)).sum::<f32>() / rows.len() as f32
}

/// Rotate a page by `angle` radians about its centre, sampling nearest-neighbour.
///
/// Nearest-neighbour rather than bilinear on purpose: interpolation softens
/// stroke edges, and a recogniser trained on crisp glyphs reads a blurred one
/// worse than a slightly ragged one.
pub fn rotate(image: &GreyImage, angle: f32) -> GreyImage {
    if image.is_empty() || angle.abs() < 1e-4 {
        return image.clone();
    }

    let (sin, cos) = angle.sin_cos();
    let (cx, cy) = (image.width as f32 / 2.0, image.height as f32 / 2.0);
    let mut out = GreyImage::white(image.width, image.height);

    for y in 0..image.height {
        for x in 0..image.width {
            // Sampled backwards — for each destination pixel, ask where it came
            // from. Going forwards leaves unfilled holes wherever rounding
            // skips a destination.
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let sx = cx + dx * cos + dy * sin;
            let sy = cy - dx * sin + dy * cos;

            if sx >= 0.0 && sy >= 0.0 && sx < image.width as f32 && sy < image.height as f32 {
                out.set(x, y, image.get(sx as u32, sy as u32));
            }
        }
    }
    out
}

/// Straighten a page, if it needs it.
pub fn deskew(image: &GreyImage) -> (GreyImage, f32) {
    let angle = estimate_skew(image);
    if angle.abs() < 0.002 {
        // A tenth of a degree. Rotating for that costs a full resample and buys
        // nothing but softer edges.
        return (image.clone(), 0.0);
    }
    (rotate(image, -angle), angle)
}

/// Stretch the tonal range so the palest ink is black and the paper is white.
///
/// Clipped at the 2nd and 98th percentiles rather than the true extremes: one
/// speck of dust or one dark staple hole would otherwise define the whole
/// range, and the actual text would be compressed into the middle of it.
pub fn normalise_contrast(image: &mut GreyImage) {
    if image.data.is_empty() {
        return;
    }

    let mut histogram = [0u32; 256];
    for value in &image.data {
        histogram[*value as usize] += 1;
    }

    let total = image.data.len() as u32;
    let cut = (total as f32 * 0.02) as u32;

    let percentile = |from_dark: bool| -> u8 {
        let mut seen = 0u32;
        if from_dark {
            for (value, count) in histogram.iter().enumerate() {
                seen += count;
                if seen > cut {
                    return value as u8;
                }
            }
            0
        } else {
            for (value, count) in histogram.iter().enumerate().rev() {
                seen += count;
                if seen > cut {
                    return value as u8;
                }
            }
            255
        }
    };

    let low = percentile(true);
    let high = percentile(false);

    // A page that is genuinely flat — blank, or a solid block — has nothing to
    // stretch, and stretching it would amplify noise into text.
    if high <= low || (high - low) < 16 {
        return;
    }

    let span = (high - low) as f32;
    for value in &mut image.data {
        let stretched = (*value as f32 - low as f32) / span * 255.0;
        *value = stretched.clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page with `lines` horizontal bars of ink on it.
    fn ruled_page(width: u32, height: u32, lines: u32) -> GreyImage {
        let mut image = GreyImage::white(width, height);
        let spacing = height / (lines + 1);
        for line in 1..=lines {
            let y = line * spacing;
            for row in y..(y + 3).min(height) {
                for x in (width / 10)..(width * 9 / 10) {
                    image.set(x, row, 0);
                }
            }
        }
        image
    }

    #[test]
    fn greyscale_uses_luma_weights_not_a_flat_average() {
        // Pure green and pure blue are very different to the eye, and averaging
        // makes them identical — which loses the contrast that tells coloured
        // text from its paper.
        let green = to_grey(&[0, 255, 0, 255], 1, 1);
        let blue = to_grey(&[0, 0, 255, 255], 1, 1);

        assert!(green.data[0] > blue.data[0] + 100, "green {} blue {}", green.data[0], blue.data[0]);
        assert_eq!(to_grey(&[255, 255, 255, 255], 1, 1).data[0], 255);
        assert_eq!(to_grey(&[0, 0, 0, 255], 1, 1).data[0], 0);
    }

    #[test]
    fn a_short_buffer_gives_white_rather_than_panicking() {
        // Guards the boundary where a caller's stride assumption is wrong.
        let image = to_grey(&[0, 0, 0, 255], 4, 4);
        assert_eq!(image.data.len(), 16);
        assert_eq!(image.data[15], 0xFF);
    }

    #[test]
    fn a_straight_page_is_reported_as_straight() {
        let page = ruled_page(400, 400, 12);
        let angle = estimate_skew(&page);
        assert!(angle.abs().to_degrees() < 0.5, "found {}° on a straight page", angle.to_degrees());
    }

    #[test]
    fn a_tilted_page_is_measured_and_straightened() {
        // Two degrees — nothing to look at, and enough to smear every line
        // across several rows of the detector's profile.
        let straight = ruled_page(400, 400, 12);
        let tilted = rotate(&straight, 2.0f32.to_radians());

        let found = estimate_skew(&tilted).to_degrees();
        assert!(
            (found - 2.0).abs() < 1.0,
            "measured {found}° on a page tilted 2°"
        );

        let (fixed, corrected) = deskew(&tilted);
        assert!(corrected.abs() > 0.0, "deskew declined to correct a 2° tilt");
        assert!(
            estimate_skew(&fixed).abs().to_degrees() < 1.0,
            "the page is still {}° off after straightening",
            estimate_skew(&fixed).to_degrees()
        );
    }

    #[test]
    fn a_page_barely_off_true_is_left_alone() {
        // Resampling costs sharpness. A tenth of a degree is not worth it.
        let page = ruled_page(400, 400, 12);
        let (out, corrected) = deskew(&page);
        assert_eq!(corrected, 0.0);
        assert_eq!(out, page, "an already-straight page was resampled anyway");
    }

    #[test]
    fn contrast_stretches_a_flat_scan_to_the_full_range() {
        // A grey scan: ink at 90, paper at 170. Nothing near black or white.
        let mut image = GreyImage::white(100, 100);
        for (i, value) in image.data.iter_mut().enumerate() {
            *value = if i % 3 == 0 { 90 } else { 170 };
        }

        normalise_contrast(&mut image);
        let low = *image.data.iter().min().unwrap();
        let high = *image.data.iter().max().unwrap();

        assert!(low < 40, "the ink is still pale: {low}");
        assert!(high > 215, "the paper is still grey: {high}");
    }

    #[test]
    fn one_speck_of_dust_does_not_define_the_range() {
        // Clipping at the true extremes would let a single black pixel set the
        // floor and compress the text into the top of the range.
        let mut image = GreyImage::white(100, 100);
        for (i, value) in image.data.iter_mut().enumerate() {
            *value = if i % 3 == 0 { 120 } else { 200 };
        }
        image.data[0] = 0; // the speck

        normalise_contrast(&mut image);
        let ink = image.data[3];
        assert!(ink < 60, "the text was left compressed by one dark pixel: {ink}");
    }

    #[test]
    fn a_blank_page_is_not_amplified_into_noise() {
        // Stretching a flat page turns sensor noise into something a detector
        // will happily find text in.
        let mut blank = GreyImage::white(50, 50);
        let before = blank.clone();
        normalise_contrast(&mut blank);
        assert_eq!(blank, before, "a blank page was contrast-stretched");
    }

    #[test]
    fn an_empty_image_is_handled_everywhere() {
        let empty = GreyImage { width: 0, height: 0, data: Vec::new() };
        assert_eq!(estimate_skew(&empty), 0.0);
        assert_eq!(rotate(&empty, 0.5), empty);
        let mut e = empty.clone();
        normalise_contrast(&mut e);
    }
}
