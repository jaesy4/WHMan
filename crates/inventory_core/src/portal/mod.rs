//! Department web portal: a small website served by the PC app so
//! Engineering and Solar Team can request items from any phone or computer.
//!
//! It listens on its own port ([`PORTAL_PORT`]), separate from phone sync, so
//! only the portal is put online (with Tailscale Funnel) — sync stays on the
//! local Wi-Fi.
//!
//! Endpoints (JSON, `Authorization: Bearer <token>` after login):
//! * `GET  /`                              — the web app
//! * `POST /api/login`                     — `{username, password}` → `{token, user}`
//! * `POST /api/logout`
//! * `GET  /api/me`
//! * `POST /api/password`                  — `{old, new}`
//! * `GET  /api/items?search=`             — items with current balance
//! * `GET  /api/requests`                  — own department's requests
//! * `POST /api/requests`                  — new request
//! * `GET  /api/requests/{id}`
//! * `POST /api/requests/{id}/approve`     — head only, `{note}`
//! * `POST /api/requests/{id}/reject`      — head only, `{note}`
//! * `POST /api/requests/{id}/cancel`      — `{note}`
//! * `POST /api/requests/{id}/received`

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::types::{ItemRequest, RequestFilter, RequestInput, WebUser};
use crate::{Actor, Db, Error, Result};

pub const PORTAL_PORT: u16 = 47812;

const INDEX_HTML: &str = include_str!("index.html");
const ICON_SVG: &str = include_str!("icon.svg");

const MAX_FAILS: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(10 * 60);

pub struct Portal {
    db: Arc<Db>,
    port: u16,
    running: AtomicBool,
    /// Failed logins per username: (count, first failure).
    fails: Mutex<HashMap<String, (u32, Instant)>>,
}

impl Portal {
    pub fn new(db: Arc<Db>, port: u16) -> Arc<Self> {
        Arc::new(Self { db, port, running: AtomicBool::new(false), fails: Mutex::new(HashMap::new()) })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    fn locked_out(&self, username: &str) -> bool {
        let mut f = self.fails.lock().unwrap_or_else(|e| e.into_inner());
        match f.get(username) {
            Some((n, since)) if since.elapsed() < LOCKOUT => *n >= MAX_FAILS,
            Some(_) => {
                f.remove(username);
                false
            }
            None => false,
        }
    }

    fn record_fail(&self, username: &str) {
        let mut f = self.fails.lock().unwrap_or_else(|e| e.into_inner());
        let e = f.entry(username.to_string()).or_insert((0, Instant::now()));
        if e.1.elapsed() >= LOCKOUT {
            *e = (0, Instant::now());
        }
        e.0 += 1;
    }

    fn clear_fails(&self, username: &str) {
        self.fails.lock().unwrap_or_else(|e| e.into_inner()).remove(username);
    }
}

/// Runs the portal until the app exits.
pub async fn serve(portal: Arc<Portal>) -> std::io::Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/icon.svg", get(icon))
        .route("/manifest.webmanifest", get(manifest))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/password", post(change_password))
        .route("/api/items", get(items))
        .route("/api/requests", get(list_requests).post(create_request))
        .route("/api/requests/{id}", get(get_request))
        .route("/api/requests/{id}/approve", post(approve))
        .route("/api/requests/{id}/reject", post(reject))
        .route("/api/requests/{id}/cancel", post(cancel))
        .route("/api/requests/{id}/received", post(received))
        .layer(axum::middleware::map_response(security_headers))
        .with_state(portal.clone());
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, portal.port)).await?;
    eprintln!("[portal] listening on 0.0.0.0:{}", portal.port);
    portal.running.store(true, Ordering::Relaxed);
    let res = axum::serve(listener, app).await;
    portal.running.store(false, Ordering::Relaxed);
    res
}

async fn security_headers(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    if !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}

// ------------------------------------------------------------ helpers

type ApiError = (StatusCode, Json<serde_json::Value>);
type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

fn fail(code: StatusCode, msg: impl Into<String>) -> ApiError {
    (code, Json(json!({ "error": msg.into() })))
}

fn api_err(e: Error) -> ApiError {
    match e {
        Error::Validation(m) => fail(StatusCode::BAD_REQUEST, m),
        Error::NotFound(m) => fail(StatusCode::NOT_FOUND, format!("Not found: {m}")),
        other => {
            eprintln!("[portal] {other}");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on the warehouse PC. Try again.")
        }
    }
}

/// Runs database work off the async threads.
async fn blocking<T, F>(f: F) -> std::result::Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(api_err)
}

fn bearer(headers: &HeaderMap) -> String {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string())
        .unwrap_or_default()
}

async fn user(p: &Arc<Portal>, headers: &HeaderMap) -> std::result::Result<WebUser, ApiError> {
    let token = bearer(headers);
    let db = p.db.clone();
    blocking(move || db.web_session_user(&token))
        .await?
        .ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "Please log in again."))
}

// ------------------------------------------------------------ static

async fn index() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], INDEX_HTML)
}

async fn icon() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/svg+xml"), (header::CACHE_CONTROL, "max-age=86400")], ICON_SVG)
}

