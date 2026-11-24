//! Small byte/string helpers shared across subsystems.

use std::collections::HashMap;

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn percent_encode(input: &str, extra_safe: &[u8]) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        let safe = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || extra_safe.contains(&b);
        if safe {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(cleaned.len() * 3 / 4);
    let mut chunk = [0u8; 4];
    let mut n = 0;
    for b in cleaned {
        chunk[n] = val(b)?;
        n += 1;
        if n == 4 {
            out.push((chunk[0] << 2) | (chunk[1] >> 4));
            out.push((chunk[1] << 4) | (chunk[2] >> 2));
            out.push((chunk[2] << 6) | chunk[3]);
            n = 0;
        }
    }
    match n {
        2 => out.push((chunk[0] << 2) | (chunk[1] >> 4)),
        3 => {
            out.push((chunk[0] << 2) | (chunk[1] >> 4));
            out.push((chunk[1] << 4) | (chunk[2] >> 2));
        }
        1 => return None,
        _ => {}
    }
    Some(out)
}

pub fn base64_encode(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(T[(n >> 6) as usize & 63] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(T[n as usize & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Decoded payload of a `data:` URL plus its media type.
pub struct DataUrl {
    pub mime: String,
    pub bytes: Vec<u8>,
}

pub fn parse_data_url(url: &str) -> Option<DataUrl> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let (mime, is_base64) = match meta.rsplit_once(";base64") {
        Some((m, _)) => (m, true),
        None => (meta, false),
    };
    let mime = if mime.is_empty() { "text/plain".to_string() } else { mime.to_string() };
    let bytes = if is_base64 {
        base64_decode(data)?
    } else {
        percent_decode(data).into_bytes()
    };
    Some(DataUrl { mime, bytes })
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", n, UNITS[0])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

/// Case-insensitive ASCII header lookup.
pub fn header_get<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

pub fn parse_http_date(input: &str) -> Option<u64> {
    // Supports IMF-fixdate ("Sun, 06 Nov 1994 08:49:37 GMT").
    let parts: Vec<&str> = input.split([' ', ',', ':']).filter(|p| !p.is_empty()).collect();
    if parts.len() < 7 {
        return None;
    }
    let day: u64 = parts[1].parse().ok()?;
    let month = match parts[2].to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    };
    let year: u64 = parts[3].parse().ok()?;
    let (h, m, s): (u64, u64, u64) = (parts[4].parse().ok()?, parts[5].parse().ok()?, parts[6].parse().ok()?);
    Some(days_from_civil(year as i64, month, day as i64) * 86400 + h * 3600 + m * 60 + s)
}

fn days_from_civil(y: i64, m: u64, d: i64) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe as i64 - 719468) as u64
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in data {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Generic LRU-ish cache keyed by FNV hash of a string.
pub struct ByteCache {
    entries: HashMap<u64, (String, Vec<u8>)>,
    order: Vec<u64>,
    capacity: usize,
    pub bytes: usize,
}

impl ByteCache {
    pub fn new(capacity: usize) -> Self {
        ByteCache { entries: HashMap::new(), order: Vec::new(), capacity, bytes: 0 }
    }
    pub fn get(&mut self, key: &str) -> Option<&[u8]> {
        let k = fnv1a(key.as_bytes());
        if self.entries.contains_key(&k) {
            self.order.retain(|x| *x != k);
            self.order.push(k);
            self.entries.get(&k).map(|(_, v)| v.as_slice())
        } else {
            None
        }
    }
    pub fn put(&mut self, key: &str, value: Vec<u8>) {
        let k = fnv1a(key.as_bytes());
        if let Some((_, old)) = self.entries.insert(k, (key.to_string(), value)) {
            self.bytes = self.bytes.saturating_sub(old.len());
        }
        self.bytes += self.entries[&k].1.len();
        self.order.push(k);
        while self.entries.len() > self.capacity && !self.order.is_empty() {
            let evict = self.order.remove(0);
            if let Some((_, v)) = self.entries.remove(&evict) {
                self.bytes = self.bytes.saturating_sub(v.len());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for s in ["", "a", "ab", "abc", "hello world!"] {
            assert_eq!(base64_decode(&base64_encode(s.as_bytes())).unwrap(), s.as_bytes());
        }
    }

    #[test]
    fn data_urls() {
        let d = parse_data_url("data:text/html;base64,PGgxPmhpPC9oMT4=").unwrap();
        assert_eq!(d.mime, "text/html");
        assert_eq!(String::from_utf8(d.bytes).unwrap(), "<h1>hi</h1>");
        let d = parse_data_url("data:,hello%20world").unwrap();
        assert_eq!(String::from_utf8(d.bytes).unwrap(), "hello world");
    }

    #[test]
    fn http_dates() {
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"), Some(784111777));
    }
}
