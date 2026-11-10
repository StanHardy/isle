use std::collections::HashMap;
use std::io::Read;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use tiny_http::{Method, Request, Response, Server};

use crate::events::{self, ev};
use crate::notify::NotifyItem;
use crate::state::AppState;

pub const PORT: u16 = 7350;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// Phone companion bridge: a token-gated LAN HTTP endpoint. Phones relay
/// notifications via POST /notify; replies are never promised (phone OS
/// restrictions), see README "Phone companion".
///
/// The socket binds lazily — only once the user enables the bridge — so the
/// Windows Firewall prompt never appears for a disabled feature.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            let enabled = {
                let state: tauri::State<AppState> = app.state();
                let s = state.settings.read().expect("settings lock");
                s.bridge_enabled
            };
            if enabled {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
        let server = match Server::http(("0.0.0.0", PORT)) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("[bridge] bind 0.0.0.0:{PORT} failed: {err}");
                return;
            }
        };
        eprintln!("[bridge] listening on 0.0.0.0:{PORT}");
        for request in server.incoming_requests() {
            handle(&app, request);
        }
    });
}

fn handle(app: &AppHandle, mut request: Request) {
    let method = request.method().clone();
    let url = request.url().to_string();
    let mut body = Vec::new();
    let _ = request
        .as_reader()
        .take(65536)
        .read_to_end(&mut body);
    let body = String::from_utf8_lossy(&body).to_string();

    if !rate_ok(request.remote_addr()) {
        let _ = request.respond(resp(429, r#"{"error":"slow down"}"#));
        return;
    }

    let response = match (&method, url.as_str()) {
        (Method::Get, "/ping") => resp(200, r#"{"status":"ok","app":"isle"}"#),
        (Method::Post, "/notify") => handle_notify(app, &body),
        (Method::Post, "/action") => handle_action(app, &body),
        _ => resp(404, r#"{"error":"not found"}"#),
    };
    let _ = request.respond(response);
}

/// Per-source-IP rate limiting: max 20 requests per 10 s window. tiny_http
/// handles requests sequentially on one thread, so the counter is cheap and
/// safe without a lock.
const RATE_LIMIT: u32 = 20;
const RATE_WINDOW: Duration = Duration::from_secs(10);
static RATE: OnceLock<Mutex<HashMap<IpAddr, (u32, Instant)>>> = OnceLock::new();

fn rate_map() -> &'static Mutex<HashMap<IpAddr, (u32, Instant)>> {
    RATE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn rate_ok(addr: Option<&std::net::SocketAddr>) -> bool {
    let Some(addr) = addr else { return true };
    let ip = addr.ip();
    let mut map = rate_map().lock().expect("rate lock");
    let now = Instant::now();
    let entry = map.entry(ip).or_insert((0, now));
    if now.duration_since(entry.1) > RATE_WINDOW {
        *entry = (0, now);
    }
    entry.0 += 1;
    entry.1 = now;
    let n = entry.0;
    if map.len() > 256 {
        map.clear();
    }
    n <= RATE_LIMIT
}

fn resp(status: u16, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body).with_status_code(status)
}

/// Constant-time token comparison (no early exit on content mismatch).
fn token_ok(presented: &str, token: &str) -> bool {
    let a = presented.as_bytes();
    let b = token.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..b.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn handle_notify(app: &AppHandle, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return resp(400, r#"{"error":"invalid json"}"#),
    };
    let state: tauri::State<AppState> = app.state();
    let (enabled, token) = {
        let s = state.settings.read().expect("settings lock");
        (s.bridge_enabled, s.bridge_token.clone())
    };
    if !enabled {
        return resp(403, r#"{"error":"bridge disabled"}"#);
    }
    let presented = value.get("token").and_then(|t| t.as_str()).unwrap_or("");
    if !token_ok(presented, &token) {
        return resp(401, r#"{"error":"bad token"}"#);
    }
    let item = NotifyItem {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        source: "phone".into(),
        app: str_field(&value, "app"),
        title: str_field(&value, "title"),
        body: str_field(&value, "body"),
        ts_ms: crate::clipboard::now_ms(),
    };
    if item.title.is_empty() && item.body.is_empty() {
        return resp(422, r#"{"error":"title or body required"}"#);
    }
    {
        let mut list = state.notifications.lock().expect("notifications lock");
        list.insert(0, item.clone());
        list.truncate(50);
    }
    events::emit(app, ev::NOTIFY_NEW, &item);
    events::emit(
        app,
        ev::ATTENTION,
        &serde_json::json!({ "kind": "phone-notify", "title": item.title }),
    );
    resp(204, "")
}

/// Phone-side actions: local dismiss works; reply is rejected honestly —
/// replying requires a device-side agent the platform does not grant a
/// remote HTTP caller.
fn handle_action(app: &AppHandle, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return resp(400, r#"{"error":"invalid json"}"#),
    };
    let state: tauri::State<AppState> = app.state();
    let (enabled, token) = {
        let s = state.settings.read().expect("settings lock");
        (s.bridge_enabled, s.bridge_token.clone())
    };
    if !enabled {
        return resp(403, r#"{"error":"bridge disabled"}"#);
    }
    let presented = value.get("token").and_then(|t| t.as_str()).unwrap_or("");
    if !token_ok(presented, &token) {
        return resp(401, r#"{"error":"bad token"}"#);
    }
    let action = value.get("action").and_then(|a| a.as_str()).unwrap_or("");
    match action {
        "dismiss" => {
            let id = value.get("id").and_then(|i| i.as_u64());
            let mut list = state.notifications.lock().expect("notifications lock");
            match id {
                Some(id) => list.retain(|n| n.id != id),
                None => list.clear(),
            }
            resp(204, "")
        }
        "reply" => resp(
            501,
            r#"{"error":"reply requires a device-side agent; not supported by this bridge"}"#,
        ),
        _ => resp(400, r#"{"error":"unknown action"}"#),
    }
}

fn str_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

pub fn status(app: &AppHandle) -> serde_json::Value {
    let state: tauri::State<AppState> = app.state();
    let s = state.settings.read().expect("settings lock");
    serde_json::json!({
        "enabled": s.bridge_enabled,
        "token": s.bridge_token,
        "port": PORT,
    })
}
