//! SQLite storage, schema migrations, device identity and the sync clock.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::error::Result;

/// Schema versions, applied in order. Never edit a released entry — append a
/// new one instead. The current version is kept in `PRAGMA user_version`.
const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE items (
        id            TEXT PRIMARY KEY,          -- UUID: safe to create offline on any device
        code          TEXT NOT NULL,
        name          TEXT NOT NULL,
        unit          TEXT NOT NULL DEFAULT '',
        location      TEXT NOT NULL DEFAULT '',
        reorder_milli INTEGER NOT NULL DEFAULT 0, -- reorder level x 1000
        remarks       TEXT NOT NULL DEFAULT '',
        deleted       INTEGER NOT NULL DEFAULT 0, -- soft delete so it can sync
        updated_at    INTEGER NOT NULL,           -- hybrid clock (ms)
        updated_by    TEXT NOT NULL,              -- device id of last change
        seq           INTEGER NOT NULL            -- local change counter (sync cursor)
    );
    CREATE INDEX idx_items_code ON items(code);
    CREATE INDEX idx_items_seq  ON items(seq);

    -- Stock is never stored as a single number: every IN and OUT is a row,
    -- and the balance is the sum. Records from different devices merge cleanly.
    CREATE TABLE movements (
        id          TEXT PRIMARY KEY,
        item_id     TEXT NOT NULL,
        kind        TEXT NOT NULL CHECK (kind IN ('IN', 'OUT')),
        qty_milli   INTEGER NOT NULL CHECK (qty_milli > 0),
        date        TEXT NOT NULL,                -- YYYY-MM-DD
        reference   TEXT NOT NULL DEFAULT '',
        party       TEXT NOT NULL DEFAULT '',     -- received from / issued to
        remarks     TEXT NOT NULL DEFAULT '',
        recorded_by TEXT NOT NULL DEFAULT '',
        created_at  INTEGER NOT NULL,
        voided      INTEGER NOT NULL DEFAULT 0,
        updated_at  INTEGER NOT NULL,
        updated_by  TEXT NOT NULL,
        seq         INTEGER NOT NULL
    );
    CREATE INDEX idx_mov_item_date ON movements(item_id, date);
    CREATE INDEX idx_mov_date      ON movements(date);
    CREATE INDEX idx_mov_seq       ON movements(seq);

    -- Paired devices. On the desktop: phones allowed to sync.
    -- On the phone: the desktop hub.
    CREATE TABLE peers (
        peer_id      TEXT PRIMARY KEY,
        name         TEXT NOT NULL,
        secret       TEXT NOT NULL,
        address      TEXT NOT NULL DEFAULT '',
        pull_cursor  INTEGER NOT NULL DEFAULT 0,
        push_cursor  INTEGER NOT NULL DEFAULT 0,
        last_sync_at TEXT
    );
    "#,
    // v2: department requests + web portal logins. These tables live on the
    // PC hub only (they are not part of phone sync). Issuing a request creates
    // normal Stock Out records, and those do sync.
    r#"
    CREATE TABLE departments (
        id   TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE
    );
    INSERT INTO departments (id, name) VALUES ('engineering', 'Engineering'), ('solar', 'Solar Team');

    CREATE TABLE web_users (
        id            TEXT PRIMARY KEY,
        username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
        full_name     TEXT NOT NULL,
        department_id TEXT NOT NULL REFERENCES departments(id),
        role          TEXT NOT NULL CHECK (role IN ('requester', 'head')),
        password      TEXT NOT NULL,              -- pbkdf2$iterations$salt$hash
        active        INTEGER NOT NULL DEFAULT 1,
        created_at    INTEGER NOT NULL
    );

    CREATE TABLE web_sessions (
        token_hash TEXT PRIMARY KEY,              -- SHA-256 of the token; the token itself is never stored
        user_id    TEXT NOT NULL REFERENCES web_users(id) ON DELETE CASCADE,
        expires_at INTEGER NOT NULL
    );

    CREATE TABLE requests (
        id            TEXT PRIMARY KEY,
        number        TEXT NOT NULL UNIQUE,       -- RQ-2026-0001
        department_id TEXT NOT NULL REFERENCES departments(id),
        user_id       TEXT NOT NULL DEFAULT '',
        requested_by  TEXT NOT NULL,
        purpose       TEXT NOT NULL DEFAULT '',
        needed_by     TEXT NOT NULL DEFAULT '',
        status        TEXT NOT NULL CHECK (status IN
                      ('SUBMITTED', 'APPROVED', 'REJECTED', 'CANCELLED', 'ISSUED', 'RECEIVED')),
        created_at    INTEGER NOT NULL,
        approved_by   TEXT NOT NULL DEFAULT '',
        approved_at   INTEGER,
        decision_note TEXT NOT NULL DEFAULT '',
        issued_by     TEXT NOT NULL DEFAULT '',
        issued_at     INTEGER,
        received_by   TEXT NOT NULL DEFAULT '',
        received_at   INTEGER,
        updated_at    INTEGER NOT NULL
    );
    CREATE INDEX idx_req_status ON requests(status, created_at);
    CREATE INDEX idx_req_dept   ON requests(department_id, created_at);

    CREATE TABLE request_lines (
        id           TEXT PRIMARY KEY,
        request_id   TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
        line_no      INTEGER NOT NULL,
        item_id      TEXT NOT NULL,
        qty_milli    INTEGER NOT NULL CHECK (qty_milli > 0),
        issued_milli INTEGER NOT NULL DEFAULT 0,
        movement_id  TEXT NOT NULL DEFAULT '',    -- the Stock Out created when issued
        remarks      TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX idx_reqline_req ON request_lines(request_id, line_no);
    "#,
];

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// Opens (or creates) the database file and brings the schema up to date.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        migrate(&mut conn)?;
        ensure_identity(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite consistent (the
        // transaction rolls back), so recovering from poisoning is safe.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.lock();
        f(&conn)
    }

    /// Runs `f` in a transaction: everything is saved, or nothing is.
    pub(crate) fn write<T>(&self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    pub fn device_id(&self) -> Result<String> {
        self.read(|c| Ok(get_meta(c, "device_id")?.unwrap_or_default()))
    }

    pub fn device_name(&self) -> Result<String> {
        self.read(|c| Ok(get_meta(c, "device_name")?.unwrap_or_default()))
    }

    pub fn set_device_name(&self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return crate::error::invalid("Device name cannot be empty.");
        }
        self.write(|c| set_meta(c, "device_name", name))
    }

    /// Simple per-device settings (warehouse name, default "recorded by", ...).
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        self.read(|c| get_meta(c, &format!("setting.{key}")))
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.write(|c| set_meta(c, &format!("setting.{key}"), value))
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", i + 1))?;
        tx.commit()?;
    }
    Ok(())
}

