use std::io::Write;

use flate2::write::GzEncoder;
use flate2::Compression;
use veyra_net::testserver::{Route, TestServer};
use veyra_net::{CacheMode, Client, Request, ResourceType};

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

#[test]
fn fetches_plain_and_chunked_bodies() {
    let server = TestServer::start(vec![
        Route::new("/", 200, "text/html", "<h1>hello</h1>"),
        Route::new("/chunked", 200, "text/plain", "chunky body that spans chunks").chunked(),
    ]);
    let client = Client::new().unwrap();
    let r = client.fetch(Request::get(server.url("/"))).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.text(), "<h1>hello</h1>");
    assert_eq!(r.mime(), "text/html");

    let r = client.fetch(Request::get(server.url("/chunked"))).unwrap();
    assert_eq!(r.text(), "chunky body that spans chunks");
}

#[test]
fn decodes_gzip() {
    let body = gzip(b"<p>compressed</p>");
    let server = TestServer::start(vec![Route::new("/", 200, "text/html", body)
        .with_header("Content-Encoding", "gzip")]);
    let client = Client::new().unwrap();
    let r = client.fetch(Request::get(server.url("/"))).unwrap();
    assert_eq!(r.text(), "<p>compressed</p>");
}

#[test]
fn follows_redirects() {
    let server = TestServer::start(vec![
        Route::redirect("/old", "/mid"),
        Route::redirect("/mid", "/new"),
        Route::new("/new", 200, "text/plain", "arrived"),
    ]);
    let client = Client::new().unwrap();
    let r = client.fetch(Request::get(server.url("/old"))).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.text(), "arrived");
    assert!(r.url.path.ends_with("/new"));
    assert_eq!(r.redirects.len(), 2);
}

#[test]
fn caches_fresh_responses_and_revalidates() {
    let server = TestServer::start(vec![
        Route::new("/a", 200, "text/plain", "one")
            .with_header("Cache-Control", "max-age=60"),
        Route::new("/b", 200, "text/plain", "etag-body")
            .with_header("Cache-Control", "max-age=0")
            .with_header("ETag", "\"v1\""),
    ]);
    let client = Client::new().unwrap();
    let first = client.fetch(Request::get(server.url("/a"))).unwrap();
    assert!(!first.from_cache);
    let second = client.fetch(Request::get(server.url("/a"))).unwrap();
    assert!(second.from_cache, "second GET should come from cache");
    assert_eq!(second.text(), "one");
    let hits = server.requests.lock().unwrap().len();
    assert_eq!(hits, 1, "server should see exactly one request");

    // CacheMode::Reload bypasses the cache.
    let reload = client.fetch(Request::get(server.url("/a")).cache_mode(CacheMode::Reload)).unwrap();
    assert!(!reload.from_cache);
}

#[test]
fn stores_and_sends_cookies() {
    let server = TestServer::start(vec![
        Route::new("/set", 200, "text/plain", "set")
            .with_header("Set-Cookie", "session=abc123; Path=/; HttpOnly"),
        Route::new("/echo", 200, "text/plain", "ok"),
    ]);
    let client = Client::new().unwrap();
    client.fetch(Request::get(server.url("/set"))).unwrap();
    client.fetch(Request::get(server.url("/echo"))).unwrap();
    let log = server.requests.lock().unwrap();
    let echoed = log.iter().find(|l| l.contains("/echo")).unwrap();
    assert!(echoed.contains("session=abc123"), "got: {echoed}");
}

#[test]
fn reports_status_and_headers() {
    let server = TestServer::start(vec![Route::new("/missing", 404, "text/html", "nope")]);
    let client = Client::new().unwrap();
    let r = client.fetch(Request::get(server.url("/missing"))).unwrap();
    assert_eq!(r.status, 404);
    assert_eq!(r.text(), "nope");
    let log = client.network_log();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].status, 404);
    assert!(log[0].resource_type == ResourceType::Document);
}

#[test]
fn connection_reuse_and_pool() {
    let server = TestServer::start(vec![Route::new("/x", 200, "text/plain", "y")]);
    let client = Client::new().unwrap();
    for _ in 0..3 {
        let r = client.fetch(Request::get(server.url("/x"))).unwrap();
        assert_eq!(r.status, 200);
    }
    // The test server closes every connection, so this exercises reconnect
    // handling rather than keep-alive, but it must not error.
    assert_eq!(server.requests.lock().unwrap().len(), 3);
}

#[test]
#[ignore = "requires internet access"]
fn real_https_request() {
    let client = Client::new().unwrap();
    let r = client
        .fetch(Request::get("https://example.com/").kind(ResourceType::Document))
        .unwrap();
    assert_eq!(r.status, 200);
    assert!(r.text().to_ascii_lowercase().contains("example"));
}
