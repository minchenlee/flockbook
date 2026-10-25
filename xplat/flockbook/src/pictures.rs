//! Bounded raster decoding and view-owned thumbnails. Rows never own decoded images.
use herdr_core::ImageRef;
use iced::widget::image::Handle;
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub const CACHE_BYTES: usize = 64 << 20;
pub const CACHE_COUNT: usize = 120;
// Serial decode bounds the temporary full raster allocation even across pane switches.
static DECODE_GATE: Mutex<()> = Mutex::new(());
static THUMB_BYTES: AtomicUsize = AtomicUsize::new(0);
static THUMB_COUNT: AtomicUsize = AtomicUsize::new(0);
static PREVIEW_BYTES: AtomicUsize = AtomicUsize::new(0);

// The budget follows the pixel owner, including handle clones kept by a renderer.
struct BudgetedPixels {
    rgba: Vec<u8>,
    preview: bool,
    #[cfg(test)]
    dropped: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
impl AsRef<[u8]> for BudgetedPixels {
    fn as_ref(&self) -> &[u8] {
        &self.rgba
    }
}
impl Drop for BudgetedPixels {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(flag) = &self.dropped {
            flag.store(true, Ordering::SeqCst);
        }
        if self.preview {
            PREVIEW_BYTES.fetch_sub(self.rgba.len(), Ordering::SeqCst);
        } else {
            THUMB_BYTES.fetch_sub(self.rgba.len(), Ordering::SeqCst);
            THUMB_COUNT.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[derive(Debug, Clone)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl Decoded {
    pub fn handle(self) -> Handle {
        Handle::from_rgba(self.width, self.height, self.rgba)
    }
    pub fn budgeted_handle(self, preview: bool) -> Option<Handle> {
        let cost = self.rgba.len();
        let (bytes, limit) = if preview {
            (&PREVIEW_BYTES, 128usize << 20)
        } else {
            (&THUMB_BYTES, CACHE_BYTES)
        };
        bytes
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                n.checked_add(cost).filter(|total| *total <= limit)
            })
            .ok()?;
        if !preview
            && THUMB_COUNT
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                    (n < CACHE_COUNT).then_some(n + 1)
                })
                .is_err()
        {
            bytes.fetch_sub(cost, Ordering::SeqCst);
            return None;
        }
        let owner = BudgetedPixels {
            rgba: self.rgba,
            preview,
            #[cfg(test)]
            dropped: None,
        };
        Some(Handle::from_rgba(
            self.width,
            self.height,
            iced::advanced::graphics::core::Bytes::from_owner(owner),
        ))
    }
    pub fn cost(&self) -> usize {
        self.rgba.len()
    }
}

/// Shared with paste previews. Applies codec allocation limits before decoding; never fetches a URL.
pub fn decode(bytes: &[u8], max_pixel: u32) -> Option<Decoded> {
    let _gate = DECODE_GATE.lock().ok()?;
    decode_inner(bytes, max_pixel)
}

