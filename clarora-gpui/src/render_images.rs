//! An Arc dropping its CPU pixels does not evict GPUI's uploaded GPU texture.
use gpui::{App, RenderImage, Window};
use std::sync::Arc;

#[derive(Default)]
pub struct RenderedImages {
    current: Vec<Arc<RenderImage>>,
}

impl RenderedImages {
    /// Call before rendering the next scene, with all images that scene may use.
    /// Deduplication matters: GPUI's atlas removal must occur once per image.
    pub fn retain(
        &mut self,
        images: impl IntoIterator<Item = Arc<RenderImage>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut next: Vec<Arc<RenderImage>> = Vec::new();
        for image in images {
            if !next.iter().any(|current| current.id == image.id) {
                next.push(image);
            }
        }
        for image in self.current.drain(..) {
            if !next.iter().any(|current| current.id == image.id) {
                cx.drop_image(image, Some(window));
            }
        }
        self.current = next;
    }

    pub fn clear(&mut self, cx: &mut App) {
        for image in self.current.drain(..) {
            cx.drop_image(image, None);
        }
    }

    pub fn len(&self) -> usize {
        self.current.len()
    }

    pub fn is_empty(&self) -> bool {
        self.current.is_empty()
    }
}
