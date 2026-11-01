//! Desktop hub: embedded HTTP sync server (axum).
//!
//! Endpoints (all JSON):
//! * `GET  /ping`                      — reachability check (no auth)
//! * `POST /pair`                      — signed with the QR secret
//! * `GET  /sync/pull?since=N&limit=M` — signed
//! * `POST /sync/push`                 — signed

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::{
    discovery, new_secret, now_ms, verify, PairRequest, PairResponse, PingResponse, PullResponse, PushRequest,
    PushResponse, QrPayload, APP_TAG, DISCOVERY_PORT, H_DEVICE, H_SIG, H_TIME, MAX_CLOCK_SKEW_MS, PAGE_SIZE,
};
use crate::types::PairingOffer;
use crate::{Db, Error, Result};

const PAIRING_TTL: Duration = Duration::from_secs(5 * 60);

struct Pending {
    secret: String,
    expires: Instant,
}

/// Shared state of the sync server. Keep one per app.
pub struct Hub {
    db: Arc<Db>,
    port: u16,
    pending: Mutex<Option<Pending>>,
}

impl Hub {
    pub fn new(db: Arc<Db>, port: u16) -> Arc<Self> {
        Arc::new(Self { db, port, pending: Mutex::new(None) })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Creates a one-time pairing secret (valid 5 minutes) and the QR code
    /// the phone scans.
    pub fn start_pairing(&self) -> Result<PairingOffer> {
        let secret = new_secret();
        let host = discovery::local_ip().map(|ip| ip.to_string()).unwrap_or_else(|| "127.0.0.1".into());
        let payload = QrPayload {
            v: 1,
            app: APP_TAG.into(),
            host: host.clone(),
            port: self.port,
            server_id: self.db.device_id()?,
            server_name: self.db.device_name()?,
            secret: secret.clone(),
        };
        let payload = serde_json::to_string(&payload)?;
        let code = qrcode::QrCode::new(payload.as_bytes()).map_err(|e| Error::Sync(e.to_string()))?;
        let qr_svg = code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(260, 260)
            .quiet_zone(true)
            .build();
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Pending { secret, expires: Instant::now() + PAIRING_TTL });
        Ok(PairingOffer { payload, qr_svg, host, port: self.port, expires_in_secs: PAIRING_TTL.as_secs() })
    }

    pub fn cancel_pairing(&self) {
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn pending_secret(&self) -> Option<String> {
        let guard = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().filter(|p| p.expires > Instant::now()).map(|p| p.secret.clone())
    }
}

/// Runs the server until the app exits. Also starts the LAN discovery responder.
pub async fn serve(hub: Arc<Hub>) -> std::io::Result<()> {
    let server_id = hub.db.device_id().unwrap_or_default();
    if let Err(e) = discovery::spawn_responder(server_id, hub.port, DISCOVERY_PORT) {
        eprintln!("[sync] discovery responder not started: {e}");
    }
    let app = Router::new()
        .route("/ping", get(ping))
        .route("/pair", post(pair))
        .route("/sync/pull", get(pull))
        .route("/sync/push", post(push))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(hub.clone());
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, hub.port)).await?;
    eprintln!("[sync] listening on 0.0.0.0:{}", hub.port);
    axum::serve(listener, app).await
}

type ApiError = (StatusCode, String);
type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

