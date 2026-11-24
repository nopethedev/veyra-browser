//! DNS resolution with a TTL cache and `/etc/hosts`-style overrides.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::conn::resolve_host;

struct Entry {
    addrs: Vec<SocketAddr>,
    at: Instant,
}

pub struct DnsCache {
    entries: Mutex<HashMap<String, Entry>>,
    overrides: HashMap<String, IpAddr>,
    ttl: Duration,
    pub lookups: Mutex<u64>,
    pub hits: Mutex<u64>,
}

impl DnsCache {
    pub fn new() -> Self {
        DnsCache {
            entries: Mutex::new(HashMap::new()),
            overrides: HashMap::new(),
            ttl: Duration::from_secs(60),
            lookups: Mutex::new(0),
            hits: Mutex::new(0),
        }
    }

    pub fn add_override(&mut self, host: &str, ip: IpAddr) {
        self.overrides.insert(host.to_ascii_lowercase(), ip);
    }

    /// Loads simple `IP hostname...` records from a hosts file.
    pub fn load_hosts_file(&mut self, path: &str) {
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                let line = line.split('#').next().unwrap_or("").trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                if let Some(ip) = parts.next().and_then(|ip| ip.parse::<IpAddr>().ok()) {
                    for host in parts {
                        self.overrides.entry(host.to_ascii_lowercase()).or_insert(ip);
                    }
                }
            }
        }
    }

    pub fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        let key = host.to_ascii_lowercase();
        if let Some(ip) = self.overrides.get(&key) {
            return Ok(vec![SocketAddr::new(*ip, port)]);
        }
        {
            let cache = self.entries.lock().unwrap();
            if let Some(e) = cache.get(&key) {
                if e.at.elapsed() < self.ttl {
                    *self.hits.lock().unwrap() += 1;
                    return Ok(e.addrs.iter().map(|a| SocketAddr::new(a.ip(), port)).collect());
                }
            }
        }
        *self.lookups.lock().unwrap() += 1;
        let addrs = resolve_host(host, port)?;
        if addrs.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no addresses"));
        }
        self.entries.lock().unwrap().insert(key, Entry { addrs: addrs.clone(), at: Instant::now() });
        Ok(addrs)
    }
}

impl Default for DnsCache {
    fn default() -> Self {
        Self::new()
    }
}
