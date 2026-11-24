//! RFC 6265 cookie storage and matching.

use std::collections::HashMap;

use veyra_common::json::Json;
use veyra_common::util::{now_unix, parse_http_date};
use veyra_common::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Unset,
    Lax,
    Strict,
    None,
}

#[derive(Debug, Clone)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub expires: Option<u64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
    pub created: u64,
}

impl Cookie {
    fn is_expired(&self, now: u64) -> bool {
        matches!(self.expires, Some(t) if t <= now)
    }

    fn domain_matches(&self, host: &str) -> bool {
        if self.host_only {
            host == self.domain
        } else {
            host == self.domain
                || (host.ends_with(&self.domain)
                    && host.len() > self.domain.len()
                    && host.as_bytes()[host.len() - self.domain.len() - 1] == b'.')
        }
    }

    fn path_matches(&self, path: &str) -> bool {
        if path == self.path {
            return true;
        }
        if path.starts_with(&self.path) {
            return self.path.ends_with('/') || path.as_bytes().get(self.path.len()) == Some(&b'/');
        }
        false
    }

    fn key(&self) -> (String, String, String) {
        (self.domain.clone(), self.path.clone(), self.name.clone())
    }
}

#[derive(Default)]
pub struct CookieJar {
    cookies: HashMap<(String, String, String), Cookie>,
    /// When true, cookies are not accepted/sent for subresource requests that
    /// cross a site boundary.
    pub block_third_party: bool,
    pub site: Option<String>,
}

impl CookieJar {
    pub fn new() -> Self {
        CookieJar::default()
    }

    pub fn set_cookie_header(&mut self, url: &Url, header: &str, top_level: bool) {
        let now = now_unix();
        let mut parts = header.split(';');
        let pair = parts.next().unwrap_or("").trim();
        let (name, value) = match pair.split_once('=') {
            Some((n, v)) => (n.trim(), v.trim()),
            None => return,
        };
        if name.is_empty() {
            return;
        }
        let mut cookie = Cookie {
            name: name.to_string(),
            value: value.trim_matches('"').to_string(),
            domain: url.host.clone(),
            host_only: true,
            path: default_path(&url.path),
            expires: None,
            secure: false,
            http_only: false,
            same_site: SameSite::Unset,
            created: now,
        };
        let mut max_age_seen = false;
        for attr in parts {
            let attr = attr.trim();
            if attr.is_empty() {
                continue;
            }
            let (k, v) = match attr.split_once('=') {
                Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim()),
                None => (attr.to_ascii_lowercase(), ""),
            };
            match k.as_str() {
                "domain" => {
                    let d = v.trim_start_matches('.').to_ascii_lowercase();
                    if !d.is_empty() {
                        if !url.host.ends_with(&d) || (url.host != d && !url.host.ends_with(&format!(".{}", d))) {
                            return; // domain mismatch: reject the cookie
                        }
                        cookie.domain = d;
                        cookie.host_only = false;
                    }
                }
                "path" => {
                    if v.starts_with('/') {
                        cookie.path = v.to_string();
                    }
                }
                "expires" => {
                    if !max_age_seen {
                        cookie.expires = parse_http_date(v);
                    }
                }
                "max-age" => {
                    if let Ok(secs) = v.parse::<i64>() {
                        max_age_seen = true;
                        cookie.expires = Some(if secs <= 0 {
                            0
                        } else {
                            now.saturating_add(secs as u64)
                        });
                    }
                }
                "secure" => cookie.secure = true,
                "httponly" => cookie.http_only = true,
                "samesite" => {
                    cookie.same_site = match v.to_ascii_lowercase().as_str() {
                        "strict" => SameSite::Strict,
                        "lax" => SameSite::Lax,
                        "none" => SameSite::None,
                        _ => SameSite::Unset,
                    }
                }
                _ => {}
            }
        }
        if cookie.secure && url.scheme != "https" {
            return;
        }
        if self.block_third_party {
            if let Some(site) = &self.site {
                if site != &registrable_site(&url.host) {
                    return;
                }
            }
        }
        if cookie.is_expired(now) {
            self.cookies.remove(&cookie.key());
            return;
        }
        if !top_level && cookie.same_site == SameSite::Strict {
            // Not fatal: Strict cookies are stored but withheld on cross-site
            // subrequests by `header_for`.
        }
        self.cookies.insert(cookie.key(), cookie);
    }

    /// Builds the Cookie header for a request. `top_level` marks a
    /// user-initiated navigation, which allows Lax cookies cross-site.
    pub fn header_for(&mut self, url: &Url, top_level: bool, from_site: Option<&str>) -> Option<String> {
        let now = now_unix();
        self.cookies.retain(|_, c| !c.is_expired(now));
        let cross_site = from_site.map(|s| s != registrable_site(&url.host)).unwrap_or(false);
        if self.block_third_party && cross_site && !top_level {
            return None;
        }
        let mut matching: Vec<&Cookie> = self
            .cookies
            .values()
            .filter(|c| {
                if !c.domain_matches(&url.host) || !c.path_matches(&url.path) {
                    return false;
                }
                if c.secure && url.scheme != "https" {
                    return false;
                }
                match c.same_site {
                    SameSite::None => c.secure,
                    SameSite::Strict => !cross_site,
                    SameSite::Lax => !cross_site || top_level,
                    SameSite::Unset => !cross_site || top_level,
                }
            })
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by(|a, b| b.path.len().cmp(&a.path.len()).then(a.created.cmp(&b.created)));
        Some(
            matching
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    pub fn all(&self) -> Vec<&Cookie> {
        self.cookies.values().collect()
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    pub fn clear(&mut self) {
        self.cookies.clear();
    }

    pub fn clear_for_host(&mut self, host: &str) {
        self.cookies.retain(|_, c| c.domain != host);
    }

    pub fn delete(&mut self, domain: &str, path: &str, name: &str) {
        self.cookies.remove(&(domain.to_string(), path.to_string(), name.to_string()));
    }

    pub fn to_json(&self) -> Json {
        Json::Arr(
            self.all()
                .into_iter()
                .map(|c| {
                    Json::obj(vec![
                        ("name", Json::string(&c.name)),
                        ("value", Json::string(&c.value)),
                        ("domain", Json::string(&c.domain)),
                        ("path", Json::string(&c.path)),
                        ("secure", Json::Bool(c.secure)),
                        ("httpOnly", Json::Bool(c.http_only)),
                        ("hostOnly", Json::Bool(c.host_only)),
                        ("expires", c.expires.map(|e| Json::Num(e as f64)).unwrap_or(Json::Null)),
                    ])
                })
                .collect(),
        )
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_json().stringify_pretty())
    }
}

fn default_path(path: &str) -> String {
    if !path.starts_with('/') {
        return "/".into();
    }
    match path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => path[..i].to_string(),
    }
}

