//! The Veyra HTTP client: request construction, redirects, content decoding,
//! cookies, caching and an inspector log.

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flate2::read::{GzDecoder, ZlibDecoder};
use rustls::ClientConfig;
use veyra_common::util::{header_get, now_unix, parse_http_date};
use veyra_common::{Url, USER_AGENT};

use crate::cache::{build_entry, is_cacheable, CacheEntry, HttpCache};
use crate::conn::Conn;
use crate::cookie::CookieJar;
use crate::dns::DnsCache;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    Document,
    StyleSheet,
    Script,
    Image,
    Font,
    Fetch,
    Other,
}

impl Default for ResourceType {
    fn default() -> Self {
        ResourceType::Document
    }
}

impl ResourceType {
    pub fn label(&self) -> &'static str {
        match self {
            ResourceType::Document => "document",
            ResourceType::StyleSheet => "stylesheet",
            ResourceType::Script => "script",
            ResourceType::Image => "image",
            ResourceType::Font => "font",
            ResourceType::Fetch => "fetch",
            ResourceType::Other => "other",
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct NetStats {
    pub requests: u64,
    pub bytes_down: u64,
    pub cache_hits: u64,
    pub errors: u64,
    pub total_ms: u64,
}

#[derive(Debug, Clone)]
pub struct NetEntry {
    pub id: u64,
    pub url: String,
    pub method: String,
    pub resource_type: ResourceType,
    pub status: u16,
    pub bytes: usize,
    pub ms: u64,
    pub from_cache: bool,
    pub error: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub started_unix_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Request {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub resource_type: ResourceType,
    /// Origin of the requesting document, for SameSite/third-party cookie rules.
    pub initiator: Option<String>,
    /// Marks a user-initiated navigation (affects Lax cookies).
    pub top_level: bool,
    pub credentials: Credentials,
    pub cache_mode: CacheMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Credentials {
    #[default]
    Include,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheMode {
    #[default]
    Default,
    NoCache,
    Reload,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Request {
        Request {
            url: url.into(),
            method: "GET".into(),
            resource_type: ResourceType::Document,
            top_level: true,
            ..Default::default()
        }
    }

    pub fn post(url: impl Into<String>, body: Vec<u8>) -> Request {
        Request {
            url: url.into(),
            method: "POST".into(),
            body: Some(body),
            resource_type: ResourceType::Fetch,
            ..Default::default()
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn kind(mut self, kind: ResourceType) -> Request {
        self.resource_type = kind;
        self.top_level = kind == ResourceType::Document;
        self
    }

    pub fn cache_mode(mut self, mode: CacheMode) -> Request {
        self.cache_mode = mode;
        self
    }

    pub fn initiator(mut self, origin: &str) -> Request {
        self.initiator = Some(origin.to_string());
        self.top_level = false;
        self
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub url: Url,
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub from_cache: bool,
    pub redirects: Vec<String>,
    pub elapsed_ms: u64,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        header_get(&self.headers, name)
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn mime(&self) -> String {
        self.header("content-type")
            .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                guess_mime(self.url.path.as_str()).unwrap_or("application/octet-stream").to_string()
            })
    }

    pub fn charset(&self) -> String {
        if let Some(ct) = self.header("content-type") {
            for part in ct.split(';').skip(1) {
                let part = part.trim();
                if let Some(v) = part.strip_prefix("charset=") {
                    return v.trim_matches('"').to_ascii_lowercase();
                }
            }
        }
        "utf-8".into()
    }

    pub fn text(&self) -> String {
        decode_body(&self.body, self.charset().as_str())
    }
}

pub fn guess_mime(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "html" | "htm" | "xhtml" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "txt" => "text/plain",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => return None,
    })
}

pub fn decode_body(bytes: &[u8], charset: &str) -> String {
    match charset {
        "iso-8859-1" | "latin1" | "windows-1252" => {
            // windows-1252 with the Latin-1 fallback.
            bytes
                .iter()
                .map(|b| match *b {
                    0x80 => '\u{20ac}',
                    0x82 => '\u{201a}',
                    0x83 => '\u{0192}',
                    0x84 => '\u{201e}',
                    0x85 => '\u{2026}',
                    0x86 => '\u{2020}',
                    0x87 => '\u{2021}',
                    0x88 => '\u{02c6}',
                    0x89 => '\u{2030}',
                    0x8a => '\u{0160}',
                    0x8b => '\u{2039}',
                    0x8c => '\u{0152}',
                    0x8e => '\u{017d}',
                    0x91 => '\u{2018}',
                    0x92 => '\u{2019}',
                    0x93 => '\u{201c}',
                    0x94 => '\u{201d}',
                    0x95 => '\u{2022}',
                    0x96 => '\u{2013}',
                    0x97 => '\u{2014}',
                    0x98 => '\u{02dc}',
                    0x99 => '\u{2122}',
                    0x9a => '\u{0161}',
                    0x9b => '\u{203a}',
                    0x9c => '\u{0153}',
                    0x9e => '\u{017e}',
                    0x9f => '\u{0178}',
                    other => other as char,
                })
                .collect()
        }
        "utf-16" | "utf-16le" | "utf-16be" => {
            let le = charset != "utf-16be";
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

#[derive(Debug)]
pub enum NetError {
    InvalidUrl(String),
    UnsupportedScheme(String),
    Io(io::Error),
    Http(String),
    TooManyRedirects,
    BodyTooLarge,
    UnsupportedEncoding(String),
    Blocked(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::InvalidUrl(u) => write!(f, "invalid URL: {u}"),
            NetError::UnsupportedScheme(s) => write!(f, "unsupported URL scheme: {s}"),
            NetError::Io(e) => write!(f, "network error: {e}"),
            NetError::Http(e) => write!(f, "HTTP error: {e}"),
            NetError::TooManyRedirects => write!(f, "too many redirects"),
            NetError::BodyTooLarge => write!(f, "response body too large"),
            NetError::UnsupportedEncoding(e) => write!(f, "unsupported content encoding: {e}"),
            NetError::Blocked(e) => write!(f, "request blocked: {e}"),
        }
    }
}

impl std::error::Error for NetError {}

impl From<io::Error> for NetError {
    fn from(e: io::Error) -> Self {
        NetError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, NetError>;

pub struct Client {
    tls: Arc<ClientConfig>,
    pool: Mutex<HashMap<String, Vec<Conn>>>,
    pub dns: DnsCache,
    pub cookies: Mutex<CookieJar>,
    pub cache: Mutex<HttpCache>,
    pub log: Mutex<Vec<NetEntry>>,
    pub stats: Mutex<NetStats>,
    pub timeout: Duration,
    pub max_redirects: usize,
    pub max_body: usize,
    pub log_limit: usize,
    next_id: Mutex<u64>,
    /// Hosts the browser must never contact (ad/tracker style block list).
    pub blocked_hosts: Mutex<Vec<String>>,
}

impl Client {
    pub fn new() -> Result<Client> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let mut dns = DnsCache::new();
        dns.load_hosts_file("/etc/hosts");
        Ok(Client {
            tls: Arc::new(tls),
            pool: Mutex::new(HashMap::new()),
            dns,
            cookies: Mutex::new(CookieJar::new()),
            cache: Mutex::new(HttpCache::new(256)),
            log: Mutex::new(Vec::new()),
            stats: Mutex::new(NetStats::default()),
            timeout: Duration::from_secs(25),
            max_redirects: 20,
            max_body: 32 * 1024 * 1024,
            log_limit: 400,
            next_id: Mutex::new(1),
            blocked_hosts: Mutex::new(Vec::new()),
        })
    }

    pub fn set_blocked_hosts(&self, hosts: Vec<String>) {
        *self.blocked_hosts.lock().unwrap() = hosts;
    }

    pub fn clear_cookies(&self) {
        self.cookies.lock().unwrap().clear();
    }

    pub fn set_cache_enabled(&self, on: bool) {
        self.cache.lock().unwrap().enabled = on;
    }

    pub fn cache_entries(&self) -> u64 {
        self.cache.lock().unwrap().entries()
    }

    pub fn network_log(&self) -> Vec<NetEntry> {
        self.log.lock().unwrap().clone()
    }

    pub fn clear_network_log(&self) {
        self.log.lock().unwrap().clear();
    }

    fn record(&self, entry: NetEntry) {
        let mut log = self.log.lock().unwrap();
        log.push(entry);
        let limit = self.log_limit;
        if log.len() > limit {
            let drop = log.len() - limit;
            log.drain(..drop);
        }
    }

    /// Performs a request, following redirects and consulting the cache.
    pub fn fetch(&self, req: Request) -> Result<Response> {
        let started = Instant::now();
        let id = {
            let mut n = self.next_id.lock().unwrap();
            let id = *n;
            *n += 1;
            id
        };
        let method = if req.method.is_empty() { "GET".to_string() } else { req.method.to_ascii_uppercase() };
        let mut current = match Url::parse(&req.url) {
            Some(u) => u,
            None => {
                self.record(NetEntry {
                    id,
                    url: req.url.clone(),
                    method: method.clone(),
                    resource_type: req.resource_type,
                    status: 0,
                    bytes: 0,
                    ms: started.elapsed().as_millis() as u64,
                    from_cache: false,
                    error: Some("invalid URL".into()),
                    request_headers: vec![],
                    response_headers: vec![],
                    started_unix_ms: now_unix() * 1000,
                });
                return Err(NetError::InvalidUrl(req.url.clone()));
            }
        };
        let mut redirects: Vec<String> = Vec::new();
        let mut method = method;
        let mut body = req.body.clone();
        let mut total_cache_hit = false;
        let mut visited: Vec<String> = Vec::new();

        for _hop in 0..=self.max_redirects {
            if !current.is_http() {
                return Err(NetError::UnsupportedScheme(current.scheme.clone()));
            }
            if self.is_blocked(&current.host) {
                return Err(NetError::Blocked(format!("{} is on the block list", current.host)));
            }
            if visited.contains(&current.to_string()) {
                return Err(NetError::Http("redirect loop detected".into()));
            }
            visited.push(current.to_string());

            // ---- cache lookup ------------------------------------------------
            let mut conditional: Vec<(String, String)> = Vec::new();
            if method == "GET" {
                let now = now_unix();
                let cached = if req.cache_mode == CacheMode::Reload {
                    None
                } else {
                    self.cache.lock().unwrap().get(&current.to_string(), now)
                };
                if let Some((entry, fresh)) = cached {
                    if fresh && req.cache_mode == CacheMode::Default {
                        total_cache_hit = true;
                        let resp = Response {
                            url: current.clone(),
                            status: entry.status,
                            reason: entry.reason.clone(),
                            headers: entry.headers.clone(),
                            body: entry.body.clone(),
                            from_cache: true,
                            redirects,
                            elapsed_ms: started.elapsed().as_millis() as u64,
                        };
                        self.stats.lock().unwrap().cache_hits += 1;
                        self.record(NetEntry {
                            id,
                            url: current.to_string(),
                            method: method.clone(),
                            resource_type: req.resource_type,
                            status: entry.status,
                            bytes: entry.body.len(),
                            ms: started.elapsed().as_millis() as u64,
                            from_cache: true,
                            error: None,
                            request_headers: vec![],
                            response_headers: entry.headers.clone(),
                            started_unix_ms: now_unix() * 1000,
                        });
                        return Ok(resp);
                    }
                    if let Some(etag) = &entry.etag {
                        conditional.push(("If-None-Match".into(), etag.clone()));
                    }
                    if let Some(lm) = &entry.last_modified {
                        conditional.push(("If-Modified-Since".into(), lm.clone()));
                    }
                }
            }

            // ---- build headers ----------------------------------------------
            let mut headers: Vec<(String, String)> = vec![
                ("Host".into(), host_header(&current)),
                ("User-Agent".into(), USER_AGENT.to_string()),
                ("Accept".into(), accept_for(req.resource_type)),
                ("Accept-Language".into(), "en-US,en;q=0.9".into()),
                ("Accept-Encoding".into(), "gzip, deflate".into()),
                ("Connection".into(), "keep-alive".into()),
                ("Upgrade-Insecure-Requests".into(), "1".into()),
            ];
            // Sites increasingly serve different markup to clients that look
            // like real browsers. Send the same Fetch Metadata headers Chrome
            // does so we get the same document and subresources.
            let same_origin = req
                .initiator
                .as_deref()
                .and_then(|s| Url::parse(s))
                .map(|u| u.host.eq_ignore_ascii_case(&current.host))
                .unwrap_or(true);
            let (site, mode) = if req.top_level {
                ("none", "navigate")
            } else if same_origin {
                ("same-origin", "no-cors")
            } else {
                ("cross-site", "no-cors")
            };
            let dest = match req.resource_type {
                ResourceType::Document => "document",
                ResourceType::StyleSheet => "style",
                ResourceType::Script => "script",
                ResourceType::Image => "image",
                ResourceType::Font => "font",
                ResourceType::Fetch => "empty",
                ResourceType::Other => "empty",
            };
            headers.push(("Sec-Fetch-Site".into(), site.into()));
            headers.push(("Sec-Fetch-Mode".into(), mode.into()));
            headers.push(("Sec-Fetch-Dest".into(), dest.into()));
            if req.top_level {
                headers.push(("Sec-Fetch-User".into(), "?1".into()));
            }
            if let Some(site) = &req.initiator {
                if !req.top_level {
                    headers.push(("Referer".into(), site.clone()));
                }
            }
            for (k, v) in &req.headers {
                if !headers.iter().any(|(ek, _)| ek.eq_ignore_ascii_case(k)) {
                    headers.push((k.clone(), v.clone()));
                }
            }
            if req.credentials == Credentials::Include {
                let initiator_site = req.initiator.as_deref().and_then(|s| Url::parse(s)).map(|u| u.host);
                let cookie = {
                    let mut jar = self.cookies.lock().unwrap();
                    jar.header_for(&current, req.top_level, initiator_site.as_deref())
                };
                if let Some(c) = cookie {
                    headers.push(("Cookie".into(), c));
                }
            }
            headers.extend(conditional.iter().cloned());
            if let Some(b) = &body {
                headers.push(("Content-Length".into(), b.len().to_string()));
                if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
                    headers.push(("Content-Type".into(), "application/x-www-form-urlencoded".into()));
                }
            }

            // ---- send -------------------------------------------------------
            let result = self.send(&current, &method, &headers, body.as_deref());
            let (status, reason, resp_headers, resp_body, conn_reusable) = match result {
                Ok(v) => v,
                Err(e) => {
                    self.stats.lock().unwrap().errors += 1;
                    self.record(NetEntry {
                        id,
                        url: current.to_string(),
                        method: method.clone(),
                        resource_type: req.resource_type,
                        status: 0,
                        bytes: 0,
                        ms: started.elapsed().as_millis() as u64,
                        from_cache: false,
                        error: Some(e.to_string()),
                        request_headers: headers.clone(),
                        response_headers: vec![],
                        started_unix_ms: now_unix() * 1000,
                    });
                    return Err(e);
                }
            };

            // ---- cookies ----------------------------------------------------
            if req.credentials == Credentials::Include {
                let set_cookies: Vec<String> = resp_headers
                    .iter()
                    .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                    .map(|(_, v)| v.clone())
                    .collect();
                if !set_cookies.is_empty() {
                    let mut jar = self.cookies.lock().unwrap();
                    for sc in set_cookies {
                        jar.set_cookie_header(&current, &sc, req.top_level);
                    }
                }
            }

            // ---- 304 revalidation -------------------------------------------
            if status == 304 {
                let cached_entry: Option<CacheEntry> = {
                    let mut cache = self.cache.lock().unwrap();
                    cache.revalidations += 1;
                    cache.get(&current.to_string(), now_unix()).and_then(|(e, _)| {
                        let mut merged = (*e).clone();
                        for (k, v) in &resp_headers {
                            if !matches!(k.to_ascii_lowercase().as_str(), "content-length" | "content-encoding") {
                                merged.headers.retain(|(ek, _)| !ek.eq_ignore_ascii_case(k));
                                merged.headers.push((k.clone(), v.clone()));
                            }
                        }
                        merged.stored_at = now_unix();
                        cache.put(merged.clone());
                        Some(merged)
                    })
                };
                if let Some(entry) = cached_entry {
                    total_cache_hit = true;
                    let resp = Response {
                        url: current.clone(),
                        status: entry.status,
                        reason: entry.reason.clone(),
                        headers: entry.headers.clone(),
                        body: entry.body.clone(),
                        from_cache: true,
                        redirects,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    };
                    self.record(NetEntry {
                        id,
                        url: current.to_string(),
                        method: method.clone(),
                        resource_type: req.resource_type,
                        status: entry.status,
                        bytes: entry.body.len(),
                        ms: started.elapsed().as_millis() as u64,
                        from_cache: true,
                        error: None,
                        request_headers: headers,
                        response_headers: entry.headers,
                        started_unix_ms: now_unix() * 1000,
                    });
                    return Ok(resp);
                }
                return Err(NetError::Http("304 without a cached response".into()));
            }

            // ---- redirects ----------------------------------------------------
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                if let Some(loc) = header_get(&resp_headers, "location") {
                    let next = current.resolve(loc);
                    if !redirects.contains(&next.to_string()) {
                        redirects.push(next.to_string());
                    }
                    if status == 303 || ((status == 301 || status == 302) && method != "GET" && method != "HEAD") {
                        method = "GET".to_string();
                        body = None;
                    }
                    current = next;
                    continue;
                }
            }

            // ---- cache store -------------------------------------------------
            if !total_cache_hit && is_cacheable(status, &method, &resp_headers) && req.cache_mode != CacheMode::NoCache {
                let mut store_headers = resp_headers.clone();
                store_headers.retain(|(k, _)| !matches!(k.to_ascii_lowercase().as_str(), "content-encoding" | "content-length" | "transfer-encoding"));
                let entry = build_entry(&current.to_string(), status, &reason, store_headers, resp_body.clone());
                self.cache.lock().unwrap().put(entry);
            }

            let bytes = resp_body.len();
            {
                let mut stats = self.stats.lock().unwrap();
                stats.requests += 1;
                stats.bytes_down += bytes as u64;
                stats.total_ms += started.elapsed().as_millis() as u64;
            }
            self.record(NetEntry {
                id,
                url: current.to_string(),
                method: method.clone(),
                resource_type: req.resource_type,
                status,
                bytes,
                ms: started.elapsed().as_millis() as u64,
                from_cache: false,
                error: None,
                request_headers: headers,
                response_headers: resp_headers.clone(),
                started_unix_ms: now_unix() * 1000,
            });
            let _ = conn_reusable;
            return Ok(Response {
                url: current,
                status,
                reason,
                headers: resp_headers,
                body: resp_body,
                from_cache: false,
                redirects,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }
        Err(NetError::TooManyRedirects)
    }

    fn is_blocked(&self, host: &str) -> bool {
        let blocked = self.blocked_hosts.lock().unwrap();
        blocked.iter().any(|b| {
            host == b
                || host.ends_with(&format!(".{}", b))
        })
    }

    #[allow(clippy::type_complexity)]
    fn send(
        &self,
        url: &Url,
        method: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<(u16, String, Vec<(String, String)>, Vec<u8>, bool)> {
        let key = format!("{}://{}:{}", url.scheme, url.host, url.host_port());
        let mut conn = self.take_conn(&key, url)?;
        let reused = conn.reused;
        let mut request = String::with_capacity(512);
        request.push_str(&format!("{} {} HTTP/1.1\r\n", method, url.request_target()));
        for (k, v) in headers {
            request.push_str(k);
            request.push_str(": ");
            request.push_str(v);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        let status_line = match write_request(&mut conn, &request, body) {
            Ok(line) => line,
            // A pooled keep-alive socket may have been closed by the peer while
            // idle; retry once on a fresh connection before reporting failure.
            Err(_e) if reused => {
                conn = Conn::connect(
                    &url.scheme,
                    &url.host,
                    url.host_port(),
                    &self.tls,
                    &self.dns,
                    self.timeout,
                )
                .map_err(NetError::Io)?;
                write_request(&mut conn, &request, body).map_err(NetError::Io)?
            }
            Err(e) => return Err(NetError::Io(e)),
        };

        let http11 = status_line.starts_with("HTTP/1.1") || status_line.starts_with("HTTP/2");
        let (status, reason) = parse_status_line(&status_line)?;
        let mut resp_headers: Vec<(String, String)> = Vec::new();
        let mut header_bytes = 0usize;
        loop {
            let line = conn.read_line(16 * 1024)?;
            if line.is_empty() {
                break;
            }
            header_bytes += line.len();
            if header_bytes > 256 * 1024 {
                return Err(NetError::Http("response headers too large".into()));
            }
            if let Some((k, v)) = line.split_once(':') {
                resp_headers.push((k.trim().to_string(), v.trim().to_string()));
            }
        }

        let mut raw: Vec<u8> = Vec::new();
        if method != "HEAD" && status != 204 && status != 304 {
            if let Some(te) = header_get(&resp_headers, "transfer-encoding") {
                if te.to_ascii_lowercase().contains("chunked") {
                    read_chunked(&mut conn, &mut raw, self.max_body)?;
                } else {
                    read_length_or_eof(&mut conn, &mut raw, &resp_headers, self.max_body)?;
                }
            } else {
                read_length_or_eof(&mut conn, &mut raw, &resp_headers, self.max_body)?;
            }
        }

        let decoded = decode_content(&raw, header_get(&resp_headers, "content-encoding"))?;

        let connection = header_get(&resp_headers, "connection");
        let connection_close = connection
            .map(|v| v.eq_ignore_ascii_case("close"))
            .unwrap_or(false);
        let connection_keep_alive = connection
            .map(|v| v.eq_ignore_ascii_case("keep-alive"))
            .unwrap_or(false);
        // An HTTP/1.0 response is closed by default unless the server opts in.
        let reusable = (http11 || connection_keep_alive)
            && !connection_close
            && status != 101
            && status != 304;
        if reusable && !conn.is_stale() {
            self.return_conn(key, conn);
        }
        Ok((status, reason, resp_headers, decoded, reusable))
    }

    fn take_conn(&self, key: &str, url: &Url) -> Result<Conn> {
        let mut pool = self.pool.lock().unwrap();
        if let Some(list) = pool.get_mut(key) {
            while let Some(mut c) = list.pop() {
                if c.is_stale() {
                    continue;
                }
                c.reused = true;
                return Ok(c);
            }
        }
        drop(pool);
        Conn::connect(&url.scheme, &url.host, url.host_port(), &self.tls, &self.dns, self.timeout)
            .map_err(NetError::Io)
    }

    fn return_conn(&self, key: String, conn: Conn) {
        let mut pool = self.pool.lock().unwrap();
        let total: usize = pool.values().map(|v| v.len()).sum();
        if total > 32 {
            return;
        }
        let list = pool.entry(key).or_default();
        if list.len() < 4 {
            list.push(conn);
        }
    }

    pub fn close_all_connections(&self) {
        self.pool.lock().unwrap().clear();
    }

    /// Convenience helper used by the engine and tests.
    pub fn get_text(&self, url: &str) -> Result<Response> {
        self.fetch(Request::get(url))
    }
}

impl Default for Client {
    fn default() -> Self {
        Client::new().expect("failed to initialise TLS")
    }
}

fn host_header(url: &Url) -> String {
    let port = url.host_port();
    let default = match url.scheme.as_str() {
        "https" => 443,
        _ => 80,
    };
    if port == default {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, port)
    }
}

fn accept_for(kind: ResourceType) -> String {
    match kind {
        ResourceType::Document => "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".into(),
        ResourceType::StyleSheet => "text/css,*/*;q=0.1".into(),
        ResourceType::Script => "*/*".into(),
        ResourceType::Image => "image/avif,image/webp,image/png,image/jpeg,image/*;q=0.8,*/*;q=0.5".into(),
        ResourceType::Font => "font/woff2,font/woff,font/ttf,*/*;q=0.5".into(),
        ResourceType::Fetch => "*/*".into(),
        ResourceType::Other => "*/*".into(),
    }
}

fn write_request(conn: &mut Conn, request: &str, body: Option<&[u8]>) -> std::io::Result<String> {
    conn.write_all(request.as_bytes())?;
    if let Some(b) = body {
        if !b.is_empty() {
            conn.write_all(b)?;
        }
    }
    conn.requests += 1;
    conn.read_line(8 * 1024)
}

fn parse_status_line(line: &str) -> Result<(u16, String)> {
    let mut parts = line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/") {
        return Err(NetError::Http(format!("malformed status line: {line:?}")));
    }
    let code = parts
        .next()
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| NetError::Http(format!("malformed status code: {line:?}")))?;
    let reason = parts.next().unwrap_or("").trim().to_string();
    Ok((code, reason))
}

fn read_length_or_eof(conn: &mut Conn, out: &mut Vec<u8>, headers: &[(String, String)], limit: usize) -> Result<()> {
    if let Some(len) = header_get(headers, "content-length").and_then(|v| v.parse::<usize>().ok()) {
        if len > limit {
            return Err(NetError::BodyTooLarge);
        }
        out.reserve(len);
        conn.read_exact_n(len, out)?;
    } else {
        conn.read_to_end_limited(out, limit)?;
    }
    Ok(())
}

fn read_chunked(conn: &mut Conn, out: &mut Vec<u8>, limit: usize) -> Result<()> {
    loop {
        let line = conn.read_line(1024)?;
        let size_str = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| NetError::Http(format!("bad chunk size {size_str:?}")))?;
        if size == 0 {
            // trailers
            loop {
                let trailer = conn.read_line(8 * 1024)?;
                if trailer.is_empty() {
                    break;
                }
            }
            return Ok(());
        }
        if out.len() + size > limit {
            return Err(NetError::BodyTooLarge);
        }
        conn.read_exact_n(size, out)?;
        let crlf = conn.read_line(2)?;
        if !crlf.is_empty() {
            return Err(NetError::Http("chunk not terminated by CRLF".into()));
        }
    }
}

fn decode_content(raw: &[u8], encoding: Option<&str>) -> Result<Vec<u8>> {
    let enc = match encoding {
        Some(e) => e.trim().to_ascii_lowercase(),
        None => return Ok(raw.to_vec()),
    };
    if enc.is_empty() || enc == "identity" {
        return Ok(raw.to_vec());
    }
    // Handle stacked encodings ("gzip, br").
    let mut data = raw.to_vec();
    for layer in enc.split(',').rev() {
        let layer = layer.trim();
        let mut out = Vec::new();
        match layer {
            "" | "identity" => {}
            "gzip" | "x-gzip" => {
                GzDecoder::new(data.as_slice())
                    .read_to_end(&mut out)
                    .map_err(|e| NetError::Http(format!("gzip: {e}")))?;
                data = out;
            }
            "deflate" => {
                match ZlibDecoder::new(data.as_slice()).read_to_end(&mut out) {
                    Ok(_) => data = out,
                    Err(_) => {
                        // Some servers send raw deflate.
                        let mut raw_out = Vec::new();
                        flate2::read::DeflateDecoder::new(data.as_slice())
                            .read_to_end(&mut raw_out)
                            .map_err(|e| NetError::Http(format!("deflate: {e}")))?;
                        data = raw_out;
                    }
                }
            }
            other => return Err(NetError::UnsupportedEncoding(other.to_string())),
        }
    }
    Ok(data)
}

/// Parses `Cache-Control`/`Expires` for callers outside the cache.
pub fn response_is_stale(headers: &[(String, String)]) -> bool {
    header_get(headers, "cache-control")
        .map(|v| v.to_ascii_lowercase().contains("no-cache") || v.to_ascii_lowercase().contains("no-store"))
        .unwrap_or(false)
        || header_get(headers, "expires").and_then(parse_http_date).map(|e| e < now_unix()).unwrap_or(false)
}
