//! TCP + TLS connection handling with a small buffered reader that supports
//! connection reuse (we never read past the end of a response body).

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::{ClientConnection, StreamOwned};

use crate::dns::DnsCache;

pub enum Transport {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Transport {
    pub fn tcp(&self) -> &TcpStream {
        match self {
            Transport::Plain(t) => t,
            Transport::Tls(s) => s.get_ref(),
        }
    }
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Transport::Plain(t) => t.read(buf),
            Transport::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Transport::Plain(t) => t.write(buf),
            Transport::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Transport::Plain(t) => t.flush(),
            Transport::Tls(s) => s.flush(),
        }
    }
}

/// A buffered connection. `read_line` and `read_exact_n` consume from an
/// internal buffer so a response body can be delimited exactly, which makes
/// keep-alive safe.
pub struct Conn {
    transport: Transport,
    buf: Vec<u8>,
    pos: usize,
    pub origin: String,
    pub reused: bool,
    pub created: Instant,
    pub requests: u32,
}

impl Conn {
    pub fn connect(
        scheme: &str,
        host: &str,
        port: u16,
        tls: &Arc<rustls::ClientConfig>,
        dns: &DnsCache,
        timeout: Duration,
    ) -> io::Result<Conn> {
        let addrs = dns.resolve(host, port)?;
        let tcp = connect_any(&addrs, timeout)?;
        tcp.set_read_timeout(Some(timeout))?;
        tcp.set_write_timeout(Some(timeout))?;
        let _ = tcp.set_nodelay(true);
        let transport = if scheme == "https" {
            let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("bad server name: {e}")))?;
            let conn = ClientConnection::new(tls.clone(), server_name)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            Transport::Tls(Box::new(StreamOwned::new(conn, tcp)))
        } else {
            Transport::Plain(tcp)
        };
        Ok(Conn {
            transport,
            buf: Vec::with_capacity(16 * 1024),
            pos: 0,
            origin: format!("{}://{}:{}", scheme, host, port),
            reused: false,
            created: Instant::now(),
            requests: 0,
        })
    }

    fn available(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn fill(&mut self) -> io::Result<usize> {
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        if self.buf.len() == self.buf.capacity() {
            self.buf.reserve(16 * 1024);
        }
        let old_len = self.buf.len();
        self.buf.resize(old_len + 16 * 1024, 0);
        let n = self.transport.read(&mut self.buf[old_len..])?;
        self.buf.truncate(old_len + n);
        Ok(n)
    }

    /// Reads a CRLF-terminated line (the CRLF is stripped).
    pub fn read_line(&mut self, limit: usize) -> io::Result<String> {
        loop {
            if let Some(idx) = find_crlf(&self.buf[self.pos..]) {
                let line = &self.buf[self.pos..self.pos + idx];
                let s = String::from_utf8_lossy(line).into_owned();
                self.pos += idx + 2;
                if s.len() > limit {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
                }
                return Ok(s);
            }
            if self.available() > limit {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
            }
            if self.fill()? == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"));
            }
        }
    }

    pub fn read_exact_n(&mut self, n: usize, out: &mut Vec<u8>) -> io::Result<()> {
        let mut remaining = n;
        while remaining > 0 {
            if self.available() == 0 && self.fill()? == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short body"));
            }
            let take = remaining.min(self.available());
            out.extend_from_slice(&self.buf[self.pos..self.pos + take]);
            self.pos += take;
            remaining -= take;
        }
        Ok(())
    }

    pub fn read_to_end_limited(&mut self, out: &mut Vec<u8>, limit: usize) -> io::Result<()> {
        loop {
            if self.available() > 0 {
                let take = self.available();
                if out.len() + take > limit {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "response too large"));
                }
                out.extend_from_slice(&self.buf[self.pos..self.pos + take]);
                self.pos += take;
                continue;
            }
            if self.fill()? == 0 {
                return Ok(());
            }
            if out.len() > limit {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "response too large"));
            }
        }
    }

    pub fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.transport.write_all(data)?;
        self.transport.flush()
    }

    pub fn is_stale(&self) -> bool {
        self.created.elapsed() > Duration::from_secs(55) || self.requests > 100
    }
}

fn find_crlf(haystack: &[u8]) -> Option<usize> {
    haystack.windows(2).position(|w| w == b"\r\n")
}

fn connect_any(addrs: &[SocketAddr], timeout: Duration) -> io::Result<TcpStream> {
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(addr, timeout) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "no addresses for host")))
}

pub fn resolve_host(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    (host, port).to_socket_addrs().map(|it| it.collect())
}
