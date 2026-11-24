//! URL parsing and resolution (RFC 3986), plus the origin model used by the
//! same-origin policy.

use std::fmt;

use crate::util;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Url {
    pub scheme: String,
    pub username: String,
    pub password: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

impl Url {
    /// Parses an absolute URL. Relative input yields `None`; use
    /// [`Url::resolve`] for those.
    pub fn parse(input: &str) -> Option<Url> {
        let mut s = input.trim();
        // Strip ASCII whitespace/newlines which browsers tolerate.
        if s.bytes().any(|b| b == b'\n' || b == b'\r' || b == b'\t') {
            let cleaned: String = s.chars().filter(|c| !matches!(c, '\n' | '\r' | '\t')).collect();
            return Url::parse(&cleaned);
        }
        let colon = s.find(':')?;
        let scheme = &s[..colon];
        if !is_valid_scheme(scheme) {
            return None;
        }
        let scheme = scheme.to_ascii_lowercase();
        s = &s[colon + 1..];
        let mut url = Url {
            scheme: scheme.clone(),
            ..Default::default()
        };
        let hierarchical = matches!(
            scheme.as_str(),
            "http" | "https" | "ws" | "wss" | "ftp" | "file"
        );
        if hierarchical {
            let mut rest = s;
            if let Some(stripped) = rest.strip_prefix("//") {
                rest = stripped;
                let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
                let authority = &rest[..end];
                rest = &rest[end..];
                let mut hostport = authority;
                if let Some(at) = authority.rfind('@') {
                    let userinfo = &authority[..at];
                    hostport = &authority[at + 1..];
                    if let Some((u, p)) = userinfo.split_once(':') {
                        url.username = util::percent_decode(u);
                        url.password = util::percent_decode(p);
                    } else {
                        url.username = util::percent_decode(userinfo);
                    }
                }
                if let Some(stripped) = hostport.strip_prefix('[') {
                    // IPv6 literal: keep the brackets in the serialisation.
                    let close = stripped.find(']')?;
                    url.host = format!("[{}]", &stripped[..close]);
                    let after = &stripped[close + 1..];
                    if let Some(p) = after.strip_prefix(':') {
                        url.port = Some(p.parse().ok()?);
                    }
                } else if let Some((h, p)) = hostport.rsplit_once(':') {
                    if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
                        url.host = h.to_ascii_lowercase();
                        url.port = Some(p.parse().ok()?);
                    } else {
                        url.host = hostport.to_ascii_lowercase();
                    }
                } else {
                    url.host = hostport.to_ascii_lowercase();
                }
                if url.host.is_empty() && scheme != "file" {
                    return None;
                }
            }
            let (rest, frag) = match rest.split_once('#') {
                Some((a, b)) => (a, Some(b.to_string())),
                None => (rest, None),
            };
            let (rest, query) = match rest.split_once('?') {
                Some((a, b)) => (a, Some(b.to_string())),
                None => (rest, None),
            };
            url.path = if rest.is_empty() { "/".to_string() } else { rest.to_string() };
            url.query = query;
            url.fragment = frag;
            // Drop default ports so origins compare cleanly.
            if (scheme == "http" && url.port == Some(80))
                || (scheme == "https" && url.port == Some(443))
                || (scheme == "ws" && url.port == Some(80))
                || (scheme == "wss" && url.port == Some(443))
            {
                url.port = None;
            }
        } else {
            // Opaque scheme (data:, mailto:, about:, javascript:).
            let (rest, frag) = match s.split_once('#') {
                Some((a, b)) => (a, Some(b.to_string())),
                None => (s, None),
            };
            url.path = rest.to_string();
            url.fragment = frag;
        }
        Some(url)
    }

    /// Resolves `reference` against this URL. Absolute references are parsed
    /// directly; malformed input falls back to the base URL's scheme/host.
    pub fn resolve(&self, reference: &str) -> Url {
        let r = reference.trim();
        if r.is_empty() {
            let mut u = self.clone();
            u.fragment = None;
            return u;
        }
        if let Some(c) = r.find(':') {
            let maybe_scheme = &r[..c];
            if is_valid_scheme(maybe_scheme) && !maybe_scheme.contains('/') {
                if let Some(u) = Url::parse(r) {
                    return u;
                }
            }
        }
        if let Some(rest) = r.strip_prefix("//") {
            return Url::parse(&format!("{}://{}", self.scheme, rest)).unwrap_or_else(|| self.clone());
        }
        let mut u = Url {
            scheme: self.scheme.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            host: self.host.clone(),
            port: self.port,
            path: self.path.clone(),
            query: None,
            fragment: None,
        };
        let (rest, frag) = match r.split_once('#') {
            Some((a, b)) => (a, Some(b.to_string())),
            None => (r, None),
        };
        let (rest, query) = match rest.split_once('?') {
            Some((a, b)) => (a, Some(b.to_string())),
            None => (rest, None),
        };
        if rest.is_empty() {
            u.query = query.or_else(|| self.query.clone());
        } else if rest.starts_with('/') {
            u.path = remove_dot_segments(rest);
            u.query = query;
        } else if rest.starts_with('\\') && self.is_special() {
            u.path = remove_dot_segments(&rest.replace('\\', "/"));
            u.query = query;
        } else {
            let base_dir = match self.path.rfind('/') {
                Some(i) => &self.path[..i + 1],
                None => "/",
            };
            u.path = remove_dot_segments(&format!("{}{}", base_dir, rest));
            u.query = query;
        }
        u.fragment = frag;
        u
    }

    pub fn is_special(&self) -> bool {
        matches!(self.scheme.as_str(), "http" | "https" | "ws" | "wss" | "ftp" | "file")
    }

    pub fn is_http(&self) -> bool {
        self.scheme == "http" || self.scheme == "https"
    }

    pub fn host_port(&self) -> u16 {
        self.port.unwrap_or(match self.scheme.as_str() {
            "https" | "wss" => 443,
            _ => 80,
        })
    }

    pub fn origin(&self) -> Option<Origin> {
        if self.host.is_empty() {
            return None;
        }
        Some(Origin {
            scheme: self.scheme.clone(),
            host: self.host.clone(),
            port: self.host_port(),
        })
    }

    pub fn same_origin(&self, other: &Url) -> bool {
        match (self.origin(), other.origin()) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// Path + query, as used by HTTP requests.
    pub fn request_target(&self) -> String {
        let mut t = if self.path.is_empty() { "/".to_string() } else { self.path.clone() };
        if let Some(q) = &self.query {
            t.push('?');
            t.push_str(q);
        }
        t
    }

    pub fn without_fragment(&self) -> Url {
        let mut u = self.clone();
        u.fragment = None;
        u
    }

    pub fn file_extension(&self) -> Option<String> {
        let name = self.path.rsplit('/').next()?;
        name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())
    }

    pub fn query_pairs(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(q) = &self.query {
            for pair in q.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (k, v) = match pair.split_once('=') {
                    Some((k, v)) => (k, v),
                    None => (pair, ""),
                };
                out.push((util::percent_decode(&k.replace('+', " ")), util::percent_decode(&v.replace('+', " "))));
            }
        }
        out
    }
}