async fn manifest(State(p): State<Arc<Portal>>) -> impl IntoResponse {
    let org = p.db.setting("warehouse_name").ok().flatten().unwrap_or_default();
    let name = if org.is_empty() { "Warehouse Requests".to_string() } else { format!("{org} Requests") };
    let body = json!({
        "name": name,
        "short_name": "Requests",
        "start_url": "/",
        "display": "standalone",
        "background_color": "#f3f4f6",
        "theme_color": "#26456b",
        "icons": [{ "src": "/icon.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "any" }]
    });
    ([(header::CONTENT_TYPE, "application/manifest+json")], body.to_string())
}

// ------------------------------------------------------------ auth

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    token: String,
    user: WebUser,
    warehouse_name: String,
}

async fn login(State(p): State<Arc<Portal>>, Json(b): Json<LoginBody>) -> ApiResult<LoginResponse> {
    let key = b.username.trim().to_lowercase();
    if p.locked_out(&key) {
        return Err(fail(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many wrong passwords. Wait 10 minutes, or ask the warehouse to reset your password.",
        ));
    }
    let db = p.db.clone();
    let (u, pw) = (b.username.clone(), b.password.clone());
    match blocking(move || db.web_login(&u, &pw)).await {
        Ok((token, user)) => {
            p.clear_fails(&key);
            let warehouse_name = p.db.setting("warehouse_name").ok().flatten().unwrap_or_default();
            Ok(Json(LoginResponse { token, user, warehouse_name }))
        }
        Err(e) => {
            if e.0 == StatusCode::BAD_REQUEST {
                p.record_fail(&key);
            }
            Err(e)
        }
    }
}

async fn logout(State(p): State<Arc<Portal>>, headers: HeaderMap) -> ApiResult<serde_json::Value> {
    let token = bearer(&headers);
    let db = p.db.clone();
    blocking(move || db.web_logout(&token)).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize)]
struct MeResponse {
    user: WebUser,
    warehouse_name: String,
}

async fn me(State(p): State<Arc<Portal>>, headers: HeaderMap) -> ApiResult<MeResponse> {
    let user = user(&p, &headers).await?;
    let warehouse_name = p.db.setting("warehouse_name").ok().flatten().unwrap_or_default();
    Ok(Json(MeResponse { user, warehouse_name }))
}

#[derive(Deserialize)]
struct PasswordBody {
    old: String,
    new: String,
}

async fn change_password(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Json(b): Json<PasswordBody>,
) -> ApiResult<serde_json::Value> {
    let u = user(&p, &headers).await?;
    let db = p.db.clone();
    blocking(move || db.web_change_password(&u.id, &b.old, &b.new)).await?;
    Ok(Json(json!({ "ok": true })))
}

// ------------------------------------------------------------ items

#[derive(Deserialize)]
struct ItemQuery {
    search: Option<String>,
}

#[derive(Serialize)]
struct WebItem {
    id: String,
    code: String,
    name: String,
    unit: String,
    location: String,
    balance: f64,
}

async fn items(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Query(q): Query<ItemQuery>,
) -> ApiResult<Vec<WebItem>> {
    user(&p, &headers).await?;
    let db = p.db.clone();
    let search = q.search.unwrap_or_default();
    let rows = blocking(move || db.stock_list(&search, false)).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| WebItem {
                id: r.item.id,
                code: r.item.code,
                name: r.item.name,
                unit: r.item.unit,
                location: r.item.location,
                balance: r.balance,
            })
            .collect(),
    ))
}

// ------------------------------------------------------------ requests

async fn list_requests(State(p): State<Arc<Portal>>, headers: HeaderMap) -> ApiResult<Vec<ItemRequest>> {
    let u = user(&p, &headers).await?;
    let db = p.db.clone();
    let f = RequestFilter { department_id: Some(u.department_id), limit: Some(100), ..Default::default() };
    Ok(Json(blocking(move || db.list_requests(&f)).await?))
}

async fn create_request(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Json(input): Json<RequestInput>,
) -> ApiResult<ItemRequest> {
    let u = user(&p, &headers).await?;
    let db = p.db.clone();
    Ok(Json(blocking(move || db.create_request(&u, &input)).await?))
}

async fn get_request(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<ItemRequest> {
    let u = user(&p, &headers).await?;
    let db = p.db.clone();
    let req = blocking(move || db.get_request(&id)).await?;
    if req.department_id != u.department_id {
        return Err(fail(StatusCode::NOT_FOUND, "Not found: request"));
    }
    Ok(Json(req))
}

#[derive(Deserialize, Default)]
struct NoteBody {
    #[serde(default)]
    note: String,
}

async fn approve(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(b): Json<NoteBody>,
) -> ApiResult<ItemRequest> {
    let u = user(&p, &headers).await?;
    let db = p.db.clone();
    Ok(Json(blocking(move || db.approve_request(&id, Actor::Web(&u), &b.note)).await?))
}

async fn reject(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(b): Json<NoteBody>,
) -> ApiResult<ItemRequest> {
    let u = user(&p, &headers).await?;
    let db = p.db.clone();
    Ok(Json(blocking(move || db.reject_request(&id, Actor::Web(&u), &b.note)).await?))
}

async fn cancel(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(b): Json<NoteBody>,
) -> ApiResult<ItemRequest> {
    let u = user(&p, &headers).await?;
    let db = p.db.clone();
    Ok(Json(blocking(move || db.cancel_request(&id, &u, &b.note)).await?))
}

async fn received(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<ItemRequest> {
    let u = user(&p, &headers).await?;
    let db = p.db.clone();
    Ok(Json(blocking(move || db.mark_request_received(&id, &u)).await?))
}
