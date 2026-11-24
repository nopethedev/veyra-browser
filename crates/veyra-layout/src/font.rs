//! Text measurement abstraction used by layout. The paint crate supplies the
//! real implementation backed by rasterised font data; tests and headless
//! callers can use [`SimpleMeasure`].

use veyra_common::{Size, Url};
use veyra_css::ComputedStyle;

#[derive(Debug, Clone, PartialEq)]
pub struct FontSpec {
    pub family: String,
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
    pub letter_spacing: f32,
}

impl FontSpec {
    pub fn from_style(style: &ComputedStyle) -> FontSpec {
        FontSpec {
            family: style.font_family.clone(),
            size: style.font_size_px(),
            weight: style.font_weight,
            italic: style.font_style != veyra_css::style::FontStyle::Normal,
            letter_spacing: style.letter_spacing,
        }
    }

    pub fn bold(&self) -> bool {
        self.weight >= 600
    }

    pub fn with_size(&self, size: f32) -> FontSpec {
        FontSpec { size, ..self.clone() }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

impl FontMetrics {
    pub fn height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }
}

pub trait TextMeasure {
    fn measure(&self, text: &str, font: &FontSpec) -> f32;
    fn metrics(&self, font: &FontSpec) -> FontMetrics;
    fn space_width(&self, font: &FontSpec) -> f32 {
        self.measure(" ", font)
    }
}

pub trait ImageInfo {
    fn image_size(&self, url: &Url) -> Option<Size>;
}

pub struct NoImages;

impl ImageInfo for NoImages {
    fn image_size(&self, _url: &Url) -> Option<Size> {
        None
    }
}

/// Deterministic approximate metrics, used where no font data is available.
pub struct SimpleMeasure;

impl TextMeasure for SimpleMeasure {
    fn measure(&self, text: &str, font: &FontSpec) -> f32 {
        let per_char = font.size * 0.541;
        let width: f32 = text
            .chars()
            .map(|c| match c {
                ' ' => per_char * 0.5,
                'i' | 'l' | 'j' | '.' | ',' | '\'' => per_char * 0.45,
                'm' | 'w' | 'M' | 'W' => per_char * 1.5,
                _ => per_char,
            })
            .sum();
        width + text.chars().count() as f32 * font.letter_spacing
    }

    fn metrics(&self, font: &FontSpec) -> FontMetrics {
        FontMetrics {
            ascent: font.size * 0.8,
            descent: font.size * 0.2,
            line_gap: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_text() {
        let m = SimpleMeasure;
        let f = FontSpec { family: "x".into(), size: 10.0, weight: 400, italic: false, letter_spacing: 0.0 };
        assert!(m.measure("hello", &f) > 20.0);
        assert_eq!(m.metrics(&f).ascent, 8.0);
    }
}
