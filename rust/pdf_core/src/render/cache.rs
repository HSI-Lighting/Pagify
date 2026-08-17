//! LRU cache of rasterised pages.
//!
//! Budgeted in **bytes, not entries**: a thumbnail and a 4x-zoomed A0 page differ
//! by three orders of magnitude, so a count-based limit either wastes memory or
//! thrashes depending on which the user happens to be looking at.
//!
//! The cache exists to make *prefetching* worthwhile. On-screen rendering draws
//! straight into a locked Android bitmap and never touches this. A background
//! prefetch of page N+1 lands here, so when the user swipes, the page arrives via
//! a memcpy (single-digit ms) instead of a re-render (tens to hundreds of ms).

use std::collections::HashMap;

use crate::render::bitmap::Bitmap;

/// Zoom is quantised before it reaches the key: a pinch gesture produces a
/// continuum of float scales, and an unquantised key would make every frame a miss
/// while filling the cache with near-duplicates.
pub const ZOOM_QUANTUM: f32 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub page_index: usize,
    /// Quantised zoom. Integer so the key can be `Hash + Eq`.
    pub zoom_step: u32,
    pub rotation_quarter_turns: u8,
}

impl CacheKey {
    pub fn new(page_index: usize, zoom: f32, rotation_quarter_turns: u8) -> Self {
        CacheKey {
            page_index,
            zoom_step: quantise_zoom(zoom),
            rotation_quarter_turns: rotation_quarter_turns % 4,
        }
    }

    /// The zoom this key actually represents, which is what a cache-filling render
    /// must be performed at for the stored bitmap to be reusable.
    pub fn effective_zoom(&self) -> f32 {
        self.zoom_step as f32 * ZOOM_QUANTUM
    }
}

/// Rounds *up* so the cached raster is never lower-resolution than requested —
/// scaling a bitmap down on the GPU is free and sharp, scaling up is neither.
pub fn quantise_zoom(zoom: f32) -> u32 {
    let clamped = zoom.max(ZOOM_QUANTUM);
    let steps = (clamped / ZOOM_QUANTUM).ceil();
    steps.max(1.0) as u32
}

pub struct PageCache {
    budget_bytes: usize,
    used_bytes: usize,
    entries: HashMap<CacheKey, Bitmap>,
    /// Least-recently-used first. Small (tens of entries at most), so the linear
    /// scans below are cheaper than maintaining an intrusive list.
    recency: Vec<CacheKey>,
    hits: u64,
    misses: u64,
}

impl PageCache {
    pub fn new(budget_bytes: usize) -> Self {
        PageCache {
            budget_bytes,
            used_bytes: 0,
            entries: HashMap::new(),
            recency: Vec::new(),
            hits: 0,
            misses: 0,
        }
    }

    pub fn get(&mut self, key: &CacheKey) -> Option<&Bitmap> {
        if self.entries.contains_key(key) {
            self.touch(key);
            self.hits += 1;
            // Re-looked-up rather than held across `touch` to satisfy the borrow checker.
            self.entries.get(key)
        } else {
            self.misses += 1;
            None
        }
    }

