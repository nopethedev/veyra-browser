//! Shared primitives for the Veyra browser engine.
//!
//! Nothing in here depends on a browser engine, a webview or a rendering
//! framework: these are the geometric, colour and URL primitives that the
//! networking, HTML, CSS, layout, paint and scripting subsystems all agree on.

pub mod color;
pub mod geom;
pub mod json;
pub mod url;
pub mod util;

pub use color::Color;
pub use geom::{Edges, Point, Rect, Size};
pub use url::Url;

/// The user-visible version string.
pub const VEYRA_VERSION: &str = "0.1.0";
/// The token a server sees in `User-Agent`.
///
/// Veyra ships a realistic Chrome token because a large share of sites serve
/// reduced or broken markup to unknown browsers (Google in particular serves
/// its basic HTML page). The same string is exposed as
/// `navigator.userAgent`, so both views of the browser agree.
pub const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
