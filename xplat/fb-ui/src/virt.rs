//! Adapted from rmdv (MIT).
//!
//! Window math for a virtualised list. Every item (here: a message block) has
//! an estimated height. Measured heights are layered on top as sparse deltas
//! (`BTreeMap`), so a measurement never rebuilds the O(n) prefix table.
//!
//! Geometry contract with the view: items are laid out in one column with
//! `BLOCK_GAP_PX` between them, so the top of item `k` sits at `prefix[k]`
//! px from the top of the column. The top and bottom spacers come from the same
//! prefix sums, which keeps the scrollbar stable while only a window of items
//! exists as widgets.

use std::collections::BTreeMap;

/// Gap between consecutive items (the column spacing).
pub const BLOCK_GAP_PX: f32 = 8.0;

/// Lists with at most this many blocks render in full (no windowing).
pub const VIRT_MIN_BLOCKS: usize = 256;

/// Fold the sparse deltas into the prefix table when there are more than this.
const COMPACT_AT: usize = 1024;

/// Render pad above and below the viewport: `max(1.5 * viewport, 1200px)`.
pub fn pad_px(viewport_h: f32) -> f32 {
    (viewport_h * 1.5).max(1200.0)
}

#[derive(Clone, Debug)]
pub struct VirtWindow {
    /// `prefix[k]` = top of item `k`; `prefix[len]` = end of the last item
    /// plus one trailing gap. Estimate based.
    prefix: Vec<f32>,
    /// Sparse measured-height corrections, keyed by item index.
    adjustments: BTreeMap<usize, f32>,
    /// Rendered slice: `[range.0, range.1)`.
    pub range: (usize, usize),
    /// Scroll band (px) in which `range` stays valid (hysteresis).
    pub band: (f32, f32),
    /// False = small list, everything is rendered, no spacers.
    pub active: bool,
    /// Blocks needed before windowing engages.
    min_blocks: usize,
}

impl Default for VirtWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtWindow {
    pub fn new() -> Self {
        Self::with_min_blocks(VIRT_MIN_BLOCKS)
    }

    /// `usize::MAX` disables windowing (baseline mode).
    pub fn with_min_blocks(min_blocks: usize) -> Self {
        Self {
            prefix: vec![0.0],
            adjustments: BTreeMap::new(),
            range: (0, 0),
            band: (f32::NEG_INFINITY, f32::INFINITY),
            active: false,
            min_blocks,
        }
    }

    pub fn from_estimates(estimates: &[f32]) -> Self {
        let mut w = Self::new();
        for h in estimates {
            w.push(*h);
        }
        w
    }

