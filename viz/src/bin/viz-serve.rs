//! Minimal static file server for viz/web (development only).
//!
//! Usage: cargo run -p logic_circuit_viz --bin viz-serve [-- PORT]
//! Serves on 127.0.0.1 with caching disabled so rebuilt WASM is picked up.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::thread;

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Maps a request path onto `root`, rejecting anything that leaves it.
fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let path = url_path.split(['?', '#']).next().unwrap_or("/");
    let rel = Path::new(path.trim_start_matches('/'));
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    let mut full = root.join(rel);
    if full.is_dir() {
        full.push("index.html");
    }
    Some(full)
}

fn respond(mut stream: TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn handle(stream: TcpStream, root: &Path) {
    let mut request_line = String::new();
    if BufReader::new(&stream).read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(url)) = (parts.next(), parts.next()) else {
        return;
    };
    if method != "GET" {
        return respond(stream, "405 Method Not Allowed", "text/plain", b"GET only");
    }
    match resolve(root, url).and_then(|p| fs::read(&p).ok().map(|b| (p, b))) {
        Some((path, body)) => respond(stream, "200 OK", content_type(&path), &body),
        None => respond(stream, "404 Not Found", "text/plain", b"not found"),
    }
}

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(8173);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web");
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("cannot bind port");
    println!("Serving {} on http://127.0.0.1:{port}", root.display());
    for stream in listener.incoming().flatten() {
        let root = root.clone();
        thread::spawn(move || handle(stream, &root));
    }
}