fn api_err(e: Error) -> ApiError {
    match e {
        Error::Validation(m) => (StatusCode::BAD_REQUEST, m),
        Error::NotFound(m) => (StatusCode::NOT_FOUND, m),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

fn unauthorized(msg: &str) -> ApiError {
    (StatusCode::UNAUTHORIZED, msg.to_string())
}

struct SignedHeaders {
    device: String,
    ts: i64,
    sig: String,
}

fn signed_headers(headers: &HeaderMap) -> std::result::Result<SignedHeaders, ApiError> {
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let device = h(H_DEVICE).ok_or_else(|| unauthorized("Missing device id."))?;
    let ts = h(H_TIME).and_then(|s| s.parse::<i64>().ok()).ok_or_else(|| unauthorized("Missing timestamp."))?;
    let sig = h(H_SIG).ok_or_else(|| unauthorized("Missing signature."))?;
    if (now_ms() - ts).abs() > MAX_CLOCK_SKEW_MS {
        return Err(unauthorized(
            "The phone's clock and the PC's clock differ by more than 10 minutes. Fix the date/time and try again.",
        ));
    }
    Ok(SignedHeaders { device, ts, sig })
}

fn path_of(uri: &Uri) -> &str {
    uri.path_and_query().map(|p| p.as_str()).unwrap_or_else(|| uri.path())
}

/// Checks the request signature against the paired device's secret.
fn authenticate(hub: &Hub, method: &str, uri: &Uri, headers: &HeaderMap, body: &[u8]) -> std::result::Result<String, ApiError> {
    let sh = signed_headers(headers)?;
    let peer = hub
        .db
        .peer(&sh.device)
        .map_err(api_err)?
        .ok_or_else(|| unauthorized("This device is not paired. Pair again from the Sync screen."))?;
    if !verify(&peer.secret, method, path_of(uri), sh.ts, body, &sh.sig) {
        return Err(unauthorized("Invalid signature. Pair the device again."));
    }
    Ok(sh.device)
}

async fn ping(State(hub): State<Arc<Hub>>) -> ApiResult<PingResponse> {
    Ok(Json(PingResponse {
        server_id: hub.db.device_id().map_err(api_err)?,
        server_name: hub.db.device_name().map_err(api_err)?,
        app: APP_TAG.into(),
    }))
}

async fn pair(State(hub): State<Arc<Hub>>, headers: HeaderMap, uri: Uri, body: Bytes) -> ApiResult<PairResponse> {
    let secret = hub
        .pending_secret()
        .ok_or_else(|| (StatusCode::GONE, "Pairing code expired. Press \"Pair a phone\" on the PC again.".to_string()))?;
    let sh = signed_headers(&headers)?;
    if !verify(&secret, "POST", path_of(&uri), sh.ts, &body, &sh.sig) {
        return Err(unauthorized("QR code does not match. Scan the code currently shown on the PC."));
    }
    let req: PairRequest =
        serde_json::from_slice(&body).map_err(|e| (StatusCode::BAD_REQUEST, format!("Bad request: {e}")))?;
    if req.device_id != sh.device || req.device_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Device id mismatch.".into()));
    }
    hub.db.save_peer(&req.device_id, req.device_name.trim(), &secret, "").map_err(api_err)?;
    hub.cancel_pairing();
    Ok(Json(PairResponse {
        server_id: hub.db.device_id().map_err(api_err)?,
        server_name: hub.db.device_name().map_err(api_err)?,
    }))
}

#[derive(Deserialize)]
struct PullQuery {
    since: Option<i64>,
    limit: Option<u32>,
}

async fn pull(
    State(hub): State<Arc<Hub>>,
    Query(q): Query<PullQuery>,
    headers: HeaderMap,
    uri: Uri,
) -> ApiResult<PullResponse> {
    let device = authenticate(&hub, "GET", &uri, &headers, b"")?;
    let db = hub.db.clone();
    let since = q.since.unwrap_or(0).max(0);
    let limit = q.limit.unwrap_or(PAGE_SIZE);
    let res = tokio::task::spawn_blocking(move || -> Result<PullResponse> {
        let (changes, next_cursor, more) = db.changes_since(since, limit, None)?;
        if !more {
            db.update_peer_progress(&device, None, None, None, true)?;
        }
        Ok(PullResponse { server_id: db.device_id()?, server_name: db.device_name()?, changes, next_cursor, more })
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(api_err)?;
    Ok(Json(res))
}

async fn push(State(hub): State<Arc<Hub>>, headers: HeaderMap, uri: Uri, body: Bytes) -> ApiResult<PushResponse> {
    let _device = authenticate(&hub, "POST", &uri, &headers, &body)?;
    let req: PushRequest =
        serde_json::from_slice(&body).map_err(|e| (StatusCode::BAD_REQUEST, format!("Bad request: {e}")))?;
    let db = hub.db.clone();
    let applied = tokio::task::spawn_blocking(move || db.apply_changes(&req.changes))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(api_err)?;
    Ok(Json(PushResponse { applied }))
}
