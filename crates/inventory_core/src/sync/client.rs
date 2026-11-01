//! Phone side: pairs with and syncs to the desktop hub.

use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::{
    discovery, now_ms, sign, PairRequest, PairResponse, PingResponse, PullResponse, PushRequest, PushResponse,
    QrPayload, APP_TAG, DISCOVERY_PORT, H_DEVICE, H_SIG, H_TIME, PAGE_SIZE,
};
use crate::types::{PeerInfo, SyncReport};
use crate::{Db, Error, Result};

fn http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(60))
        .build()?)
}

async fn read_json<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Sync(if text.is_empty() { format!("Desktop answered {status}") } else { text }));
    }
    Ok(resp.json::<T>().await?)
}

struct Signed<'a> {
    http: &'a reqwest::Client,
    address: String,
    device_id: String,
    secret: String,
}

impl Signed<'_> {
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let ts = now_ms();
        let sig = sign(&self.secret, "GET", path, ts, b"");
        let resp = self
            .http
            .get(format!("http://{}{path}", self.address))
            .header(H_DEVICE, &self.device_id)
            .header(H_TIME, ts.to_string())
            .header(H_SIG, sig)
            .send()
            .await?;
        read_json(resp).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let bytes = serde_json::to_vec(body)?;
        let ts = now_ms();
        let sig = sign(&self.secret, "POST", path, ts, &bytes);
        let resp = self
            .http
            .post(format!("http://{}{path}", self.address))
            .header(H_DEVICE, &self.device_id)
            .header(H_TIME, ts.to_string())
            .header(H_SIG, sig)
            .header("content-type", "application/json")
            .body(bytes)
            .send()
            .await?;
        read_json(resp).await
    }
}

/// Parses the scanned QR text and pairs this phone with the desktop.
pub async fn pair(db: Arc<Db>, qr_text: &str) -> Result<PeerInfo> {
    let qr: QrPayload = serde_json::from_str(qr_text.trim())
        .map_err(|_| Error::Validation("This QR code is not a pairing code from the desktop app.".into()))?;
    if qr.app != APP_TAG || qr.v != 1 {
        return Err(Error::Validation("This QR code is not a pairing code from the desktop app.".into()));
    }
    let client = http()?;
    let address = format!("{}:{}", qr.host, qr.port);
    let s = Signed { http: &client, address: address.clone(), device_id: db.device_id()?, secret: qr.secret.clone() };
    let req = PairRequest { device_id: db.device_id()?, device_name: db.device_name()? };
    let res: PairResponse = s.post("/pair", &req).await?;
    db.save_peer(&res.server_id, &res.server_name, &qr.secret, &address)?;
    Ok(PeerInfo { peer_id: res.server_id, name: res.server_name, address, last_sync_at: None })
}

async fn reachable(client: &reqwest::Client, address: &str, server_id: &str) -> bool {
    let Ok(resp) = client.get(format!("http://{address}/ping")).timeout(Duration::from_secs(3)).send().await else {
        return false;
    };
    match read_json::<PingResponse>(resp).await {
        Ok(p) => p.server_id == server_id,
        Err(_) => false,
    }
}

/// Full two-way sync with the paired desktop: send local changes, then
/// receive the desktop's changes.
pub async fn sync_with_hub(db: Arc<Db>) -> Result<SyncReport> {
    let peer = db
        .first_peer()?
        .ok_or_else(|| Error::Validation("Not paired yet. Scan the QR code shown on the PC first.".into()))?;
    let client = http()?;

    // 1. Find the desktop (saved address, else LAN broadcast).
    let mut address = peer.address.clone();
    if !reachable(&client, &address, &peer.peer_id).await {
        let id = peer.peer_id.clone();
        let found = tokio::task::spawn_blocking(move || discovery::find_hub(&id, DISCOVERY_PORT, Duration::from_secs(4)))
            .await
            .map_err(|e| Error::Sync(e.to_string()))?;
        match found {
            Some((_, addr)) => address = addr,
            None => {
                return Err(Error::Sync(format!(
                    "Cannot find \"{}\" on this Wi-Fi. Make sure the PC app is open and on the same network.",
                    peer.name
                )))
            }
        }
    }
    let s = Signed { http: &client, address: address.clone(), device_id: db.device_id()?, secret: peer.secret.clone() };
    let mut report = SyncReport { address: address.clone(), ..Default::default() };

    // 2. Push changes made on this phone. Rows received from the desktop are
    //    not sent back (it already has them); on the very first sync with a
    //    hub everything is sent.
    let me = db.device_id()?;
    let only_mine = if peer.push_cursor == 0 { None } else { Some(me.as_str()) };
    let mut push_cursor = peer.push_cursor;
    loop {
        let (changes, cursor, more) = db.changes_since(push_cursor, PAGE_SIZE, only_mine)?;
        if changes.is_empty() {
            break;
        }
        let n = changes.len() as u32;
        let res: PushResponse = s.post("/sync/push", &PushRequest { changes }).await?;
        report.pushed += n;
        let _ = res.applied;
        push_cursor = cursor;
        db.update_peer_progress(&peer.peer_id, Some(&address), None, Some(push_cursor), false)?;
        if !more {
            break;
        }
    }

    // 3. Pull the desktop's changes.
    let mut pull_cursor = peer.pull_cursor;
    loop {
        let res: PullResponse = s.get(&format!("/sync/pull?since={pull_cursor}&limit={PAGE_SIZE}")).await?;
        report.pulled += res.changes.len() as u32;
        let applied = db.apply_changes(&res.changes)?;
        report.applied += applied;
        pull_cursor = res.next_cursor;
        db.update_peer_progress(&peer.peer_id, None, Some(pull_cursor), None, false)?;
        if !res.more {
            break;
        }
    }

    // After the first successful sync, switch to "only my changes" mode.
    let first_done = if push_cursor == 0 { Some(1) } else { None };
    db.update_peer_progress(&peer.peer_id, Some(&address), None, first_done, true)?;
    report.finished_at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    Ok(report)
}
