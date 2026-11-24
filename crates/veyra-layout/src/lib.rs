//! Veyra's layout engine: box tree construction, block/inline flow, flexbox,
//! grid, tables and text line breaking.

pub mod font;
pub mod layout;

pub use font::{FontMetrics, FontSpec, ImageInfo, NoImages, SimpleMeasure, TextMeasure};
pub use layout::{
    layout_document, FormControlKind, FormInfo, ImagePlacement, LayoutBox, LayoutKind, LayoutTree,
    TextFragment,
};
