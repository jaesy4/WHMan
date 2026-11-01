//! Phone ⇄ desktop sync over local Wi-Fi.
//!
//! * The **desktop is the hub**: it runs a small HTTP server (feature
//!   `server`). Phones connect to it (feature `client`).
//! * **Pairing**: the desktop shows a QR code holding its address and a fresh
//!   secret. The phone scans it; the secret itself is never sent over the
//!   network — every request is signed with HMAC-SHA256 instead.
//! * **Incremental**: every local write gets a sequence number. Each side asks
//!   for "changes after N" and remembers N per peer.
//! * **Conflicts**: item details → latest edit wins (hybrid clock, device id
//!   as tie-breaker). Stock records are only ever added or voided, so IN/OUT
//!   from different devices always merge without losing anything.

pub mod discovery;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "client")]
pub mod client;

use hmac::{Hmac, Mac};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::db::{current_seq, next_seq, observe_stamp, Db};
use crate::error::Result;
use crate::types::PeerInfo;

pub const DEFAULT_PORT: u16 = 47810;
pub const DISCOVERY_PORT: u16 = 47811;
pub const PAGE_SIZE: u32 = 500;
/// Requests older/newer than this are rejected (replay protection).
pub const MAX_CLOCK_SKEW_MS: i64 = 10 * 60 * 1000;

pub const H_DEVICE: &str = "x-device-id";
pub const H_TIME: &str = "x-timestamp";
pub const H_SIG: &str = "x-signature";

