//! Veyra's browser engine: loads real pages over HTTP/TLS, builds the DOM and
//! CSSOM, runs JavaScript, performs layout and paints into a software canvas.

pub mod bindings;
pub mod browser;
pub mod dom_api;
pub mod event_api;
pub mod page;
pub mod state;
pub mod style_api;

pub use browser::{Bookmark, Browser, HistoryEntry, Tab};
pub use page::{LoadOptions, Page};
pub use state::DomState;
