//! Development server for viz/web, with a small API to build chip layouts
//! from the browser.
//!
//! Usage: cargo run -p logic_circuit_viz --bin viz-serve [-- PORT]
//! Serves on 127.0.0.1 with caching disabled so rebuilt WASM is picked up.
//!
//! API (JSON unless noted):
//! - `GET  /api/health`                  -> {"ok":true}
//! - `GET  /api/chip/runs?name=N`        -> runs of design N (metrics, layout present?)
//! - `GET  /api/chip/layout?name=N&run=R` -> layout.bin of a run (extracted on demand)
//! - `POST /api/chip/build?name=N&max_layer=met5&level=basic&util=40&optimize=0&keep_latches=0&clock=10&mem_delay=0`
//!   starts `chip build` + `chip extract` in the background (one job at a time)
//! - `GET  /api/chip/job`                -> state of the current/last job, with the log tail
//!
//! Requests that change something must carry the header `X-LogicCircuit: 1`:
//! a cross-site page cannot set it without a CORS preflight, which this
//! server never approves, so other websites cannot start builds.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use logic_circuit::chip::{self, BuildOptions};
use logic_circuit::cldl::{CircuitBuilder, FlatLevel};
use serde_json::{Value, json};

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
fn resolve(root: &Path, path: &str) -> Option<PathBuf> {
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

fn respond_json(stream: TcpStream, status: &str, v: &Value) {
    respond(stream, status, "application/json", v.to_string().as_bytes());
}

fn query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.replace("%5B", "[").replace("%5D", "]")))
        .collect()
}

/// State of the background build job.
#[derive(Default)]
struct Job {
    id: u64,
    state: &'static str, // "", "running", "done", "error"
    name: String,
    run: String,
    started: Option<Instant>,
    elapsed_s: f64,
    error: String,
}

struct Server {
    web: PathBuf,
    /// Repository root (chip/, circuits/).
    root: PathBuf,
    job: Mutex<Job>,
}

impl Server {
    fn builder(&self) -> CircuitBuilder {
        let mut b = CircuitBuilder::new();
        let _ = b.load_circuits_from_path(&self.root.join("circuits"));
        b
    }

    /// Design names are limited to known circuits; run names to a safe charset.
    fn check_name(&self, name: &str) -> Result<(), String> {
        if self.builder().circuit_names().iter().any(|n| n == name) {
            Ok(())
        } else {
            Err(format!("unknown circuit '{name}'"))
        }
    }
}

fn safe_run(run: &str) -> bool {
    !run.is_empty() && run.len() < 80 && run.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && !run.contains("..")
}

fn runs_json(server: &Server, name: &str) -> Value {
    let dir = chip::chip_dir(&server.root, name);
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            let run = d.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            let has_layout = d.join("layout.bin").is_file();
            let has_def = fs::read_dir(&d).map(|es| es.flatten().any(|e| e.path().extension().is_some_and(|x| x == "def"))).unwrap_or(false);
            if !has_layout && !has_def {
                continue; // failed or unfinished build
            }
            let metrics = chip::read_metrics(&d)
                .ok()
                .and_then(|m| serde_json::from_str::<Value>(&chip::to_json(name, &[m])).ok())
                .and_then(|v| v["runs"].get(0).cloned());
            out.push(json!({ "run": run, "layout": has_layout, "own": run.starts_with("own-"), "metrics": metrics }));
        }
    }
    json!({ "name": name, "runs": out })
}

fn layout_bytes(server: &Server, name: &str, run: &str) -> Result<Vec<u8>, String> {
    let dir = chip::chip_dir(&server.root, name).join(run);
    let file = dir.join("layout.bin");
    if !file.is_file() {
        let (layout, _) = chip::extract(&server.root, &dir)?;
        fs::write(&file, layout.write_binary()).map_err(|e| e.to_string())?;
    }
    fs::read(&file).map_err(|e| e.to_string())
}

fn start_build(server: &Arc<Server>, q: &HashMap<String, String>) -> Result<Value, String> {
    let name = q.get("name").cloned().unwrap_or_default();
    server.check_name(&name)?;
    let max_layer = q.get("max_layer").cloned().unwrap_or_else(|| "met5".into());
    if !["met2", "met3", "met4", "met5"].contains(&max_layer.as_str()) {
        return Err(format!("max_layer must be met2..met5, got '{max_layer}'"));
    }
    let level = match q.get("level").map(String::as_str) {
        Some("nand") => FlatLevel::Nand,
        _ => FlatLevel::Basic,
    };
    let utilization: u32 = q.get("util").and_then(|v| v.parse().ok()).unwrap_or(chip::DEFAULT_UTIL);
    if !(10..=90).contains(&utilization) {
        return Err("util must be 10..90".into());
    }
    let clock_period_ns: f64 = q.get("clock").and_then(|v| v.parse().ok()).unwrap_or(10.0);
    if !(1.0..=100.0).contains(&clock_period_ns) {
        return Err("clock must be 1..100 ns".into());
    }
    let mem_delay_ns: f64 = q.get("mem_delay").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    if !(0.0..=50.0).contains(&mem_delay_ns) {
        return Err("mem_delay must be 0..50 ns".into());
    }
    let opts = BuildOptions {
        level,
        keep_latches: q.get("keep_latches").is_some_and(|v| v == "1"),
        max_layer,
        clock_period_ns,
        utilization,
        die_size_um: None,
        optimize: q.get("optimize").is_some_and(|v| v == "1"),
        mem_delay_ns,
    };
    let run = opts.run_name();
    let id = {
        let mut job = server.job.lock().unwrap();
        if job.state == "running" {
            return Err(format!("a build is already running ({} / {})", job.name, job.run));
        }
        *job = Job { id: job.id + 1, state: "running", name: name.clone(), run: run.clone(), started: Some(Instant::now()), ..Default::default() };
        job.id
    };
    let server = Arc::clone(server);
    let response = json!({ "job": id, "run": run });
    thread::spawn(move || {
        let builder = server.builder();
        let result = chip::build(&builder, &name, &opts, &server.root).and_then(|_| {
            let dir = chip::chip_dir(&server.root, &name).join(&run);
            let (layout, checks) = chip::extract(&server.root, &dir)?;
            fs::write(dir.join("layout.bin"), layout.write_binary()).map_err(|e| e.to_string())?;
            if !checks.missing_cells.is_empty() || !checks.open_nets.is_empty() {
                return Err(format!(
                    "layout checks failed: {} cells missing, {} open nets",
                    checks.missing_cells.len(),
                    checks.open_nets.len()
                ));
            }
            Ok(())
        });
        let mut job = server.job.lock().unwrap();
        job.elapsed_s = job.started.map_or(0.0, |s| s.elapsed().as_secs_f64());
        match result {
            Ok(()) => job.state = "done",
            Err(e) => {
                job.state = "error";
                job.error = e;
            }
        }
    });
    Ok(response)
}

