//! Veyra's networking subsystem.
//!
//! Written from scratch on top of TCP sockets and `rustls` (TLS primitives
//! only): HTTP/1.1 framing, redirects, cookies, caching, DNS caching,
//! compression and downloads are implemented here.

pub mod cache;
pub mod client;
pub mod conn;
pub mod cookie;
pub mod dns;
pub mod download;
pub mod testserver;

pub use client::{
    decode_body, guess_mime, CacheMode, Client, Credentials, NetEntry, NetError, NetStats, Request,
    ResourceType, Response,
};
pub use cookie::{Cookie, CookieJar, SameSite};
pub use download::{Download, DownloadManager, DownloadStatus};
