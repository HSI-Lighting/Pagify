//! Finding the signature in a photo of a signed page, so uploading one
//! brings in just the ink — not the whole page it was written on.
//!
//! # The problem this solves
//!
//! A photo of a signed line carries far more than the signature: the rest
//! of the page, ruled lines, whatever bled through from the sheet beneath,
//! the table it was photographed on. Someone who uploads that photo means
//! the signature, not the page — so this finds the pen ink and returns a
//! tight crop around it with everything else made transparent, rather than
//! painted over — see [`Extracted`] for why that matters downstream.
//!
//! # Why not one threshold
//!
//! A page like this has several distinct tones, not two — bright paper,
//! faint bleed-through from the sheet beneath, printed grey rule lines,
//! and the signature's own ink, which is not reliably the darkest thing on
//! the page (bold rule lines and bleed-through can be as dark as a light
//! pen stroke). A single global threshold (Otsu's method, tried first)
//! either lets the faint stuff in or cuts off real strokes. This uses two
//! thresholds instead: a strict one (the darkest sliver of the page) to
//! *locate* the signature with confidence, and a looser, locally-calibrated
//! one to recover the lighter parts of the same strokes once the rough
//! location is known.
//!
//! # Why not just flood-fill the loose threshold
//!
//! It was tried, and it bridges things a person would never call part of
//! the signature: a faint ruled line runs the width of the page, and at the
//! loose threshold, flood-fill treats it as one connected object with
//! whatever it happens to graze — bleed-through text well below the
//! signature, a shadow at the frame's edge. A chain of barely-qualifying
//! pixels can connect two things arbitrarily far apart even though no
//! single step looks unreasonable. Two changes fix it: ruled lines are
//! stripped out *before* flood-fill with a morphological opening (a line is
//! only a few pixels tall; a pen stroke is not, so eroding and re-growing
//! along the vertical axis erases the line but leaves the stroke), and
//! acceptance is judged by actual pixel proximity to the confidently-located
//! ink (dilated by a bounded radius) rather than by flood-fill reachability
//! or by bounding-box overlap, which is coarse enough to overlap two things
//! whose real pixels never come close.

use std::collections::VecDeque;

