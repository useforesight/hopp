//! Per-window texture bookkeeping. A `RenderImage` lives in each window's sprite atlas
//! once painted, so every window releases the frames it has moved past.
use std::{collections::HashMap, sync::Arc};

use gpui_kit::{RenderImage, Window};

#[derive(Default)]
pub struct Painted {
    images: HashMap<String, Arc<RenderImage>>,
    seen: Vec<String>,
}

impl Painted {
    /// Record that `key` now shows `image`, releasing whatever it showed before.
    pub fn show(&mut self, key: &str, image: &Arc<RenderImage>, window: &mut Window) {
        self.seen.push(key.to_owned());
        match self.images.get(key) {
            Some(previous) if Arc::ptr_eq(previous, image) => {}
            _ => {
                if let Some(previous) = self.images.insert(key.to_owned(), image.clone()) {
                    let _ = window.drop_image(previous);
                }
            }
        }
    }

    /// Release images for keys not shown during this render.
    pub fn finish(&mut self, window: &mut Window) {
        let seen = std::mem::take(&mut self.seen);
        let stale: Vec<_> = self
            .images
            .keys()
            .filter(|key| !seen.contains(key))
            .cloned()
            .collect();
        for key in stale {
            if let Some(image) = self.images.remove(&key) {
                let _ = window.drop_image(image);
            }
        }
    }
}
