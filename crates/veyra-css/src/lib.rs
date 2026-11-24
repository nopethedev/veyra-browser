//! Veyra's CSS engine: values, stylesheet parsing, the cascade and computed
//! styles. Layout consumes the computed styles produced here.

pub mod cascade;
pub mod parser;
pub mod style;
pub mod ua;
pub mod values;

pub use cascade::{PseudoElement, StyleEngine, StyleStats};
pub use parser::{parse_declarations, parse_stylesheet, Declaration, Origin, StyleRule, Stylesheet};
pub use style::{ComputedStyle, Cursor, StyleCtx};
pub use values::*;
