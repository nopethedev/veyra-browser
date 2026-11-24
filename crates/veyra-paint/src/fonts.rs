//! Font loading, shaping (advance-width based) and glyph rasterisation.
//!
//! Veyra ships DejaVu fonts so that pages render identically regardless of
//! what the operating system provides.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use fontdue::{Font, FontSettings};
use veyra_layout::{FontMetrics, FontSpec, TextMeasure};

const REGULAR: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSans.ttf");
const BOLD: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSans-Bold.ttf");
const OBLIQUE: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSans-Oblique.ttf");
const SERIF: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSerif.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSansMono.ttf");

#[derive(Debug, Clone)]
pub struct Glyph {
    pub width: usize,
    pub height: usize,
    pub xmin: i32,
    pub ymin: i32,
    pub advance: f32,
    pub bitmap: Vec<u8>,
}

pub struct FontSystem {
    fonts: Vec<Font>,
    cache: Mutex<HashMap<(u8, u32, u32), Arc<Glyph>>>,
}

impl FontSystem {
    pub fn new() -> FontSystem {
        let load = |bytes: &[u8]| {
            Font::from_bytes(bytes, FontSettings::default()).expect("embedded font must parse")
        };
        FontSystem {
            fonts: vec![load(REGULAR), load(BOLD), load(OBLIQUE), load(SERIF), load(MONO)],
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn font_index(&self, spec: &FontSpec) -> usize {
        let family = spec.family.to_ascii_lowercase();
        let mono = family.contains("mono") || family.contains("courier") || family.contains("consolas") || family.contains("menlo");
        if mono {
            return 4;
        }
        let generic_serif = family.contains("serif") && !family.contains("sans-serif");
        let generic_sans = family.contains("sans-serif") || family.contains("arial") || family.contains("helvetica");
        if spec.italic {
            if spec.bold() {
                return 1;
            }
            return 2;
        }
        if spec.bold() {
            return 1;
        }
        if generic_serif && !generic_sans {
            return 3;
        }
        0
    }

    pub fn glyph(&self, spec: &FontSpec, ch: char) -> Arc<Glyph> {
        let idx = self.font_index(spec) as u8;
        let size_q = ((spec.size.max(1.0)) * 2.0).round().clamp(2.0, 800.0) as u32;
        let key = (idx, ch as u32, size_q);
        if let Some(g) = self.cache.lock().unwrap().get(&key) {
            return g.clone();
        }
        let font = &self.fonts[idx as usize];
        let px = size_q as f32 / 2.0;
        let (metrics, bitmap) = font.rasterize(ch, px);
        let glyph = Arc::new(Glyph {
            width: metrics.width,
            height: metrics.height,
            xmin: metrics.xmin,
            ymin: metrics.ymin,
            advance: metrics.advance_width,
            bitmap,
        });
        self.cache.lock().unwrap().insert(key, glyph.clone());
        glyph
    }

    pub fn glyph_count(&self) -> usize {
        self.cache.lock().unwrap().len()
    }

    pub fn ascent_descent(&self, spec: &FontSpec) -> (f32, f32) {
        let idx = self.font_index(spec);
        let lm = self.fonts[idx].horizontal_line_metrics(spec.size.max(1.0));
        match lm {
            Some(m) => (m.ascent, -m.descent),
            None => (spec.size * 0.8, spec.size * 0.2),
        }
    }
}

impl Default for FontSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextMeasure for FontSystem {
    fn measure(&self, text: &str, spec: &FontSpec) -> f32 {
        let idx = self.font_index(spec);
        let font = &self.fonts[idx];
        let size = spec.size.max(0.1);
        let mut width = 0.0;
        for ch in text.chars() {
            let m = font.metrics(ch, size);
            width += m.advance_width + spec.letter_spacing;
        }
        width
    }

    fn metrics(&self, spec: &FontSpec) -> FontMetrics {
        let idx = self.font_index(spec);
        match self.fonts[idx].horizontal_line_metrics(spec.size.max(0.1)) {
            Some(m) => FontMetrics {
                ascent: m.ascent,
                descent: -m.descent,
                line_gap: m.line_gap,
            },
            None => FontMetrics {
                ascent: spec.size * 0.8,
                descent: spec.size * 0.2,
                line_gap: 0.0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(size: f32) -> FontSpec {
        FontSpec { family: "DejaVu Sans".into(), size, weight: 400, italic: false, letter_spacing: 0.0 }
    }

    #[test]
    fn measures_and_rasterises() {
        let fonts = FontSystem::new();
        let w = fonts.measure("hello", &spec(16.0));
        assert!(w > 30.0 && w < 60.0, "width {w}");
        let g = fonts.glyph(&spec(16.0), 'A');
        assert!(g.advance > 5.0);
        assert!(!g.bitmap.is_empty());
        let m = fonts.metrics(&spec(16.0));
        assert!(m.ascent > 10.0 && m.descent > 0.0);
    }

    #[test]
    fn resolves_families() {
        let fonts = FontSystem::new();
        let mut s = spec(14.0);
        s.family = "monospace".into();
        assert_eq!(fonts.font_index(&s), 4);
        s.family = "serif".into();
        assert_eq!(fonts.font_index(&s), 3);
        s.family = "Inter, sans-serif".into();
        s.weight = 700;
        assert_eq!(fonts.font_index(&s), 1);
    }
}