/// Approximate registrable domain (eTLD+1) for SameSite and third-party
/// decisions. Not a full public suffix list.
pub fn registrable_site(host: &str) -> String {
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return host.to_string();
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() <= 2 {
        return host.to_string();
    }
    let two_part_suffixes = ["co.uk", "org.uk", "ac.uk", "com.au", "co.jp", "co.nz", "com.br"];
    let tail2 = parts[parts.len() - 2..].join(".");
    if two_part_suffixes.contains(&tail2.as_str()) && parts.len() >= 3 {
        parts[parts.len() - 3..].join(".")
    } else {
        tail2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_matches() {
        let mut jar = CookieJar::new();
        let url = Url::parse("https://example.com/a/b").unwrap();
        jar.set_cookie_header(&url, "id=42; Path=/; Secure; HttpOnly", true);
        jar.set_cookie_header(&url, "theme=dark; Path=/", true);
        let header = jar.header_for(&url, true, None).unwrap();
        assert!(header.contains("id=42"));
        assert!(header.contains("theme=dark"));
        let other = Url::parse("https://example.com/other").unwrap();
        assert!(jar.header_for(&other, false, None).unwrap().contains("theme=dark"));
        let sub = Url::parse("https://sub.example.com/").unwrap();
        // Host-only cookies must not leak to subdomains.
        assert!(jar.header_for(&sub, false, None).is_none());
        assert!(jar.header_for(&Url::parse("http://example.com/").unwrap(), true, None).is_some());
    }

    #[test]
    fn domain_mismatch_rejected() {
        let mut jar = CookieJar::new();
        let url = Url::parse("https://example.com/").unwrap();
        jar.set_cookie_header(&url, "x=1; Domain=other.com", true);
        assert!(jar.header_for(&url, true, None).is_none());
    }

    #[test]
    fn expiry_deletes() {
        let mut jar = CookieJar::new();
        let url = Url::parse("https://example.com/").unwrap();
        jar.set_cookie_header(&url, "x=1", true);
        jar.set_cookie_header(&url, "x=0; Max-Age=0", true);
        assert!(jar.header_for(&url, true, None).is_none());
    }

    #[test]
    fn sites() {
        assert_eq!(registrable_site("www.example.co.uk"), "example.co.uk");
        assert_eq!(registrable_site("a.b.example.com"), "example.com");
        assert_eq!(registrable_site("127.0.0.1"), "127.0.0.1");
    }
}
