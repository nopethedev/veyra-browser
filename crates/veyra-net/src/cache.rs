//! A real HTTP cache: freshness lifetimes, validators and revalidation,
//! honouring `Cache-Control`, `Expires`, `ETag` and `Last-Modified`.
//!
//! `Vary` is recorded but the cache is keyed by URL alone, so responses with
//! `Vary` are stored only when the header is absent or `Accept-Encoding`.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use veyra_common::util::{header_get, parse_http_date, now_unix};

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub url: String,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub stored_at: u64,
    pub max_age: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub no_cache: bool,
    pub must_revalidate: bool,
    pub immutable: bool,
}

impl CacheEntry {
    pub fn freshness(&self) -> u64 {
        if self.immutable {
            return 31_536_000;
        }
        if let Some(age) = self.max_age {
            return age;
        }
        // Expires - Date
        let expires = header_get(&self.headers, "expires").and_then(parse_http_date);
        let date = header_get(&self.headers, "date").and_then(parse_http_date).unwrap_or(self.stored_at);
        if let Some(exp) = expires {
            return exp.saturating_sub(date);
        }
        // Heuristic freshness from Last-Modified: 10% of age, capped at a day.
        if let Some(lm) = header_get(&self.headers, "last-modified").and_then(parse_http_date) {
            let age = date.saturating_sub(lm);
            return (age / 10).min(86_400);
        }
        0
    }

    pub fn is_fresh(&self, now: u64) -> bool {
        if self.no_cache {
            return false;
        }
        now.saturating_sub(self.stored_at) < self.freshness()
    }

    pub fn is_stale(&self, now: u64) -> bool {
        !self.is_fresh(now)
    }

    pub fn age(&self, now: u64) -> u64 {
        now.saturating_sub(self.stored_at)
    }
}

pub struct HttpCache {
    entries: HashMap<String, Arc<CacheEntry>>,
    order: VecDeque<String>,
    capacity: usize,
    bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub revalidations: u64,
    pub enabled: bool,
}

impl HttpCache {
    pub fn new(capacity: usize) -> Self {
        HttpCache {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity,
            bytes: 0,
            hits: 0,
            misses: 0,
            revalidations: 0,
            enabled: true,
        }
    }

    pub fn get(&mut self, url: &str, now: u64) -> Option<(Arc<CacheEntry>, bool)> {
        if !self.enabled {
            return None;
        }
        let entry = self.entries.get(url).cloned()?;
        self.order.retain(|u| u != url);
        self.order.push_back(url.to_string());
        if entry.is_fresh(now) {
            self.hits += 1;
            Some((entry, true))
        } else {
            self.misses += 1;
            Some((entry, false))
        }
    }

    pub fn put(&mut self, entry: CacheEntry) {
        if !self.enabled {
            return;
        }
        let url = entry.url.clone();
        if let Some(old) = self.entries.get(&url) {
            self.bytes = self.bytes.saturating_sub(old.body.len());
        }
        self.bytes += entry.body.len();
        self.entries.insert(url.clone(), Arc::new(entry));
        self.order.retain(|u| u != &url);
        self.order.push_back(url);
        while self.order.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                if let Some(e) = self.entries.remove(&old) {
                    self.bytes = self.bytes.saturating_sub(e.body.len());
                }
            }
        }
    }

    pub fn remove(&mut self, url: &str) {
        if let Some(e) = self.entries.remove(url) {
            self.bytes = self.bytes.saturating_sub(e.body.len());
            self.order.retain(|u| u != url);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.bytes = 0;
    }

    pub fn entries(&self) -> u64 {
        self.entries.len() as u64
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

/// Extracts freshness directives. `req_headers` is unused for now but keeps
/// the signature stable for future `no-cache` request directives.
pub fn directives(headers: &[(String, String)]) -> (Option<u64>, bool, bool, bool) {
    let mut max_age = None;
    let mut no_cache = false;
    let mut must_revalidate = false;
    let mut immutable = false;
    if let Some(cc) = header_get(headers, "cache-control") {
        for raw in cc.split(',') {
            let part = raw.trim();
            let (name, val) = match part.split_once('=') {
                Some((n, v)) => (n.trim().to_ascii_lowercase(), v.trim().trim_matches('"')),
                None => (part.to_ascii_lowercase(), ""),
            };
            match name.as_str() {
                "max-age" => max_age = val.parse::<u64>().ok(),
                "s-maxage" => {}
                "no-cache" => no_cache = true,
                "no-store" => {
                    no_cache = true;
                    must_revalidate = true;
                }
                "must-revalidate" | "proxy-revalidate" => must_revalidate = true,
                "immutable" => immutable = true,
                _ => {}
            }
        }
    }
    (max_age, no_cache, must_revalidate, immutable)
}

pub fn is_cacheable(status: u16, method: &str, headers: &[(String, String)]) -> bool {
    if method != "GET" {
        return false;
    }
    if !matches!(status, 200 | 203 | 204 | 301 | 308 | 404 | 410) {
        return false;
    }
    if let Some(cc) = header_get(headers, "cache-control") {
        let lower = cc.to_ascii_lowercase();
        if lower.contains("no-store") {
            return false;
        }
    }
    if let Some(vary) = header_get(headers, "vary") {
        let v = vary.trim();
        if !(v.is_empty() || v.eq_ignore_ascii_case("accept-encoding")) {
            return false;
        }
    }
    true
}

pub fn build_entry(
    url: &str,
    status: u16,
    reason: &str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> CacheEntry {
    let (max_age, no_cache, must_revalidate, immutable) = directives(&headers);
    let etag = header_get(&headers, "etag").map(|s| s.to_string());
    let last_modified = header_get(&headers, "last-modified").map(|s| s.to_string());
    CacheEntry {
        url: url.to_string(),
        status,
        reason: reason.to_string(),
        headers,
        body,
        stored_at: now_unix(),
        max_age,
        etag,
        last_modified,
        no_cache,
        must_revalidate,
        immutable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn freshness_from_max_age() {
        let e = build_entry("u", 200, "OK", h(&[("Cache-Control", "max-age=60")]), vec![]);
        let now = e.stored_at;
        assert!(e.is_fresh(now));
        assert!(e.is_stale(now + 61));
    }

    #[test]
    fn no_store_is_not_cacheable() {
        assert!(!is_cacheable(200, "GET", &h(&[("Cache-Control", "no-store, max-age=5")])));
        assert!(is_cacheable(200, "GET", &h(&[("Cache-Control", "max-age=5")])));
        assert!(!is_cacheable(200, "POST", &h(&[])));
    }
}