    pub fn contains(&self, key: &CacheKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Insert, evicting least-recently-used entries until the budget is met.
    ///
    /// A bitmap larger than the whole budget is dropped rather than stored, so a
    /// single huge page cannot evict everything else and then be evicted itself.
    pub fn put(&mut self, key: CacheKey, bitmap: Bitmap) {
        let size = bitmap.byte_len();
        if size > self.budget_bytes {
            log::debug!(
                "page {} at zoom step {} needs {} bytes, over the {} byte budget; not cached",
                key.page_index,
                key.zoom_step,
                size,
                self.budget_bytes
            );
            return;
        }

        if let Some(old) = self.entries.remove(&key) {
            self.used_bytes -= old.byte_len();
            self.recency.retain(|k| k != &key);
        }

        while self.used_bytes + size > self.budget_bytes {
            if !self.evict_one() {
                break;
            }
        }

        self.used_bytes += size;
        self.entries.insert(key, bitmap);
        self.recency.push(key);
    }

    /// Drop every entry for one page, at any zoom. Used when a page's content
    /// changes — the hook the editing phase needs.
    pub fn invalidate_page(&mut self, page_index: usize) {
        let doomed: Vec<CacheKey> = self
            .entries
            .keys()
            .filter(|k| k.page_index == page_index)
            .copied()
            .collect();
        for key in doomed {
            self.remove(&key);
        }
    }

    pub fn remove(&mut self, key: &CacheKey) {
        if let Some(bitmap) = self.entries.remove(key) {
            self.used_bytes -= bitmap.byte_len();
            self.recency.retain(|k| k != key);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.recency.clear();
        self.used_bytes = 0;
    }

    pub fn set_budget(&mut self, budget_bytes: usize) {
        self.budget_bytes = budget_bytes;
        while self.used_bytes > self.budget_bytes {
            if !self.evict_one() {
                break;
            }
        }
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.entries.len(),
            used_bytes: self.used_bytes,
            budget_bytes: self.budget_bytes,
        }
    }

    fn touch(&mut self, key: &CacheKey) {
        if let Some(pos) = self.recency.iter().position(|k| k == key) {
            let key = self.recency.remove(pos);
            self.recency.push(key);
        }
    }

    fn evict_one(&mut self) -> bool {
        if self.recency.is_empty() {
            return false;
        }
        let victim = self.recency.remove(0);
        if let Some(bitmap) = self.entries.remove(&victim) {
            self.used_bytes -= bitmap.byte_len();
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub used_bytes: usize,
    pub budget_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::bitmap::PixelOrder;

    /// ~`px*px*4` bytes, so tests can reason about the budget directly.
    fn bitmap_of(px: u32) -> Bitmap {
        Bitmap::new(px, px, PixelOrder::Rgba).unwrap()
    }

    #[test]
    fn quantised_zoom_rounds_up_so_cached_pages_are_never_too_blurry() {
        assert_eq!(quantise_zoom(1.0), 4); // 4 * 0.25
        assert_eq!(quantise_zoom(1.01), 5); // rounds up, not to nearest
        assert_eq!(quantise_zoom(0.25), 1);
    }

    #[test]
    fn tiny_and_zero_zooms_clamp_to_the_smallest_step() {
        assert_eq!(quantise_zoom(0.0), 1);
        assert_eq!(quantise_zoom(0.0001), 1);
        assert_eq!(quantise_zoom(-3.0), 1);
    }

    #[test]
    fn nearby_pinch_zooms_collapse_onto_one_key() {
        let a = CacheKey::new(0, 1.60, 0);
        let b = CacheKey::new(0, 1.74, 0);
        assert_eq!(a, b, "a pinch gesture must not miss on every frame");
        assert_eq!(a.effective_zoom(), 1.75);
    }

    #[test]
    fn rotation_and_page_are_part_of_the_identity() {
        assert_ne!(CacheKey::new(0, 1.0, 0), CacheKey::new(1, 1.0, 0));
        assert_ne!(CacheKey::new(0, 1.0, 0), CacheKey::new(0, 1.0, 1));
        assert_eq!(
            CacheKey::new(0, 1.0, 4),
            CacheKey::new(0, 1.0, 0),
            "quarter turns wrap"
        );
    }

    #[test]
    fn hits_and_misses_are_counted() {
        let mut cache = PageCache::new(1024 * 1024);
        let key = CacheKey::new(0, 1.0, 0);

        assert!(cache.get(&key).is_none());
        cache.put(key, bitmap_of(4));
        assert!(cache.get(&key).is_some());

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn eviction_removes_the_least_recently_used_entry_first() {
        // Budget fits exactly two 10x10 bitmaps (400 bytes each).
        let mut cache = PageCache::new(800);
        let (a, b, c) = (
            CacheKey::new(0, 1.0, 0),
            CacheKey::new(1, 1.0, 0),
            CacheKey::new(2, 1.0, 0),
        );

        cache.put(a, bitmap_of(10));
        cache.put(b, bitmap_of(10));
        cache.get(&a); // `a` is now the most recent, so `b` is the victim.
        cache.put(c, bitmap_of(10));

        assert!(cache.contains(&a), "recently used entry survived");
        assert!(!cache.contains(&b), "least recently used entry evicted");
        assert!(cache.contains(&c));
        assert_eq!(cache.used_bytes(), 800);
    }

    #[test]
    fn used_bytes_tracks_insertions_evictions_and_removals() {
        let mut cache = PageCache::new(10_000);
        cache.put(CacheKey::new(0, 1.0, 0), bitmap_of(10)); // 400
        cache.put(CacheKey::new(1, 1.0, 0), bitmap_of(20)); // 1600
        assert_eq!(cache.used_bytes(), 2000);

        cache.remove(&CacheKey::new(0, 1.0, 0));
        assert_eq!(cache.used_bytes(), 1600);

        cache.clear();
        assert_eq!(cache.used_bytes(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn reinserting_the_same_key_does_not_double_count_memory() {
        let mut cache = PageCache::new(10_000);
        let key = CacheKey::new(0, 1.0, 0);
        cache.put(key, bitmap_of(10));
        cache.put(key, bitmap_of(10));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.used_bytes(), 400, "the old entry's bytes were reclaimed");
    }

    #[test]
    fn a_bitmap_larger_than_the_whole_budget_is_refused_without_evicting() {
        let mut cache = PageCache::new(1000);
        let keeper = CacheKey::new(0, 1.0, 0);
        cache.put(keeper, bitmap_of(10)); // 400 bytes

        cache.put(CacheKey::new(1, 8.0, 0), bitmap_of(100)); // 40_000 bytes

        assert!(
            cache.contains(&keeper),
            "an unstorable page must not flush the cache on its way to being dropped"
        );
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn invalidating_a_page_drops_every_zoom_level_of_it_only() {
        let mut cache = PageCache::new(100_000);
        cache.put(CacheKey::new(3, 1.0, 0), bitmap_of(10));
        cache.put(CacheKey::new(3, 2.0, 0), bitmap_of(10));
        cache.put(CacheKey::new(4, 1.0, 0), bitmap_of(10));

        cache.invalidate_page(3);

        assert_eq!(cache.len(), 1);
        assert!(cache.contains(&CacheKey::new(4, 1.0, 0)));
        assert_eq!(cache.used_bytes(), 400, "byte accounting survives invalidation");
    }

    #[test]
    fn shrinking_the_budget_evicts_down_to_the_new_limit() {
        let mut cache = PageCache::new(10_000);
        for page in 0..5 {
            cache.put(CacheKey::new(page, 1.0, 0), bitmap_of(10)); // 400 each
        }
        assert_eq!(cache.used_bytes(), 2000);

        cache.set_budget(900);

        assert!(cache.used_bytes() <= 900);
        assert_eq!(cache.len(), 2);
    }
}