// ------------------------------------------------------------ wire types

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemRecord {
    pub id: String,
    pub code: String,
    pub name: String,
    pub unit: String,
    pub location: String,
    pub reorder_milli: i64,
    pub remarks: String,
    pub deleted: bool,
    pub updated_at: i64,
    pub updated_by: String,
    #[serde(skip)]
    seq: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovementRecord {
    pub id: String,
    pub item_id: String,
    pub kind: String,
    pub qty_milli: i64,
    pub date: String,
    pub reference: String,
    pub party: String,
    pub remarks: String,
    pub recorded_by: String,
    pub created_at: i64,
    pub voided: bool,
    pub updated_at: i64,
    pub updated_by: String,
    #[serde(skip)]
    seq: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub items: Vec<ItemRecord>,
    pub movements: Vec<MovementRecord>,
}

impl ChangeSet {
    pub fn len(&self) -> usize {
        self.items.len() + self.movements.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResponse {
    pub server_id: String,
    pub server_name: String,
    pub changes: ChangeSet,
    pub next_cursor: i64,
    pub more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    pub changes: ChangeSet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResponse {
    pub applied: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequest {
    pub device_id: String,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairResponse {
    pub server_id: String,
    pub server_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingResponse {
    pub server_id: String,
    pub server_name: String,
    pub app: String,
}

/// Content of the pairing QR code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QrPayload {
    /// Format version.
    pub v: u8,
    pub app: String,
    pub host: String,
    pub port: u16,
    pub server_id: String,
    pub server_name: String,
    pub secret: String,
}

pub const APP_TAG: &str = "warehouse-inventory";

// ------------------------------------------------------------- signing

type HmacSha256 = Hmac<Sha256>;

fn mac(secret: &str, method: &str, path: &str, ts: i64, body: &[u8]) -> HmacSha256 {
    let mut m = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    m.update(method.to_ascii_uppercase().as_bytes());
    m.update(b"\n");
    m.update(path.as_bytes());
    m.update(b"\n");
    m.update(ts.to_string().as_bytes());
    m.update(b"\n");
    m.update(body);
    m
}

/// Signature for a request: HMAC-SHA256(secret, METHOD \n path?query \n timestamp \n body).
pub fn sign(secret: &str, method: &str, path: &str, ts: i64, body: &[u8]) -> String {
    hex::encode(mac(secret, method, path, ts, body).finalize().into_bytes())
}

pub fn verify(secret: &str, method: &str, path: &str, ts: i64, body: &[u8], sig_hex: &str) -> bool {
    let Ok(sig) = hex::decode(sig_hex.trim()) else {
        return false;
    };
    mac(secret, method, path, ts, body).verify_slice(&sig).is_ok()
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 256-bit random secret (hex).
pub fn new_secret() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

// ------------------------------------------------------ change exchange

#[derive(Debug, Clone)]
pub(crate) struct PeerRow {
    pub peer_id: String,
    pub name: String,
    pub secret: String,
    pub address: String,
    pub pull_cursor: i64,
    pub push_cursor: i64,
    pub last_sync_at: Option<String>,
}

fn peer_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PeerRow> {
    Ok(PeerRow {
        peer_id: r.get(0)?,
        name: r.get(1)?,
        secret: r.get(2)?,
        address: r.get(3)?,
        pull_cursor: r.get(4)?,
        push_cursor: r.get(5)?,
        last_sync_at: r.get(6)?,
    })
}

const PEER_COLS: &str = "peer_id, name, secret, address, pull_cursor, push_cursor, last_sync_at";

fn apply_item(c: &Connection, r: &ItemRecord) -> Result<bool> {
    let local: Option<(i64, String)> = c
        .query_row("SELECT updated_at, updated_by FROM items WHERE id = ?1", [&r.id], |x| {
            Ok((x.get(0)?, x.get(1)?))
        })
        .optional()?;
    observe_stamp(c, r.updated_at)?;
    if let Some(l) = local {
        if (r.updated_at, r.updated_by.as_str()) <= (l.0, l.1.as_str()) {
            return Ok(false); // ours is the same or newer
        }
    }
    c.execute(
        "INSERT OR REPLACE INTO items (id, code, name, unit, location, reorder_milli, remarks,
                                       deleted, updated_at, updated_by, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            r.id,
            r.code,
            r.name,
            r.unit,
            r.location,
            r.reorder_milli,
            r.remarks,
            r.deleted as i64,
            r.updated_at,
            r.updated_by,
            next_seq(c)?,
        ],
    )?;
    Ok(true)
}

fn apply_movement(c: &Connection, r: &MovementRecord) -> Result<bool> {
    if (r.kind != "IN" && r.kind != "OUT") || r.qty_milli <= 0 {
        return Ok(false); // malformed: ignore rather than abort the batch
    }
    let local: Option<(i64, String, i64)> = c
        .query_row("SELECT updated_at, updated_by, voided FROM movements WHERE id = ?1", [&r.id], |x| {
            Ok((x.get(0)?, x.get(1)?, x.get(2)?))
        })
        .optional()?;
    observe_stamp(c, r.updated_at)?;
    if let Some(l) = local {
        // A void always wins: once cancelled on any device it stays cancelled.
        let incoming_newer = (r.updated_at, r.updated_by.as_str()) > (l.0, l.1.as_str());
        let wins = (r.voided && l.2 == 0) || (incoming_newer && !(l.2 != 0 && !r.voided));
        if !wins {
            return Ok(false);
        }
    }
    c.execute(
        "INSERT OR REPLACE INTO movements (id, item_id, kind, qty_milli, date, reference, party, remarks,
                                           recorded_by, created_at, voided, updated_at, updated_by, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            r.id,
            r.item_id,
            r.kind,
            r.qty_milli,
            r.date,
            r.reference,
            r.party,
            r.remarks,
            r.recorded_by,
            r.created_at,
            r.voided as i64,
            r.updated_at,
            r.updated_by,
            next_seq(c)?,
        ],
    )?;
    Ok(true)
}

impl Db {
    /// Changes with sequence number greater than `since`, oldest first.
    /// With `only_by`, only rows whose last change was made by that device.
    /// Returns the changes, the cursor to ask from next time, and whether more remain.
    pub fn changes_since(&self, since: i64, limit: u32, only_by: Option<&str>) -> Result<(ChangeSet, i64, bool)> {
        let limit = limit.clamp(1, 5000);
        let by = only_by.unwrap_or("");
        self.read(|c| {
            let mut stmt = c.prepare(
                "SELECT id, code, name, unit, location, reorder_milli, remarks, deleted,
                        updated_at, updated_by, seq
                 FROM items WHERE seq > ?1 AND (?3 = '' OR updated_by = ?3) ORDER BY seq LIMIT ?2",
            )?;
            let mut items = stmt
                .query_map(params![since, limit, by], |r| {
                    Ok(ItemRecord {
                        id: r.get(0)?,
                        code: r.get(1)?,
                        name: r.get(2)?,
                        unit: r.get(3)?,
                        location: r.get(4)?,
                        reorder_milli: r.get(5)?,
                        remarks: r.get(6)?,
                        deleted: r.get::<_, i64>(7)? != 0,
                        updated_at: r.get(8)?,
                        updated_by: r.get(9)?,
                        seq: r.get(10)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut stmt = c.prepare(
                "SELECT id, item_id, kind, qty_milli, date, reference, party, remarks, recorded_by,
                        created_at, voided, updated_at, updated_by, seq
                 FROM movements WHERE seq > ?1 AND (?3 = '' OR updated_by = ?3) ORDER BY seq LIMIT ?2",
            )?;
            let mut movements = stmt
                .query_map(params![since, limit, by], |r| {
                    Ok(MovementRecord {
                        id: r.get(0)?,
                        item_id: r.get(1)?,
                        kind: r.get(2)?,
                        qty_milli: r.get(3)?,
                        date: r.get(4)?,
                        reference: r.get(5)?,
                        party: r.get(6)?,
                        remarks: r.get(7)?,
                        recorded_by: r.get(8)?,
                        created_at: r.get(9)?,
                        voided: r.get::<_, i64>(10)? != 0,
                        updated_at: r.get(11)?,
                        updated_by: r.get(12)?,
                        seq: r.get(13)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            // Keep only the `limit` lowest sequence numbers across both tables,
            // so the cursor never skips a change.
            let mut seqs: Vec<i64> = items.iter().map(|r| r.seq).chain(movements.iter().map(|r| r.seq)).collect();
            seqs.sort_unstable();
            let more = seqs.len() >= limit as usize;
            let cursor = if seqs.is_empty() {
                since
            } else {
                seqs[(limit as usize).min(seqs.len()) - 1]
            };
            items.retain(|r| r.seq <= cursor);
            movements.retain(|r| r.seq <= cursor);
            Ok((ChangeSet { items, movements }, cursor, more))
        })
    }

    /// Applies changes received from another device. Returns how many rows
    /// actually changed here.
    pub fn apply_changes(&self, changes: &ChangeSet) -> Result<u32> {
        self.write(|c| {
            let mut n = 0;
            for r in &changes.items {
                n += apply_item(c, r)? as u32;
            }
            for r in &changes.movements {
                n += apply_movement(c, r)? as u32;
            }
            Ok(n)
        })
    }

    pub fn current_seq(&self) -> Result<i64> {
        self.read(current_seq)
    }

    // ---- peers

    pub(crate) fn peer(&self, peer_id: &str) -> Result<Option<PeerRow>> {
        self.read(|c| {
            Ok(c.query_row(&format!("SELECT {PEER_COLS} FROM peers WHERE peer_id = ?1"), [peer_id], peer_from_row)
                .optional()?)
        })
    }

    pub(crate) fn first_peer(&self) -> Result<Option<PeerRow>> {
        self.read(|c| {
            Ok(c.query_row(
                &format!("SELECT {PEER_COLS} FROM peers ORDER BY last_sync_at DESC LIMIT 1"),
                [],
                peer_from_row,
            )
            .optional()?)
        })
    }

    pub(crate) fn save_peer(&self, peer_id: &str, name: &str, secret: &str, address: &str) -> Result<()> {
        self.write(|c| {
            c.execute(
                "INSERT INTO peers (peer_id, name, secret, address) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(peer_id) DO UPDATE SET name = excluded.name, secret = excluded.secret,
                     address = excluded.address",
                params![peer_id, name, secret, address],
            )?;
            Ok(())
        })
    }

    pub(crate) fn update_peer_progress(
        &self,
        peer_id: &str,
        address: Option<&str>,
        pull_cursor: Option<i64>,
        push_cursor: Option<i64>,
        touched: bool,
    ) -> Result<()> {
        self.write(|c| {
            if let Some(a) = address {
                c.execute("UPDATE peers SET address = ?2 WHERE peer_id = ?1", params![peer_id, a])?;
            }
            if let Some(p) = pull_cursor {
                c.execute("UPDATE peers SET pull_cursor = ?2 WHERE peer_id = ?1", params![peer_id, p])?;
            }
            if let Some(p) = push_cursor {
                c.execute("UPDATE peers SET push_cursor = ?2 WHERE peer_id = ?1", params![peer_id, p])?;
            }
            if touched {
                let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
                c.execute("UPDATE peers SET last_sync_at = ?2 WHERE peer_id = ?1", params![peer_id, now])?;
            }
            Ok(())
        })
    }

    pub fn list_peers(&self) -> Result<Vec<PeerInfo>> {
        self.read(|c| {
            let mut stmt = c.prepare(&format!("SELECT {PEER_COLS} FROM peers ORDER BY name"))?;
            let rows = stmt.query_map([], peer_from_row)?;
            let mut out = Vec::new();
            for r in rows {
                let r = r?;
                out.push(PeerInfo { peer_id: r.peer_id, name: r.name, address: r.address, last_sync_at: r.last_sync_at });
            }
            Ok(out)
        })
    }

    /// Un-pairs a device: it can no longer sync until paired again.
    pub fn remove_peer(&self, peer_id: &str) -> Result<()> {
        self.write(|c| {
            c.execute("DELETE FROM peers WHERE peer_id = ?1", [peer_id])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_round_trip() {
        let s = new_secret();
        let sig = sign(&s, "post", "/sync/push", 123, b"{}");
        assert!(verify(&s, "POST", "/sync/push", 123, b"{}", &sig));
        assert!(!verify(&s, "POST", "/sync/push", 124, b"{}", &sig));
        assert!(!verify("other", "POST", "/sync/push", 123, b"{}", &sig));
        assert!(!verify(&s, "POST", "/sync/push", 123, b"{} ", &sig));
    }
}