fn ensure_identity(conn: &Connection) -> Result<()> {
    if get_meta(conn, "device_id")?.is_none() {
        set_meta(conn, "device_id", &uuid::Uuid::new_v4().simple().to_string())?;
    }
    if get_meta(conn, "device_name")?.is_none() {
        let name = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "Device".to_string());
        set_meta(conn, "device_name", &name)?;
    }
    Ok(())
}

pub(crate) fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub(crate) fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn get_meta_i64(conn: &Connection, key: &str) -> Result<i64> {
    Ok(get_meta(conn, key)?.and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// Next local change number. Every write (local or applied from sync) gets
/// one; peers ask for "everything after N".
pub(crate) fn next_seq(conn: &Connection) -> Result<i64> {
    let n = get_meta_i64(conn, "seq")? + 1;
    set_meta(conn, "seq", &n.to_string())?;
    Ok(n)
}

pub(crate) fn current_seq(conn: &Connection) -> Result<i64> {
    get_meta_i64(conn, "seq")
}

/// Hybrid logical clock: wall-clock milliseconds, but never goes backwards
/// and always moves past any timestamp seen from another device. Used for
/// "latest edit wins" when the same item is edited on two devices.
pub(crate) fn next_stamp(conn: &Connection) -> Result<i64> {
    let last = get_meta_i64(conn, "hlc")?;
    let now = chrono::Utc::now().timestamp_millis();
    let stamp = now.max(last + 1);
    set_meta(conn, "hlc", &stamp.to_string())?;
    Ok(stamp)
}

pub(crate) fn observe_stamp(conn: &Connection, remote: i64) -> Result<()> {
    let last = get_meta_i64(conn, "hlc")?;
    if remote > last {
        set_meta(conn, "hlc", &remote.to_string())?;
    }
    Ok(())
}

pub(crate) fn local_device(conn: &Connection) -> Result<String> {
    Ok(get_meta(conn, "device_id")?.unwrap_or_default())
}

pub(crate) fn new_id() -> String {
    // v7 = time-ordered UUID: unique across devices and sorts by creation.
    uuid::Uuid::now_v7().simple().to_string()
}
