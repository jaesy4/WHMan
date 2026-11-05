//! Departments and logins for the department web portal.
//!
//! Passwords are stored as PBKDF2-HMAC-SHA256 (`pbkdf2$iterations$salt$hash`).
//! Session tokens are random; only their SHA-256 is stored, so a copy of the
//! database cannot be used to log in.

use hmac::{Hmac, Mac};
use rusqlite::{params, Connection, OptionalExtension, Row};
use sha2::{Digest, Sha256};

use crate::db::{new_id, Db};
use crate::error::{invalid, Error, Result};
use crate::types::{Department, UserRole, WebUser, WebUserInput};

/// Fewer rounds in debug builds so tests stay fast. The count is stored with
/// each hash, so both kinds verify correctly.
#[cfg(debug_assertions)]
const PBKDF2_ROUNDS: u32 = 2_000;
#[cfg(not(debug_assertions))]
const PBKDF2_ROUNDS: u32 = 120_000;

/// How long a login stays valid.
const SESSION_DAYS: i64 = 30;

pub const MIN_PASSWORD_LEN: usize = 6;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn random_hex() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn pbkdf2_sha256(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    type H = Hmac<Sha256>;
    let mut mac = <H as Mac>::new_from_slice(password).expect("HMAC accepts any key length");
    mac.update(salt);
    mac.update(&1u32.to_be_bytes());
    let mut u = [0u8; 32];
    u.copy_from_slice(&mac.finalize().into_bytes());
    let mut out = u;
    for _ in 1..rounds {
        let mut mac = <H as Mac>::new_from_slice(password).expect("HMAC accepts any key length");
        mac.update(&u);
        u.copy_from_slice(&mac.finalize().into_bytes());
        for (o, b) in out.iter_mut().zip(u.iter()) {
            *o ^= b;
        }
    }
    out
}

pub(crate) fn hash_password(password: &str) -> String {
    let salt = uuid::Uuid::new_v4().simple().to_string();
    let hash = pbkdf2_sha256(password.as_bytes(), salt.as_bytes(), PBKDF2_ROUNDS);
    format!("pbkdf2${PBKDF2_ROUNDS}${salt}${}", hex::encode(hash))
}