/// State of the current/last job, with the last informative line of its
/// OpenROAD log (`step`) and the last lines of the log (`log`).
fn job_json(server: &Server) -> Value {
    let job = server.job.lock().unwrap();
    let elapsed = match job.state {
        "running" => job.started.map_or(0.0, |s| s.elapsed().as_secs_f64()),
        _ => job.elapsed_s,
    };
    let log = chip::chip_dir(&server.root, &job.name).join(&job.run).join("pnr.log");
    let text = fs::read_to_string(&log).unwrap_or_default();
    let step = text
        .lines()
        .rev()
        .find(|x| x.starts_with("[INFO"))
        .map(|x| x.chars().take(140).collect::<String>())
        .unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail: Vec<String> = lines[lines.len().saturating_sub(14)..]
        .iter()
        .map(|l| l.chars().take(160).collect())
        .collect();
    json!({
        "id": job.id, "state": job.state, "name": job.name, "run": job.run,
        "elapsed_s": elapsed, "step": step, "log": tail, "error": job.error,
    })
}

fn handle(stream: TcpStream, server: Arc<Server>) {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut headers: HashMap<String, String> = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(url)) = (parts.next(), parts.next()) else {
        return;
    };
    let (path, q) = url.split_once('?').unwrap_or((url, ""));
    let path = path.split('#').next().unwrap_or("/");
    let q = query(q);

    if let Some(api) = path.strip_prefix("/api/") {
        // Changes need our custom header (blocks cross-site requests).
        let trusted = headers.get("x-logiccircuit").map(String::as_str) == Some("1");
        match (method, api) {
            ("GET", "health") => respond_json(stream, "200 OK", &json!({ "ok": true })),
            ("GET", "chip/job") => respond_json(stream, "200 OK", &job_json(&server)),
            ("GET", "chip/runs") => {
                let name = q.get("name").cloned().unwrap_or_default();
                match server.check_name(&name) {
                    Ok(()) => respond_json(stream, "200 OK", &runs_json(&server, &name)),
                    Err(e) => respond_json(stream, "400 Bad Request", &json!({ "error": e })),
                }
            }
            ("GET", "chip/layout") => {
                let name = q.get("name").cloned().unwrap_or_default();
                let run = q.get("run").cloned().unwrap_or_default();
                let result = server.check_name(&name).and_then(|_| {
                    if safe_run(&run) { layout_bytes(&server, &name, &run) } else { Err("invalid run".into()) }
                });
                match result {
                    Ok(bytes) => respond(stream, "200 OK", "application/octet-stream", &bytes),
                    Err(e) => respond_json(stream, "404 Not Found", &json!({ "error": e })),
                }
            }
            ("POST", "chip/build") if trusted => match start_build(&server, &q) {
                Ok(v) => respond_json(stream, "202 Accepted", &v),
                Err(e) => respond_json(stream, "409 Conflict", &json!({ "error": e })),
            },
            ("POST", _) => respond_json(stream, "403 Forbidden", &json!({ "error": "missing X-LogicCircuit header" })),
            _ => respond_json(stream, "404 Not Found", &json!({ "error": "unknown API" })),
        }
        return;
    }

    if method != "GET" {
        return respond(stream, "405 Method Not Allowed", "text/plain", b"GET only");
    }
    match resolve(&server.web, path).and_then(|p| fs::read(&p).ok().map(|b| (p, b))) {
        Some((path, body)) => respond(stream, "200 OK", content_type(&path), &body),
        None => respond(stream, "404 Not Found", "text/plain", b"not found"),
    }
}

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(8173);
    let viz = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let server = Arc::new(Server {
        web: viz.join("web"),
        root: viz.parent().unwrap().to_path_buf(),
        job: Mutex::new(Job::default()),
    });
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("cannot bind port");
    println!("Serving {} on http://127.0.0.1:{port} (chip builds: /api/chip/*)", server.web.display());
    for stream in listener.incoming().flatten() {
        let server = Arc::clone(&server);
        thread::spawn(move || handle(stream, server));
    }
}