/// A signature lifted out of a larger photo: a tight crop around the ink,
/// with a real alpha channel rather than a background painted over —
/// background pixels carry alpha 0, ink pixels their own colour at full or
/// partial opacity, fading smoothly between.
///
/// **Why alpha, when the picture ends up opaque while merely placed.** The
/// mechanism a picture is *placed* through — see `pdf_core`'s
/// `Annotation::Image` — genuinely cannot carry alpha, so while a signature
/// sits there unapplied, it is composited pixel for pixel against this
/// page's own rendered pixels (`Session::place_image_signature`, at the
/// moment the destination is known — not here). But applying goes through a different,
/// lower-level path that hand-writes the PDF bytes and is not bound by that
/// limit — see `pdf_core`'s `DocumentMut::remember_image_alpha` — so the
/// real alpha extracted here does eventually reach the page as a genuine
/// soft mask, once the signature is applied, in the same session it was
/// placed in. Keeping the true background/ink distinction here, rather than
/// baking in one guess (white) up front, is what makes both of those
/// possible; deciding what either of them actually needs belongs to the
/// code that knows where the signature is going, not to this function.
pub struct Extracted {
    /// RGBA, row-major, top row first — same layout as
    /// [`crate::signatures::StoredImage`].
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Find the signature in `rgba` (row-major RGBA, `width`×`height`, as
/// decoded from an uploaded photo) and return a tight crop around it with
/// real alpha — see [`Extracted`]. `None` when nothing plausible enough to
/// be a signature was found — a blank page, or a photo of something else
/// entirely — in which case the caller should fall back to the picture as
/// uploaded rather than show nothing.
pub fn extract_signature(rgba: &[u8], width: u32, height: u32) -> Option<Extracted> {
    if width == 0 || height == 0 || (rgba.len() as u64) < (width as u64) * (height as u64) * 4 {
        return None;
    }
    let luma_buf: Vec<u8> = (0..(width as u64 * height as u64) as usize)
        .map(|i| luma(rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]))
        .collect();

    // A page photographed at an angle often has the sheet's own edge, or
    // whatever it's resting on, right at the frame's border — ignored by a
    // small inset so it never competes with the ink itself.
    let inset_x = ((width as f64 * 0.06) as u32).min(width - 1);
    let inset_y = ((height as f64 * 0.03) as u32).min(height - 1);
    let (x0, y0) = (inset_x, inset_y);
    let (iw, ih) = (width - 2 * inset_x, height - 2 * inset_y);

    let mut hist = [0u32; 256];
    for y in y0..y0 + ih {
        for x in x0..x0 + iw {
            hist[luma_buf[(y * width + x) as usize] as usize] += 1;
        }
    }
    // The darkest sliver of the page — bold enough that only real ink (or a
    // bold rule line, filtered out below by shape) is ever this dark.
    const STRICT_FRACTION: f64 = 0.006;
    let strict_threshold = percentile_threshold(&hist, STRICT_FRACTION);

    let mut strict_mask = vec![false; (iw * ih) as usize];
    for y in 0..ih {
        for x in 0..iw {
            strict_mask[(y * iw + x) as usize] = luma_buf[((y + y0) * width + (x + x0)) as usize] < strict_threshold;
        }
    }
    let (components, strict_labels) = connected_components(&strict_mask, iw, ih, &luma_buf, x0, y0, width);

    let min_area = ((iw * ih) as f64 * 0.00004).max(10.0) as u32;
    let keep: Vec<bool> = components.iter().map(|c| plausible_stroke(c, iw, ih, min_area)).collect();

    // Union-find: components within a small gap of each other belong to the
    // same mark — a signature is rarely pen-continuous letter to letter.
    let n = components.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], x: usize) -> usize {
        if parent[x] != x {
            parent[x] = find(parent, parent[x]);
        }
        parent[x]
    }
    const CLUSTER_GAP: i64 = 40;
    for i in 0..n {
        if !keep[i] {
            continue;
        }
        for j in (i + 1)..n {
            if !keep[j] {
                continue;
            }
            let (a, b) = (&components[i], &components[j]);
            let overlap_x =
                (a.min_x as i64 - CLUSTER_GAP) < b.max_x as i64 && (b.min_x as i64 - CLUSTER_GAP) < a.max_x as i64;
            let overlap_y =
                (a.min_y as i64 - CLUSTER_GAP) < b.max_y as i64 && (b.min_y as i64 - CLUSTER_GAP) < a.max_y as i64;
            if overlap_x && overlap_y {
                let (ra, rb) = (find(&mut parent, i), find(&mut parent, j));
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }

    // (left, top, right, bottom, area, luma_sum) per cluster.
    let mut clusters: std::collections::HashMap<usize, (u32, u32, u32, u32, u32, u64)> = std::collections::HashMap::new();
    for i in 0..n {
        if !keep[i] {
            continue;
        }
        let root = find(&mut parent, i);
        let c = &components[i];
        let entry = clusters.entry(root).or_insert((c.min_x, c.min_y, c.max_x, c.max_y, 0, 0));
        entry.0 = entry.0.min(c.min_x);
        entry.1 = entry.1.min(c.min_y);
        entry.2 = entry.2.max(c.max_x);
        entry.3 = entry.3.max(c.max_y);
        entry.4 += c.area;
        entry.5 += c.luma_sum;
    }

    // Area weighted by boldness (how far below the strict threshold the
    // average pixel sits) — printed grey text sits right at the cutoff,
    // firm ink well below it — breaking the near-tie between a short bold
    // word and a longer but fainter one better than area alone. A cluster
    // centred in the top eighth of the page is penalised, not excluded: a
    // letterhead or a printed "Name / Date" lives there; a signature on a
    // document essentially never does.
    let score = |v: &(u32, u32, u32, u32, u32, u64)| {
        let avg = v.5 as f64 / v.4 as f64;
        let boldness = (strict_threshold as f64 - avg).max(1.0);
        let centre_y = (v.1 + v.3) as f64 / 2.0;
        let header_zone = ih as f64 * 0.12;
        let header_penalty = if centre_y < header_zone { 0.15 } else { 1.0 };
        v.4 as f64 * boldness * header_penalty
    };
    let (&winning_root, &(ux0, uy0, ux1, uy1, ..)) =
        clusters.iter().max_by(|(_, a), (_, b)| score(a).total_cmp(&score(b)))?;

    // The confident core: strict-threshold pixels belonging to the winning
    // cluster. Everything from here on may only grow this, never shrink it.
    let roots: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
    let mut sig_mask = vec![false; (iw * ih) as usize];
    for (idx, &label) in strict_labels.iter().enumerate() {
        if label >= 0 && keep[label as usize] && roots[label as usize] == winning_root {
            sig_mask[idx] = true;
        }
    }

    // A padded window around the located cluster — small enough that a
    // bounded search stays cheap, generous enough to hold the signature's
    // lighter stroke ends and the ruled line immediately around it.
    let pad_x = ((ux1 - ux0) as f64 * 0.4).max(40.0) as u32;
    let pad_y = pad_x;
    let wx0 = ux0.saturating_sub(pad_x);
    let wy0 = uy0.saturating_sub(pad_y);
    let wx1 = (ux1 + pad_x).min(iw - 1);
    let wy1 = (uy1 + pad_y).min(ih - 1);
    let (ww, wh) = (wx1 - wx0 + 1, wy1 - wy0 + 1);

    let mut window_mask = vec![false; (ww * wh) as usize];
    for y in 0..wh {
        for x in 0..ww {
            window_mask[(y * ww + x) as usize] = sig_mask[((wy0 + y) * iw + (wx0 + x)) as usize];
        }
    }

    // A second, looser threshold, calibrated to the paper right around the
    // signature rather than the whole page, recovers the lighter, thinner
    // ends of a stroke the strict pass missed.
    let mut local_hist = [0u32; 256];
    for y in (y0 + wy0)..=(y0 + wy1) {
        for x in (x0 + wx0)..=(x0 + wx1) {
            local_hist[luma_buf[(y * width + x) as usize] as usize] += 1;
        }
    }
    let loose_threshold = percentile_threshold(&local_hist, 1.0 / 6.0).max(strict_threshold + 1);

    let mut loose_mask = vec![false; (ww * wh) as usize];
    for y in 0..wh {
        for x in 0..ww {
            loose_mask[(y * ww + x) as usize] =
                luma_buf[((y0 + wy0 + y) * width + (x0 + wx0 + x)) as usize] < loose_threshold;
        }
    }
    // Strip ruled lines (a few pixels tall, page-wide) before flood-fill can
    // use them to bridge the signature to anything else they cross.
    let loose_mask = open_vertical(&loose_mask, ww, wh, 5);
    let (loose_components, loose_labels) = connected_components(&loose_mask, ww, wh, &luma_buf, x0 + wx0, y0 + wy0, width);

    // Accept a loose component only if it is both shaped like ink (dense
    // within its own box — a bridged tangle of thin lines is mostly gap,
    // even when its box is large) and actually close to the confident core,
    // judged against the core's real pixel shape rather than its bounding
    // box, which two unrelated things can overlap on paper while staying
    // far apart along the diagonal.
    let window_min_area = ((ww * wh) as f64 * 0.00004).max(6.0) as u32;
    const SEARCH_RADIUS: u32 = 30;
    let search_region = dilate(&window_mask, ww, wh, SEARCH_RADIUS);
    let mut touches_search = vec![false; loose_components.len()];
    for (idx, &label) in loose_labels.iter().enumerate() {
        if label >= 0 && search_region[idx] {
            touches_search[label as usize] = true;
        }
    }
    let accepted_loose: Vec<bool> = loose_components
        .iter()
        .enumerate()
        .map(|(label, c)| {
            let bw = (c.max_x - c.min_x + 1) as f64;
            let bh = (c.max_y - c.min_y + 1) as f64;
            let fill_ratio = c.area as f64 / (bw * bh);
            c.area >= window_min_area && fill_ratio >= 0.06 && touches_search[label]
        })
        .collect();

    let mut grown_window = window_mask;
    for (idx, &label) in loose_labels.iter().enumerate() {
        if label >= 0 && accepted_loose[label as usize] {
            grown_window[idx] = true;
        }
    }
    // A small dilation so the blend below has a soft rind at each stroke's
    // edge instead of a hard pixel cliff.
    let grown_window = dilate(&grown_window, ww, wh, 2);

    let (mut bx0, mut by0, mut bx1, mut by1) = (ww, wh, 0u32, 0u32);
    let mut any = false;
    for y in 0..wh {
        for x in 0..ww {
            if grown_window[(y * ww + x) as usize] {
                any = true;
                bx0 = bx0.min(x);
                by0 = by0.min(y);
                bx1 = bx1.max(x);
                by1 = by1.max(y);
            }
        }
    }
    let (fx0, fy0, fx1, fy1) = if any {
        (x0 + wx0 + bx0, y0 + wy0 + by0, x0 + wx0 + bx1, y0 + wy0 + by1)
    } else {
        (x0 + ux0, y0 + uy0, x0 + ux1, y0 + uy1)
    };

    let margin_x = ((fx1 - fx0) as f64 * 0.12).max(12.0) as u32;
    let margin_y = ((fy1 - fy0) as f64 * 0.12).max(12.0) as u32;
    let cx0 = fx0.saturating_sub(margin_x);
    let cy0 = fy0.saturating_sub(margin_y);
    let cx1 = (fx1 + margin_x).min(width - 1);
    let cy1 = (fy1 + margin_y).min(height - 1);
    let (cw, ch) = (cx1 - cx0 + 1, cy1 - cy0 + 1);

    let mut out = vec![0u8; (cw * ch * 4) as usize];
    let lo = strict_threshold as f64;
    let hi = loose_threshold as f64;
    for y in 0..ch {
        for x in 0..cw {
            let ax = cx0 + x;
            let ay = cy0 + y;
            let wxr = ax as i64 - (x0 + wx0) as i64;
            let wyr = ay as i64 - (y0 + wy0) as i64;
            let in_mask = wxr >= 0
                && wyr >= 0
                && (wxr as u32) < ww
                && (wyr as u32) < wh
                && grown_window[(wyr as u32 * ww + wxr as u32) as usize];
            let src_idx = ((ay * width + ax) * 4) as usize;
            let out_idx = ((y * cw + x) * 4) as usize;
            // The pixel's own colour survives unchanged — this crop carries
            // real alpha now, not a paint-toward-white approximation, so
            // fading is alpha's job, not the colour channels'.
            out[out_idx..out_idx + 3].copy_from_slice(&rgba[src_idx..src_idx + 3]);
            out[out_idx + 3] = if in_mask {
                let v = luma_buf[(ay * width + ax) as usize] as f64;
                // 0 at or below the strict threshold (all ink, fully
                // opaque), 1 at or above the loose one (background, fully
                // transparent) — blended between so a stroke's edge fades
                // rather than ending in a hard cliff.
                let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
                (255.0 * (1.0 - t)).round() as u8
            } else {
                // Outside the grown mask — fully transparent no matter how
                // dark, which is what excludes a disconnected shadow or
                // thumb.
                0
            };
        }
    }
    Some(Extracted { rgba: out, width: cw, height: ch })
}

fn luma(r: u8, g: u8, b: u8) -> u8 {
    (0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64).round() as u8
}

/// The luma value below which the darkest `fraction` of pixels counted in
/// `hist` fall.
fn percentile_threshold(hist: &[u32; 256], fraction: f64) -> u8 {
    let total: u64 = hist.iter().map(|&c| c as u64).sum();
    let target = (total as f64 * fraction) as u64;
    let mut running = 0u64;
    for (v, &count) in hist.iter().enumerate() {
        running += count as u64;
        if running >= target {
            return v as u8;
        }
    }
    255
}

/// Separable max-dilation: grows `mask` by `radius` in every direction.
/// Unlike flood-filling a looser threshold over a wider area, a small
/// fixed-radius geometric grow cannot leap farther than `radius` — it
/// cannot bridge to a thumb, a shadow, or bleed-through text on the other
/// side of a gap, no matter how many faint qualifying pixels form a path
/// across it.
fn dilate(mask: &[bool], w: u32, h: u32, radius: u32) -> Vec<bool> {
    let mut rows = vec![false; mask.len()];
    for y in 0..h {
        for x in 0..w {
            let lo = x.saturating_sub(radius);
            let hi = (x + radius).min(w - 1);
            rows[(y * w + x) as usize] = (lo..=hi).any(|xx| mask[(y * w + xx) as usize]);
        }
    }
    let mut out = vec![false; mask.len()];
    for y in 0..h {
        let lo = y.saturating_sub(radius);
        let hi = (y + radius).min(h - 1);
        for x in 0..w {
            out[(y * w + x) as usize] = (lo..=hi).any(|yy| rows[(yy * w + x) as usize]);
        }
    }
    out
}

/// Erosion then dilation restricted to a single column (the vertical axis)
/// — a ruled line is page-wide but only a few pixels *tall*; a pen stroke
/// is much shorter across the page but far taller than that. Eroding
/// vertically then dilating back by the same radius (a vertical "opening")
/// erases anything that thin while leaving thicker strokes close to their
/// original shape.
fn open_vertical(mask: &[bool], w: u32, h: u32, radius: u32) -> Vec<bool> {
    let mut eroded = vec![false; mask.len()];
    for x in 0..w {
        for y in 0..h {
            let lo = y.saturating_sub(radius);
            let hi = (y + radius).min(h - 1);
            eroded[(y * w + x) as usize] = (lo..=hi).all(|yy| mask[(yy * w + x) as usize]);
        }
    }
    let mut out = vec![false; mask.len()];
    for x in 0..w {
        for y in 0..h {
            let lo = y.saturating_sub(radius);
            let hi = (y + radius).min(h - 1);
            out[(y * w + x) as usize] = (lo..=hi).any(|yy| eroded[(yy * w + x) as usize]);
        }
    }
    out
}

struct Component {
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
    area: u32,
    luma_sum: u64,
}

/// Rejects noise specks (too small), ruled lines and margin rules in either
/// orientation (too elongated), and anything spanning most of the region it
/// was found in (a page-wide mark, not a single stroke).
fn plausible_stroke(c: &Component, region_w: u32, region_h: u32, min_area: u32) -> bool {
    let bw = (c.max_x - c.min_x + 1) as f64;
    let bh = (c.max_y - c.min_y + 1) as f64;
    let elongation = bw.max(bh) / bw.min(bh);
    let too_big = bw > region_w as f64 * 0.4 || bh > region_h as f64 * 0.4;
    c.area >= min_area && elongation < 7.0 && !too_big
}

/// 8-connected components of `mask` (`w`×`h`), reading pixel darkness from
/// `luma_buf` at `(x0 + x, y0 + y)` with row stride `stride` — `luma_buf` is
/// usually a larger buffer that `mask` is a window onto. Returns each
/// component alongside a same-sized label buffer (component index per
/// pixel, -1 for background) so callers can build exact pixel masks rather
/// than trusting a component's bounding box.
fn connected_components(
    mask: &[bool],
    w: u32,
    h: u32,
    luma_buf: &[u8],
    x0: u32,
    y0: u32,
    stride: u32,
) -> (Vec<Component>, Vec<i32>) {
    let mut visited = vec![false; mask.len()];
    let mut labels = vec![-1i32; mask.len()];
    let mut out: Vec<Component> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            if !mask[idx] || visited[idx] {
                continue;
            }
            let label = out.len() as i32;
            let mut q = VecDeque::new();
            q.push_back((x, y));
            visited[idx] = true;
            let (mut min_x, mut min_y, mut max_x, mut max_y) = (x, y, x, y);
            let mut area = 0u32;
            let mut luma_sum = 0u64;
            while let Some((cx, cy)) = q.pop_front() {
                labels[(cy * w + cx) as usize] = label;
                area += 1;
                luma_sum += luma_buf[((cy + y0) * stride + (cx + x0)) as usize] as u64;
                min_x = min_x.min(cx);
                min_y = min_y.min(cy);
                max_x = max_x.max(cx);
                max_y = max_y.max(cy);
                for (nx, ny) in [
                    (cx.wrapping_sub(1), cy),
                    (cx + 1, cy),
                    (cx, cy.wrapping_sub(1)),
                    (cx, cy + 1),
                    // 8-connected: a pencil stroke a single pixel wide can
                    // step diagonally between rows and break 4-connectivity.
                    (cx.wrapping_sub(1), cy.wrapping_sub(1)),
                    (cx + 1, cy.wrapping_sub(1)),
                    (cx.wrapping_sub(1), cy + 1),
                    (cx + 1, cy + 1),
                ] {
                    if nx >= w || ny >= h {
                        continue;
                    }
                    let nidx = (ny * w + nx) as usize;
                    if mask[nidx] && !visited[nidx] {
                        visited[nidx] = true;
                        q.push_back((nx, ny));
                    }
                }
            }
            out.push(Component { min_x, min_y, max_x, max_y, area, luma_sum });
        }
    }
    (out, labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paints a `width`×`height` white canvas, everything opaque.
    fn white_canvas(width: u32, height: u32) -> Vec<u8> {
        vec![255u8; (width as usize) * (height as usize) * 4]
    }

    fn set_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, v: u8) {
        let i = ((y * width + x) * 4) as usize;
        rgba[i..i + 4].copy_from_slice(&[v, v, v, 255]);
    }

    /// Real ink is never one flat luma value — a pen stroke is darkest
    /// along its centre, lighter toward its edges. A uniformly flat
    /// synthetic rectangle has no such variation, so the darkest-N%
    /// percentile threshold lands exactly on the fill value and the `<`
    /// comparison excludes every pixel of it — a boundary case a real photo
    /// never hits, but a naive synthetic fixture does. A radial gradient
    /// (darkest at the centre, `rim` at the edge) sidesteps it cheaply and
    /// is closer to how a pen stroke actually looks: it gives every
    /// percentile threshold in range a solid, connected darkest region to
    /// find, not a single flat plateau.
    fn draw_gradient_blob(rgba: &mut [u8], width: u32, x0: u32, y0: u32, x1: u32, y1: u32, centre: u8, rim: u8) {
        let (cx, cy) = ((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0);
        let max_dist = (((x1 - x0) as f64 / 2.0).powi(2) + ((y1 - y0) as f64 / 2.0).powi(2)).sqrt().max(1.0);
        for y in y0..y1 {
            for x in x0..x1 {
                let dist = (((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt() / max_dist).min(1.0);
                let v = centre as f64 + dist * (rim as f64 - centre as f64);
                set_pixel(rgba, width, x, y, v.round() as u8);
            }
        }
    }

    /// A bold blob: dark enough at its centre for the strict pass to find
    /// with confidence. The centre-to-rim range is kept narrow so a large
    /// fraction of the blob's own radius — not just a small dot at its
    /// centre — ends up below whatever threshold the strict pass picks,
    /// which matters for tests that need the confident core to be roughly
    /// the size of the blob itself, not a speck at its middle.
    fn draw_ink_blob(rgba: &mut [u8], width: u32, x0: u32, y0: u32, x1: u32, y1: u32) {
        draw_gradient_blob(rgba, width, x0, y0, x1, y1, 5, 20);
    }

    /// A lighter mark: real ink, but too pale anywhere in it for the
    /// strict pass alone — only recoverable by the loose pass once
    /// something bold nearby has located the signature. Close to the bold
    /// blob's own rim value rather than dramatically fainter: the loose
    /// threshold is calibrated *from what is actually in the local
    /// window*, so it reaches a little past confidently-dark ink, not all
    /// the way to a barely-there value with a wide gap of nothing between.
    fn draw_faint_blob(rgba: &mut [u8], width: u32, x0: u32, y0: u32, x1: u32, y1: u32) {
        draw_gradient_blob(rgba, width, x0, y0, x1, y1, 25, 45);
    }

    /// A blank photo, however bright or shadowed, has no signature.
    #[test]
    fn a_blank_photo_finds_nothing() {
        let (w, h) = (400, 300);
        assert!(extract_signature(&white_canvas(w, h), w, h).is_none());
    }

    /// A single bold, book-shaped blob of ink — the simplest possible
    /// signature — is found, and the crop lands tightly around it rather
    /// than returning the whole photo.
    #[test]
    fn a_solid_dark_blob_is_found_and_cropped_tightly() {
        let (w, h) = (800, 600);
        let mut rgba = white_canvas(w, h);
        draw_ink_blob(&mut rgba, w, 300, 250, 500, 320);

        let extracted = extract_signature(&rgba, w, h).expect("a signature was drawn");
        assert!(extracted.width < w, "the crop should be smaller than the whole photo");
        assert!(extracted.height < h, "the crop should be smaller than the whole photo");
        // The blob's own pixels must have survived into the crop, dark.
        let any_dark = extracted
            .rgba
            .chunks_exact(4)
            .any(|p| p[0] < 100 && p[1] < 100 && p[2] < 100);
        assert!(any_dark, "the ink itself should still be dark in the crop");
    }

    /// The crop carries real alpha, not a background painted white: ink
    /// pixels come back opaque, and background — including inside the
    /// crop's own margin, which sits between the ink and the crop's edge —
    /// comes back transparent. Both must be present, since a crop that
    /// somehow ended up entirely one or the other would pass a check for
    /// either alone.
    #[test]
    fn the_crop_carries_real_alpha_not_a_white_background() {
        let (w, h) = (800, 600);
        let mut rgba = white_canvas(w, h);
        draw_ink_blob(&mut rgba, w, 300, 250, 500, 320);

        let extracted = extract_signature(&rgba, w, h).expect("a signature was drawn");
        let pixels: Vec<&[u8]> = extracted.rgba.chunks_exact(4).collect();
        assert!(pixels.iter().any(|p| p[3] > 200), "some pixel should be opaque ink");
        assert!(pixels.iter().any(|p| p[3] == 0), "the crop's own margin should be transparent");
    }

    /// A faint pencil stroke immediately beside a bold ink blob is
    /// recovered by the loose pass rather than clipped off at the strict
    /// threshold — the crop ends up noticeably wider than the bold part
    /// alone would produce.
    #[test]
    fn a_lighter_stroke_next_to_bold_ink_is_recovered() {
        let (w, h) = (200, 150);

        let mut bold_only = white_canvas(w, h);
        draw_ink_blob(&mut bold_only, w, 60, 50, 90, 75);
        let bold_extracted = extract_signature(&bold_only, w, h).expect("bold ink alone is found");

        let mut with_tail = white_canvas(w, h);
        draw_ink_blob(&mut with_tail, w, 60, 50, 90, 75);
        // A lighter tail immediately beside it — too light for the strict
        // pass alone, but real ink, not noise.
        draw_faint_blob(&mut with_tail, w, 90, 55, 130, 70);
        let tail_extracted = extract_signature(&with_tail, w, h).expect("ink with a lighter tail is found");

        assert!(
            tail_extracted.width > bold_extracted.width + 15,
            "the lighter tail should have widened the crop: bold-only {} vs with-tail {}",
            bold_extracted.width,
            tail_extracted.width
        );
    }

    /// A ruled line running the width of the photo, near an ink blob but
    /// never touching it, must not pull the whole line into the crop, and
    /// a dark blob far from the ink (a shadow, a thumb) must not survive
    /// into the crop even though it is dark enough to otherwise qualify.
    #[test]
    fn a_ruled_line_and_a_distant_dark_blob_are_excluded() {
        let (w, h) = (1200, 900);
        let mut rgba = white_canvas(w, h);
        // The signature: bold and comfortably the largest dark thing on the
        // page, so cluster-scoring picks it over the distant blob below.
        draw_ink_blob(&mut rgba, w, 500, 400, 750, 470);
        // A ruled line the width of the photo, run right through it, as a
        // real signature line would.
        for x in 0..w {
            set_pixel(&mut rgba, w, x, 435, 170);
        }
        // A dark, distant blob near the top-left corner — both far from the
        // signature and, being in the header zone, scored down even if it
        // were not: nothing about it should end up in the final crop.
        draw_ink_blob(&mut rgba, w, 0, 0, 100, 100);

        let extracted = extract_signature(&rgba, w, h).expect("a signature was drawn");
        assert!(
            extracted.width < 500,
            "the ruled line must not have bridged the crop out toward the distant blob, got width {}",
            extracted.width
        );
    }

    /// Too small, or too oddly shaped, to panic on: extraction on a tiny
    /// image either finds nothing or returns cleanly — it never crashes.
    #[test]
    fn tiny_and_degenerate_images_do_not_panic() {
        for (w, h) in [(1, 1), (2, 3), (10, 500), (500, 10)] {
            let rgba = white_canvas(w, h);
            let _ = extract_signature(&rgba, w, h);
        }
        assert!(extract_signature(&[], 0, 0).is_none());
    }
}