    pub fn len(&self) -> usize {
        self.prefix.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn adjustment_count(&self) -> usize {
        self.adjustments.len()
    }

    /// Append one item with an estimated height. The caller re-windows after.
    pub fn push(&mut self, estimate: f32) {
        let last = *self.prefix.last().expect("prefix is never empty");
        self.prefix.push(last + estimate + BLOCK_GAP_PX);
    }

    /// Keep the first `len` items (and their measured corrections); drop the rest. The caller re-windows after.
    pub fn truncate(&mut self, len: usize) {
        if len >= self.len() {
            return;
        }
        self.prefix.truncate(len + 1);
        self.adjustments.retain(|i, _| *i < len);
    }

    fn adjustment_before(&self, index: usize) -> f32 {
        self.adjustments.range(..index).map(|(_, d)| *d).sum()
    }

    /// Top of item `index` (or the end of the list for `index == len`).
    pub fn prefix_at(&self, index: usize) -> f32 {
        let i = index.min(self.len());
        self.prefix[i] + self.adjustment_before(i)
    }

    /// Record a measured height as a delta against the current height.
    pub fn apply_height_delta(&mut self, index: usize, delta: f32) {
        if delta.abs() <= f32::EPSILON || index >= self.len() {
            return;
        }
        let entry = self.adjustments.entry(index).or_insert(0.0);
        *entry += delta;
        if entry.abs() <= 0.001 {
            self.adjustments.remove(&index);
        }
        if self.adjustments.len() > COMPACT_AT {
            self.compact();
        }
    }

    /// Fold all sparse deltas into the prefix table. Positions do not change.
    pub fn compact(&mut self) {
        if self.adjustments.is_empty() {
            return;
        }
        let mut running = 0.0f32;
        let mut deltas = std::mem::take(&mut self.adjustments).into_iter().peekable();
        for (k, slot) in self.prefix.iter_mut().enumerate() {
            while let Some((_, d)) = deltas.next_if(|(i, _)| *i < k) {
                running += d;
            }
            *slot += running;
        }
    }

    pub fn block_top(&self, index: usize) -> f32 {
        self.prefix_at(index)
    }

    /// Height of item `index` without the trailing gap.
    pub fn block_height(&self, index: usize) -> f32 {
        if index >= self.len() {
            return 0.0;
        }
        (self.prefix_at(index + 1) - self.prefix_at(index) - BLOCK_GAP_PX).max(0.0)
    }

    /// Total column height (every item plus a gap between neighbours).
    pub fn total_height(&self) -> f32 {
        (self.prefix_at(self.len()) - BLOCK_GAP_PX).max(0.0)
    }

    /// Spacer standing in for items above the window; with the column's gap
    /// after it, the first rendered item lands at `prefix[range.0]`.
    pub fn top_spacer(&self) -> Option<f32> {
        if !self.active || self.range.0 == 0 {
            return None;
        }
        Some((self.prefix_at(self.range.0) - BLOCK_GAP_PX).max(0.0))
    }

    /// Spacer standing in for items below the window.
    pub fn bottom_spacer(&self) -> Option<f32> {
        if !self.active || self.range.1 >= self.len() {
            return None;
        }
        let tail = self.prefix_at(self.len()) - self.prefix_at(self.range.1);
        Some((tail - BLOCK_GAP_PX).max(0.0))
    }

    /// True when `offset_y` left the band the current window was built for.
    pub fn needs_rebuild(&self, offset_y: f32) -> bool {
        self.active && (offset_y < self.band.0 || offset_y > self.band.1)
    }

    /// Item whose span contains `offset_y`.
    pub fn index_at(&self, offset_y: f32) -> Option<usize> {
        if self.is_empty() {
            return None;
        }
        let k = partition_point(self.len(), |i| self.prefix_at(i + 1) <= offset_y);
        Some(k.min(self.len() - 1))
    }

    /// Re-window around `offset_y` (px of the viewport top). Returns true when
    /// the rendered range changed.
    pub fn rebuild_at(&mut self, offset_y: f32, viewport_h: f32) -> bool {
        let before = (self.range, self.active);
        let len = self.len();
        self.active = len > self.min_blocks;
        if !self.active {
            self.range = (0, len);
            self.band = (f32::NEG_INFINITY, f32::INFINITY);
            return before != (self.range, self.active);
        }
        let vh = viewport_h.max(400.0);
        let pad = pad_px(vh);
        let offset_y = offset_y.clamp(0.0, self.total_height());
        let lo = offset_y - pad;
        let hi = offset_y + vh + pad;
        let start = partition_point(len, |i| self.prefix_at(i + 1) < lo);
        let end = partition_point(len, |i| self.prefix_at(i) <= hi);
        let start = start.min(len);
        self.range = (start, end.max(start).min(len));
        self.band = (offset_y - pad * 0.5, offset_y + pad * 0.5);
        before != (self.range, self.active)
    }

    /// Does the rendered range cover `[offset, offset + viewport_h]`?
    pub fn covers(&self, offset_y: f32, viewport_h: f32) -> bool {
        let (s, e) = self.range;
        let top_ok = s == 0 || self.prefix_at(s) <= offset_y + 0.5;
        let bottom_ok = e >= self.len() || self.prefix_at(e) >= offset_y + viewport_h - 0.5;
        top_ok && bottom_ok
    }
}

/// First index in `0..len` where `predicate` is false (predicate is monotone).
fn partition_point(len: usize, mut predicate: impl FnMut(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, len);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if predicate(mid) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(n: usize, h: f32) -> VirtWindow {
        VirtWindow::from_estimates(&vec![h; n])
    }

    #[test]
    fn prefix_sums_add_gaps() {
        let w = VirtWindow::from_estimates(&[10.0, 20.0, 30.0]);
        assert_eq!(w.prefix_at(0), 0.0);
        assert_eq!(w.prefix_at(1), 10.0 + BLOCK_GAP_PX);
        assert_eq!(w.prefix_at(2), 30.0 + 2.0 * BLOCK_GAP_PX);
        assert_eq!(w.total_height(), 60.0 + 2.0 * BLOCK_GAP_PX);
        assert_eq!(w.block_height(1), 20.0);
    }

    #[test]
    fn small_list_renders_fully() {
        let mut w = uniform(10, 40.0);
        w.rebuild_at(0.0, 800.0);
        assert!(!w.active);
        assert_eq!(w.range, (0, 10));
        assert_eq!((w.top_spacer(), w.bottom_spacer()), (None, None));
        assert!(!w.needs_rebuild(1e9));
    }

    #[test]
    fn range_for_offsets_covers_viewport() {
        let mut w = uniform(1000, 40.0);
        for off in [0.0, 1234.0, 20_000.0, 47_000.0, 1e9] {
            w.rebuild_at(off, 900.0);
            let clamped = off.clamp(0.0, w.total_height());
            assert!(w.active);
            assert!(w.covers(clamped, 900.0), "offset {off} range {:?}", w.range);
            assert!(w.range.1 - w.range.0 < 200, "window must stay small");
        }
    }

    #[test]
    fn spacers_plus_window_equal_total() {
        let mut w = uniform(1000, 40.0);
        w.rebuild_at(20_000.0, 800.0);
        let (s, e) = w.range;
        let top = w.top_spacer().unwrap();
        let bottom = w.bottom_spacer().unwrap();
        assert!((top + BLOCK_GAP_PX - w.prefix_at(s)).abs() < 0.01);
        let assembled = top + BLOCK_GAP_PX + (w.prefix_at(e) - w.prefix_at(s)) + bottom;
        assert!((assembled - w.total_height()).abs() < 0.5);
    }

    #[test]
    fn measured_delta_shifts_later_items_only() {
        let mut w = uniform(300, 40.0);
        w.apply_height_delta(5, 100.0);
        assert_eq!(w.prefix_at(5), 5.0 * (40.0 + BLOCK_GAP_PX));
        assert_eq!(w.prefix_at(6), 6.0 * (40.0 + BLOCK_GAP_PX) + 100.0);
        assert_eq!(w.block_height(5), 140.0);
        assert_eq!(w.adjustment_count(), 1);
        // The opposite delta cancels the entry.
        w.apply_height_delta(5, -100.0);
        assert_eq!(w.adjustment_count(), 0);
    }

    #[test]
    fn compaction_keeps_positions() {
        let mut w = uniform(5000, 30.0);
        for i in (0..4000).step_by(2) {
            w.apply_height_delta(i, (i % 7) as f32 + 0.5);
        }
        let probe: Vec<f32> = (0..=5000).step_by(97).map(|i| w.prefix_at(i)).collect();
        let before_total = w.total_height();
        w.compact();
        assert_eq!(w.adjustment_count(), 0);
        let after: Vec<f32> = (0..=5000).step_by(97).map(|i| w.prefix_at(i)).collect();
        for (a, b) in probe.iter().zip(&after) {
            assert!((a - b).abs() < 0.5, "{a} vs {b}");
        }
        assert!((before_total - w.total_height()).abs() < 0.5);
    }

    #[test]
    fn hysteresis_small_scrolls_do_not_rebuild() {
        let mut w = uniform(1000, 40.0);
        let off = 20_000.0;
        w.rebuild_at(off, 900.0);
        let pad = pad_px(900.0);
        assert!(!w.needs_rebuild(off));
        assert!(!w.needs_rebuild(off + 100.0));
        assert!(!w.needs_rebuild(off - pad * 0.4));
        assert!(w.needs_rebuild(off + pad * 0.6));
        assert!(w.needs_rebuild(0.0));
        // Inside the band the window still covers the viewport.
        let edge = off + pad * 0.49;
        assert!(w.covers(edge, 900.0));
    }

    #[test]
    fn pad_is_max_of_viewport_multiple_and_floor() {
        assert_eq!(pad_px(400.0), 1200.0);
        assert_eq!(pad_px(2000.0), 3000.0);
    }

    #[test]
    fn index_at_maps_offsets() {
        let w = uniform(100, 40.0);
        assert_eq!(w.index_at(0.0), Some(0));
        assert_eq!(w.index_at(w.prefix_at(5) + 1.0), Some(5));
        assert_eq!(w.index_at(1e9), Some(99));
    }

    #[test]
    fn push_after_adjustments_stays_consistent() {
        let mut w = uniform(400, 40.0);
        w.apply_height_delta(10, 25.0);
        let total = w.total_height();
        w.push(50.0);
        assert!((w.total_height() - (total + BLOCK_GAP_PX + 50.0)).abs() < 0.01);
    }
}
