//! A tiny loopback HTTP server used by the engine's tests and the
//! compatibility harness. Not part of the browser's network stack.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Clone)]
pub struct Route {
    pub path: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub chunked: bool,
}

impl Route {
    pub fn new(path: &str, status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Route {
        Route {
            path: path.to_string(),
            status,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: body.into(),
            chunked: false,
        }
    }

    pub fn redirect(path: &str, location: &str) -> Route {
        Route {
            path: path.into(),
            status: 302,
            headers: vec![("Location".into(), location.into())],
            body: vec![],
            chunked: false,
        }
    }

    pub fn chunked(mut self) -> Route {
        self.chunked = true;
        self
    }

    pub fn with_header(mut self, k: &str, v: &str) -> Route {
        self.headers.push((k.into(), v.into()));
        self
    }
}

pub struct TestServer {
    pub addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    pub requests: Arc<Mutex<Vec<String>>>,
}

impl TestServer {
    pub fn start(routes: Vec<Route>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let routes = Arc::new(routes);
        let stop = shutdown.clone();
        let log = requests.clone();
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                match stream {
                    Ok(s) => {
                        let _ = handle_conn(s, &routes, &log);
                    }
                    Err(_) => break,
                }
            }
        });
        TestServer { addr, shutdown, handle: Some(handle), requests }
    }

    pub fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.origin(), path)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn handle_conn(mut stream: TcpStream, routes: &[Route], log: &Mutex<Vec<String>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let first = head.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let cookie = head
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("cookie:"))
        .map(|l| l[7..].trim().to_string());
    log.lock().unwrap().push(format!("{} {} cookie={:?}", method, target, cookie));

    let path = target.split('?').next().unwrap_or("/");
    let route = routes.iter().find(|r| r.path == path).cloned().unwrap_or_else(|| {
        Route::new("/", 404, "text/html", "<h1>404 Not Found</h1>")
    });
    let mut response = format!("HTTP/1.1 {} {}\r\n", route.status, status_text(route.status));
    for (k, v) in &route.headers {
        response.push_str(&format!("{}: {}\r\n", k, v));
    }
    if route.chunked {
        response.push_str("Transfer-Encoding: chunked\r\n");
    } else {
        response.push_str(&format!("Content-Length: {}\r\n", route.body.len()));
    }
    response.push_str("Connection: close\r\n\r\n");
    stream.write_all(response.as_bytes())?;
    if route.chunked {
        for chunk in route.body.chunks(7) {
            stream.write_all(format!("{:x}\r\n", chunk.len()).as_bytes())?;
            stream.write_all(chunk)?;
            stream.write_all(b"\r\n")?;
        }
        stream.write_all(b"0\r\n\r\n")?;
    } else {
        stream.write_all(&route.body)?;
    }
    stream.flush()?;
    Ok(())
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}
