//! Decoded image storage and sampling.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use veyra_common::{Color, Url};

#[derive(Debug, Clone)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// RGBA8, straight alpha.
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn from_rgba(width: usize, height: usize, pixels: Vec<u8>) -> Image {
        Image { width, height, pixels }
    }

    pub fn decode(bytes: &[u8]) -> Option<Image> {
        // SVG needs a vector renderer; Veyra currently skips it instead of
        // displaying broken raster data.
        let head = &bytes[..bytes.len().min(256)];
        if head.windows(4).any(|w| w == b"<svg") {
            return None;
        }
        let img = image::load_from_memory(bytes).ok()?;
        let (w, h) = (img.width() as usize, img.height() as usize);
        if w == 0 || h == 0 || w > 12_000 || h > 12_000 {
            return None;
        }
        let rgba = img.to_rgba8();
        Some(Image::from_rgba(w, h, rgba.into_raw()))
    }

    #[inline]
    pub fn sample(&self, x: usize, y: usize) -> Color {
        if x >= self.width || y >= self.height {
            return Color::TRANSPARENT;
        }
        let i = (y * self.width + x) * 4;
        Color::rgba(
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        )
    }

    pub fn sample_bilinear(&self, x: f32, y: f32) -> Color {
        if self.width == 0 || self.height == 0 {
            return Color::TRANSPARENT;
        }
        let x = x.clamp(0.0, (self.width - 1) as f32);
        let y = y.clamp(0.0, (self.height - 1) as f32);
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;
        let c00 = self.sample(x0, y0);
        let c10 = self.sample(x1, y0);
        let c01 = self.sample(x0, y1);
        let c11 = self.sample(x1, y1);
        let top = c00.mix(c10, fx);
        let bottom = c01.mix(c11, fx);
        top.mix(bottom, fy)
    }

    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }
}

#[derive(Default)]
pub struct ImageCache {
    entries: HashMap<String, Arc<Image>>,
    failures: HashSet<String>,
    bytes: usize,
}

impl ImageCache {
    pub fn new() -> ImageCache {
        ImageCache::default()
    }

    pub fn insert(&mut self, key: &str, image: Arc<Image>) {
        if let Some(old) = self.entries.insert(key.to_string(), image.clone()) {
            self.bytes = self.bytes.saturating_sub(old.byte_len());
        }
        self.bytes += image.byte_len();
    }

    /// Decodes and stores an image. Returns false when the format is not
    /// decodable (including SVG, which needs a vector renderer).
    pub fn insert_bytes(&mut self, key: &str, bytes: &[u8]) -> bool {
        match Image::decode(bytes) {
            Some(img) => {
                self.insert(key, Arc::new(img));
                true
            }
            None => {
                self.failures.insert(key.to_string());
                false
            }
        }
    }

    pub fn get(&self, url: &Url) -> Option<Arc<Image>> {
        self.entries.get(&url.to_string()).cloned()
    }

    pub fn get_str(&self, key: &str) -> Option<Arc<Image>> {
        self.entries.get(key).cloned()
    }

    pub fn failed(&self, key: &str) -> bool {
        self.failures.contains(key)
    }

    /// Remembers a URL that failed to download or decode so it is not retried
    /// on every relayout.
    pub fn mark_failed(&mut self, key: &str) {
        self.failures.insert(key.to_string());
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn total_bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_and_scales() {
        let img = Image::from_rgba(2, 2, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255]);
        assert_eq!(img.sample(0, 0), Color::rgb(255, 0, 0));
        assert_eq!(img.sample(9, 9), Color::TRANSPARENT);
        let mid = img.sample_bilinear(0.5, 0.0);
        assert!(mid.r > 100 && mid.g > 100);
    }

    #[test]
    fn decodes_png() {
        let canvas = crate::canvas::Canvas::new(3, 2);
        let png = canvas.to_png();
        let img = Image::decode(&png).expect("png decodes");
        assert_eq!((img.width, img.height), (3, 2));
        assert_eq!(img.sample(1, 1), Color::rgb(255, 255, 255));
    }

    #[test]
    fn rejects_svg_without_panicking() {
        assert!(Image::decode(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").is_none());
    }
}