pub(crate) fn check_password(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    let [scheme, rounds, salt, hash] = parts.as_slice() else {
        return false;
    };
    let Ok(rounds) = rounds.parse::<u32>() else {
        return false;
    };
    if *scheme != "pbkdf2" || rounds == 0 {
        return false;
    }
    let Ok(expected) = hex::decode(hash) else {
        return false;
    };
    let got = pbkdf2_sha256(password.as_bytes(), salt.as_bytes(), rounds);
    // Constant-time comparison.
    expected.len() == got.len() && expected.iter().zip(got.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

const USER_SELECT: &str = "
    SELECT u.id, u.username, u.full_name, u.department_id, d.name, u.role, u.active
    FROM web_users u
    JOIN departments d ON d.id = u.department_id";

fn user_from_row(r: &Row<'_>) -> rusqlite::Result<WebUser> {
    let role: String = r.get(5)?;
    Ok(WebUser {
        id: r.get(0)?,
        username: r.get(1)?,
        full_name: r.get(2)?,
        department_id: r.get(3)?,
        department_name: r.get(4)?,
        role: UserRole::parse(&role).unwrap_or(UserRole::Requester),
        active: r.get::<_, i64>(6)? != 0,
    })
}

pub(crate) fn get_user(conn: &Connection, id: &str) -> Result<WebUser> {
    conn.query_row(&format!("{USER_SELECT} WHERE u.id = ?1"), [id], user_from_row)
        .optional()?
        .ok_or_else(|| Error::NotFound("user".into()))
}

pub(crate) fn department_name(conn: &Connection, id: &str) -> Result<String> {
    conn.query_row("SELECT name FROM departments WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| Error::Validation("Choose a valid department.".into()))
}

fn valid_username(s: &str) -> bool {
    (3..=32).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl Db {
    pub fn list_departments(&self) -> Result<Vec<Department>> {
        self.read(|c| {
            let mut stmt = c.prepare("SELECT id, name FROM departments ORDER BY name")?;
            let rows = stmt.query_map([], |r| Ok(Department { id: r.get(0)?, name: r.get(1)? }))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn list_web_users(&self) -> Result<Vec<WebUser>> {
        self.read(|c| {
            let mut stmt = c.prepare(&format!("{USER_SELECT} ORDER BY d.name, u.full_name"))?;
            let rows = stmt.query_map([], user_from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Creates a user (`id = None`) or edits one. On edit, an empty password
    /// keeps the current one.
    pub fn save_web_user(&self, id: Option<&str>, input: &WebUserInput) -> Result<WebUser> {
        let username = input.username.trim().to_string();
        let full_name = input.full_name.trim().to_string();
        let password = input.password.as_str();
        if !valid_username(&username) {
            return invalid("Username must be 3–32 characters: letters, numbers, dot, dash or underscore (no spaces).");
        }
        if full_name.is_empty() {
            return invalid("Full name is required.");
        }
        let changing_password = id.is_none() || !password.is_empty();
        if changing_password && password.chars().count() < MIN_PASSWORD_LEN {
            return invalid(format!("Password must be at least {MIN_PASSWORD_LEN} characters."));
        }
        // Hash outside the database lock (it takes a moment on purpose).
        let hashed = changing_password.then(|| hash_password(password));
        self.write(|c| {
            department_name(c, &input.department_id)?;
            let taken: Option<String> = c
                .query_row(
                    "SELECT id FROM web_users WHERE username = ?1 COLLATE NOCASE AND id != ?2",
                    params![username, id.unwrap_or("")],
                    |r| r.get(0),
                )
                .optional()?;
            if taken.is_some() {
                return invalid(format!("Username \"{username}\" is already used."));
            }
            let uid = match id {
                None => {
                    let uid = new_id();
                    c.execute(
                        "INSERT INTO web_users (id, username, full_name, department_id, role, password, active, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            uid,
                            username,
                            full_name,
                            input.department_id,
                            input.role.as_str(),
                            hashed.as_deref().unwrap_or_default(),
                            input.active as i64,
                            now_ms()
                        ],
                    )?;
                    uid
                }
                Some(uid) => {
                    get_user(c, uid)?;
                    c.execute(
                        "UPDATE web_users SET username = ?2, full_name = ?3, department_id = ?4, role = ?5, active = ?6
                         WHERE id = ?1",
                        params![uid, username, full_name, input.department_id, input.role.as_str(), input.active as i64],
                    )?;
                    if let Some(h) = &hashed {
                        c.execute("UPDATE web_users SET password = ?2 WHERE id = ?1", params![uid, h])?;
                    }
                    // Changed password or deactivated: log out everywhere.
                    if hashed.is_some() || !input.active {
                        c.execute("DELETE FROM web_sessions WHERE user_id = ?1", [uid])?;
                    }
                    uid.to_string()
                }
            };
            get_user(c, &uid)
        })
    }

    pub fn delete_web_user(&self, id: &str) -> Result<()> {
        self.write(|c| {
            get_user(c, id)?;
            c.execute("DELETE FROM web_sessions WHERE user_id = ?1", [id])?;
            c.execute("DELETE FROM web_users WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    /// Checks the password and starts a session. Returns the session token
    /// (give it to the browser) and the user.
    pub fn web_login(&self, username: &str, password: &str) -> Result<(String, WebUser)> {
        let found: Option<(String, String, bool)> = self.read(|c| {
            Ok(c.query_row(
                "SELECT id, password, active FROM web_users WHERE username = ?1 COLLATE NOCASE",
                [username.trim()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)),
            )
            .optional()?)
        })?;
        let wrong = || Error::Validation("Wrong username or password.".into());
        let (uid, stored, active) = match found {
            Some(f) => f,
            None => {
                // Spend the same time as a real check so usernames can't be guessed by timing.
                let _ = check_password(password, &hash_password("x"));
                return Err(wrong());
            }
        };
        if !check_password(password, &stored) {
            return Err(wrong());
        }
        if !active {
            return invalid("This account is disabled. Ask the warehouse to enable it.");
        }
        let token = random_hex();
        let expires = now_ms() + SESSION_DAYS * 24 * 3600 * 1000;
        self.write(|c| {
            c.execute("DELETE FROM web_sessions WHERE expires_at < ?1", [now_ms()])?;
            c.execute(
                "INSERT INTO web_sessions (token_hash, user_id, expires_at) VALUES (?1, ?2, ?3)",
                params![sha256_hex(&token), uid, expires],
            )?;
            Ok((token, get_user(c, &uid)?))
        })
    }

    /// The logged-in user for a session token, or `None` if the token is
    /// unknown, expired, or the account was disabled.
    pub fn web_session_user(&self, token: &str) -> Result<Option<WebUser>> {
        if token.is_empty() {
            return Ok(None);
        }
        self.read(|c| {
            let uid: Option<String> = c
                .query_row(
                    "SELECT s.user_id FROM web_sessions s JOIN web_users u ON u.id = s.user_id
                     WHERE s.token_hash = ?1 AND s.expires_at > ?2 AND u.active = 1",
                    params![sha256_hex(token), now_ms()],
                    |r| r.get(0),
                )
                .optional()?;
            match uid {
                Some(uid) => Ok(Some(get_user(c, &uid)?)),
                None => Ok(None),
            }
        })
    }

    pub fn web_logout(&self, token: &str) -> Result<()> {
        self.write(|c| {
            c.execute("DELETE FROM web_sessions WHERE token_hash = ?1", [sha256_hex(token)])?;
            Ok(())
        })
    }

    /// Lets a logged-in user change their own password.
    pub fn web_change_password(&self, user_id: &str, old: &str, new: &str) -> Result<()> {
        if new.chars().count() < MIN_PASSWORD_LEN {
            return invalid(format!("New password must be at least {MIN_PASSWORD_LEN} characters."));
        }
        let stored: String =
            self.read(|c| Ok(c.query_row("SELECT password FROM web_users WHERE id = ?1", [user_id], |r| r.get(0))?))?;
        if !check_password(old, &stored) {
            return invalid("Current password is wrong.");
        }
        let hashed = hash_password(new);
        self.write(|c| {
            c.execute("UPDATE web_users SET password = ?2 WHERE id = ?1", params![user_id, hashed])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let h = hash_password("secret123");
        assert!(check_password("secret123", &h));
        assert!(!check_password("secret124", &h));
        assert!(!check_password("secret123", "garbage"));
    }

    #[test]
    fn pbkdf2_known_vector() {
        // RFC 7914 §11 test vector for PBKDF2-HMAC-SHA256, c = 1.
        let out = pbkdf2_sha256(b"passwd", b"salt", 1);
        assert_eq!(hex::encode(out), "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc");
    }
}