fn decode_inner(bytes: &[u8], max_pixel: u32) -> Option<Decoded> {
    herdr_core::log_image::header(bytes)?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(192 << 20);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    if img.width() as u64 * img.height() as u64 > herdr_core::log_image::MAX_PIXELS {
        return None;
    }
    let side = max_pixel.clamp(1, 4096);
    let rgba = if max_pixel == 0 {
        img.to_rgba8()
    } else {
        img.thumbnail(side, side).to_rgba8()
    };
    Some(Decoded {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

pub fn load(path: &Path, r: &ImageRef, max_pixel: u32) -> Option<Decoded> {
    let _gate = DECODE_GATE.lock().ok()?;
    decode_inner(&herdr_core::log_image::read(r, path)?, max_pixel)
}

pub fn fit(width: u32, height: u32, maxw: f32, maxh: f32) -> (f32, f32) {
    let k = 1f32
        .min(maxw / width.max(1) as f32)
        .min(maxh / height.max(1) as f32);
    (
        (width as f32 * k).round().max(1.0),
        (height as f32 * k).round().max(1.0),
    )
}

#[derive(Default)]
pub struct Pictures {
    pub path: Option<PathBuf>,
    pub refs: HashMap<u64, ImageRef>,
    pub near: HashSet<u64>,
    pub handles: HashMap<u64, Handle>,
    costs: HashMap<u64, usize>,
    pub pending: HashSet<u64>,
    pub failed: HashSet<u64>,
    pub preview: Option<Handle>,
    pub preview_id: Option<u64>,
    pub preview_seq: u64,
    pub load_seq: u64,
}
impl Pictures {
    pub fn prune_costs(&mut self) {
        self.costs.retain(|id, _| self.handles.contains_key(id));
    }
    pub fn clear_visible(&mut self) {
        self.load_seq = self.load_seq.wrapping_add(1);
        self.pending.clear();
        self.failed.clear();
        self.near.clear();
        self.handles.clear();
        self.costs.clear();
        self.preview = None;
        self.preview_id = None;
        self.preview_seq = self.preview_seq.wrapping_add(1);
    }
    pub fn retain_near(&mut self, ids: Vec<u64>) {
        // Even square 960 px RGBA thumbnails fit the byte limit without eviction/reload loops.
        let slots = CACHE_COUNT.min(CACHE_BYTES / (960 * 960 * 4));
        self.near = ids.into_iter().take(slots).collect();
        self.handles.retain(|id, _| self.near.contains(id));
        self.costs.retain(|id, _| self.handles.contains_key(id));
        self.failed.retain(|id| self.near.contains(id));
    }
    pub fn insert(&mut self, id: u64, d: Decoded) {
        if !self.near.contains(&id) {
            return;
        }
        let cost = d.cost();
        if cost > CACHE_BYTES {
            return;
        }
        while self.handles.len() >= CACHE_COUNT
            || self.costs.values().sum::<usize>() + cost > CACHE_BYTES
        {
            let Some(old) = self.handles.keys().copied().next() else {
                break;
            };
            self.handles.remove(&old);
            self.costs.remove(&old);
        }
        if let Some(handle) = d.budgeted_handle(false) {
            self.handles.insert(id, handle);
            self.costs.insert(id, cost);
        } else {
            self.failed.insert(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_decode_and_fixed_layout() {
        assert!(decode(b"not an image", 480).is_none());
        assert_eq!(fit(2000, 1000, 480., 640.), (480., 240.));
    }
    #[test]
    fn leaving_view_drops_the_last_state_handle() {
        let mut p = Pictures::default();
        p.retain_near(vec![1]);
        p.insert(
            1,
            Decoded {
                width: 1,
                height: 1,
                rgba: vec![0; 4],
            },
        );
        assert_eq!(p.handles.len(), 1);
        p.retain_near(vec![]);
        assert!(p.handles.is_empty());
        assert!(p.costs.is_empty());
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn many_visible_refs_cannot_grow_handle_count_without_bound() {
        let mut p = Pictures::default();
        p.retain_near((0..500).collect());
        for id in 0..500 {
            p.insert(
                id,
                Decoded {
                    width: 1,
                    height: 1,
                    rgba: vec![0; 4],
                },
            );
        }
        assert!(p.handles.len() <= CACHE_COUNT);
        assert!(p.costs.values().sum::<usize>() <= CACHE_BYTES);
        assert_eq!(p.handles.len(), CACHE_BYTES / (960 * 960 * 4));
        let seq = p.load_seq;
        p.clear_visible();
        assert!(p.handles.is_empty());
        assert_ne!(p.load_seq, seq);
    }
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    #[test]
    fn pixel_budget_follows_the_last_renderer_handle_clone() {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        THUMB_BYTES.fetch_add(4, Ordering::SeqCst);
        THUMB_COUNT.fetch_add(1, Ordering::SeqCst);
        let owner = BudgetedPixels {
            rgba: vec![0; 4],
            preview: false,
            dropped: Some(flag.clone()),
        };
        let bytes = iced::advanced::graphics::core::Bytes::from_owner(owner);
        let handle = Handle::from_rgba(1, 1, bytes);
        let renderer_copy = handle.clone();
        drop(handle);
        assert!(!flag.load(Ordering::SeqCst));
        drop(renderer_copy);
        assert!(flag.load(Ordering::SeqCst));
    }
}