fn is_valid_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

pub fn remove_dot_segments(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let trailing_slash = path.ends_with('/') || path.ends_with("/.") || path.ends_with("/..");
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    let mut s = String::from("/");
    s.push_str(&out.join("/"));
    if trailing_slash && !s.ends_with('/') {
        s.push('/');
    }
    s
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.scheme)?;
        if self.is_special() {
            if !self.host.is_empty() {
                write!(f, "//")?;
                if !self.username.is_empty() {
                    write!(f, "{}", self.username)?;
                    if !self.password.is_empty() {
                        write!(f, ":{}", self.password)?;
                    }
                    write!(f, "@")?;
                }
                write!(f, "{}", self.host)?;
                if let Some(p) = self.port {
                    write!(f, ":{}", p)?;
                }
            }
            write!(f, "{}", if self.path.is_empty() { "/" } else { &self.path })?;
        } else {
            write!(f, "{}", self.path)?;
        }
        if let Some(q) = &self.query {
            write!(f, "?{}", q)?;
        }
        if let Some(fr) = &self.fragment {
            write!(f, "#{}", fr)?;
        }
        Ok(())
    }
}

impl std::str::FromStr for Url {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Url::parse(s).ok_or(())
    }
}

/// Display form for the address bar: no credentials, no fragment.
pub fn display_url(url: &Url) -> String {
    let mut u = url.without_fragment();
    u.username.clear();
    u.password.clear();
    u.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_absolute() {
        let u = Url::parse("HTTPS://Example.COM:443/a/b?q=1#frag").unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, None);
        assert_eq!(u.path, "/a/b");
        assert_eq!(u.query.as_deref(), Some("q=1"));
        assert_eq!(u.fragment.as_deref(), Some("frag"));
        assert_eq!(u.to_string(), "https://example.com/a/b?q=1#frag");
    }

    #[test]
    fn resolves_relative() {
        let base = Url::parse("https://example.com/dir/page.html?x=1").unwrap();
        assert_eq!(base.resolve("other.html").to_string(), "https://example.com/dir/other.html");
        assert_eq!(base.resolve("../up.html").to_string(), "https://example.com/up.html");
        assert_eq!(base.resolve("/root").to_string(), "https://example.com/root");
        assert_eq!(base.resolve("//cdn.example.org/x.js").to_string(), "https://cdn.example.org/x.js");
        assert_eq!(base.resolve("#top").to_string(), "https://example.com/dir/page.html?x=1#top");
        assert_eq!(base.resolve("?y=2").to_string(), "https://example.com/dir/page.html?y=2");
        assert_eq!(base.resolve("mailto:a@b.c").to_string(), "mailto:a@b.c");
    }

    #[test]
    fn origins() {
        let a = Url::parse("https://example.com/x").unwrap();
        let b = Url::parse("https://example.com:443/y").unwrap();
        let c = Url::parse("http://example.com/y").unwrap();
        assert!(a.same_origin(&b));
        assert!(!a.same_origin(&c));
    }

    #[test]
    fn dot_segments() {
        assert_eq!(remove_dot_segments("/a/b/../c/./d"), "/a/c/d");
        assert_eq!(remove_dot_segments("/a/b/.."), "/a/");
    }
}
