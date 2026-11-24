//! Veyra's software rasteriser: canvas primitives, fonts, image cache and the
//! painter that turns a layout tree into pixels.

pub mod canvas;
pub mod fonts;
pub mod images;
pub mod painter;

pub use canvas::Canvas;
pub use fonts::{FontSystem, Glyph};
pub use images::{Image, ImageCache};
pub use painter::{paint_page, PaintConfig};
