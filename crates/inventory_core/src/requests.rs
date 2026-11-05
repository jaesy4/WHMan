//! Department item requests.
//!
//! Flow: **Submitted** (by a requester) → **Approved** (by the department
//! head, or the warehouse) → **Issued** (by the warehouse; creates Stock Out
//! records) → **Received** (confirmed by the department).
//! A request can be **Rejected** (head / warehouse) or **Cancelled** (by the
//! department) while it is still Submitted or Approved.

use rusqlite::{params, Connection, OptionalExtension, Row, ToSql};

use crate::db::{get_meta, new_id, set_meta, Db};
use crate::error::{invalid, Error, Result};
use crate::items::{balance_milli, get_item};
use crate::movements::insert_movement;
use crate::qty::{format as fq, from_milli, to_milli};
use crate::types::{
    IssueInput, ItemRequest, MovementInput, MovementKind, RequestFilter, RequestInput, RequestLine, RequestStatus,
    UserRole, WebUser,
};
use crate::web_users::department_name;

const MAX_LINES: usize = 50;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Epoch ms → "YYYY-MM-DD HH:MM" in the PC's local time.
fn fmt_ms(ms: Option<i64>) -> String {
    use chrono::TimeZone;
    match ms.and_then(|m| chrono::Local.timestamp_millis_opt(m).single()) {
        Some(t) => t.format("%Y-%m-%d %H:%M").to_string(),
        None => String::new(),
    }
}

const REQ_SELECT: &str = "
    SELECT r.id, r.number, r.department_id, d.name, r.requested_by, r.purpose, r.needed_by, r.status,
           r.created_at, r.approved_by, r.approved_at, r.decision_note, r.issued_by, r.issued_at,
           r.received_by, r.received_at
    FROM requests r
    JOIN departments d ON d.id = r.department_id";

fn header_from_row(r: &Row<'_>) -> rusqlite::Result<ItemRequest> {
    let status: String = r.get(7)?;
    Ok(ItemRequest {
        id: r.get(0)?,
        number: r.get(1)?,
        department_id: r.get(2)?,
        department_name: r.get(3)?,
        requested_by: r.get(4)?,
        purpose: r.get(5)?,
        needed_by: r.get(6)?,
        status: RequestStatus::parse(&status).unwrap_or(RequestStatus::Submitted),
        created_at: fmt_ms(r.get(8)?),
        approved_by: r.get(9)?,
        approved_at: fmt_ms(r.get(10)?),
        decision_note: r.get(11)?,
        issued_by: r.get(12)?,
        issued_at: fmt_ms(r.get(13)?),
        received_by: r.get(14)?,
        received_at: fmt_ms(r.get(15)?),
        lines: Vec::new(),
    })
}

fn load_lines(conn: &Connection, request_id: &str) -> Result<Vec<RequestLine>> {
    let mut stmt = conn.prepare(
        "SELECT l.id, l.item_id, COALESCE(i.code, ''), COALESCE(i.name, '(deleted item)'), COALESCE(i.unit, ''),
                l.qty_milli, l.issued_milli, l.remarks
         FROM request_lines l
         LEFT JOIN items i ON i.id = l.item_id
         WHERE l.request_id = ?1
         ORDER BY l.line_no",
    )?;
    let rows = stmt.query_map([request_id], |r| {
        Ok(RequestLine {
            id: r.get(0)?,
            item_id: r.get(1)?,
            item_code: r.get(2)?,
            item_name: r.get(3)?,
            unit: r.get(4)?,
            qty_requested: from_milli(r.get(5)?),
            qty_issued: from_milli(r.get(6)?),
            remarks: r.get(7)?,
            balance: 0.0,
        })
    })?;
    let mut lines = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for l in &mut lines {
        l.balance = from_milli(balance_milli(conn, &l.item_id)?);
    }
    Ok(lines)
}

pub(crate) fn get_request(conn: &Connection, id: &str) -> Result<ItemRequest> {
    let mut req = conn
        .query_row(&format!("{REQ_SELECT} WHERE r.id = ?1"), [id], header_from_row)
        .optional()?
        .ok_or_else(|| Error::NotFound("request".into()))?;
    req.lines = load_lines(conn, &req.id)?;
    Ok(req)
}

/// Next number for this year: RQ-2026-0001, RQ-2026-0002, ...
fn next_number(conn: &Connection) -> Result<String> {
    let year = chrono::Local::now().format("%Y").to_string();
    let key = format!("request_no.{year}");
    let n = get_meta(conn, &key)?.and_then(|v| v.parse::<i64>().ok()).unwrap_or(0) + 1;
    set_meta(conn, &key, &n.to_string())?;
    Ok(format!("RQ-{year}-{n:04}"))
}

fn ensure_status(req: &ItemRequest, allowed: &[RequestStatus], action: &str) -> Result<()> {
    if allowed.contains(&req.status) {
        Ok(())
    } else {
        invalid(format!("Request {} is {} and can no longer be {action}.", req.number, req.status.label().to_lowercase()))
    }
}

/// Who is acting on a request.
#[derive(Debug, Clone)]
pub enum Actor<'a> {
    /// The warehouse man on the PC app (full rights).
    Warehouse(&'a str),
    /// A logged-in department user from the web portal.
    Web(&'a WebUser),
}

impl Actor<'_> {
    fn name(&self) -> String {
        match self {
            Actor::Warehouse(n) => {
                let n = n.trim();
                if n.is_empty() { "Warehouse".to_string() } else { n.to_string() }
            }
            Actor::Web(u) => u.full_name.clone(),
        }
    }
    fn check_department(&self, req: &ItemRequest) -> Result<()> {
        match self {
            Actor::Web(u) if u.department_id != req.department_id => {
                Err(Error::NotFound("request".into()))
            }
            _ => Ok(()),
        }
    }
}

impl Db {
    /// A department user files a new request. A department head's own
    /// requests are approved right away.
    pub fn create_request(&self, user: &WebUser, input: &RequestInput) -> Result<ItemRequest> {
        if input.lines.is_empty() {
            return invalid("Add at least one item to the request.");
        }
        if input.lines.len() > MAX_LINES {
            return invalid(format!("A request can have at most {MAX_LINES} items. Split it into two requests."));
        }
        let needed_by = input.needed_by.trim();
        if !needed_by.is_empty() && !crate::valid_date(needed_by) {
            return invalid("\"Needed by\" must be a valid date.");
        }
        if input.purpose.trim().is_empty() {
            return invalid("Please write the purpose of the request (e.g. project or site).");
        }
        self.write(|c| {
            department_name(c, &user.department_id)?;
            let mut seen = std::collections::HashSet::new();
            for l in &input.lines {
                let item = get_item(c, &l.item_id)?;
                if !l.qty.is_finite() || to_milli(l.qty) <= 0 {
                    return invalid(format!("Quantity for {} must be more than zero.", item.name));
                }
                if !seen.insert(l.item_id.clone()) {
                    return invalid(format!("{} is listed twice. Combine it into one line.", item.name));
                }
            }
            let id = new_id();
            let now = now_ms();
            let auto_approve = user.role == UserRole::Head;
            let status = if auto_approve { RequestStatus::Approved } else { RequestStatus::Submitted };
            c.execute(
                "INSERT INTO requests (id, number, department_id, user_id, requested_by, purpose, needed_by,
                                       status, created_at, approved_by, approved_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?9)",
                params![
                    id,
                    next_number(c)?,
                    user.department_id,
                    user.id,
                    user.full_name,
                    input.purpose.trim(),
                    needed_by,
                    status.as_str(),
                    now,
                    if auto_approve { user.full_name.as_str() } else { "" },
                    auto_approve.then_some(now),
                ],
            )?;
            for (n, l) in input.lines.iter().enumerate() {
                c.execute(
                    "INSERT INTO request_lines (id, request_id, line_no, item_id, qty_milli, remarks)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![new_id(), id, n as i64 + 1, l.item_id, to_milli(l.qty), l.remarks.trim()],
                )?;
            }
            get_request(c, &id)
        })
    }

    pub fn get_request(&self, id: &str) -> Result<ItemRequest> {
        self.read(|c| get_request(c, id))
    }

    /// Newest first.
    pub fn list_requests(&self, f: &RequestFilter) -> Result<Vec<ItemRequest>> {
        self.read(|c| {
            let mut sql = format!("{REQ_SELECT} WHERE 1 = 1");
            let mut args: Vec<Box<dyn ToSql>> = Vec::new();
            if let Some(s) = f.status {
                args.push(Box::new(s.as_str()));
                sql.push_str(&format!(" AND r.status = ?{}", args.len()));
            }
            if f.open_only {
                sql.push_str(" AND r.status IN ('SUBMITTED', 'APPROVED')");
            }
            if let Some(d) = f.department_id.as_deref().filter(|s| !s.is_empty()) {
                args.push(Box::new(d.to_string()));
                sql.push_str(&format!(" AND r.department_id = ?{}", args.len()));
            }
            if let Some(q) = f.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                args.push(Box::new(format!("%{q}%")));
                let n = args.len();
                sql.push_str(&format!(
                    " AND (r.number LIKE ?{n} OR r.requested_by LIKE ?{n} OR r.purpose LIKE ?{n}
                       OR EXISTS (SELECT 1 FROM request_lines l JOIN items i ON i.id = l.item_id
                                  WHERE l.request_id = r.id AND (i.code LIKE ?{n} OR i.name LIKE ?{n})))"
                ));
            }
            sql.push_str(" ORDER BY r.created_at DESC");
            sql.push_str(&format!(" LIMIT {}", f.limit.unwrap_or(200)));
            let mut stmt = c.prepare(&sql)?;
            let refs: Vec<&dyn ToSql> = args.iter().map(|b| b.as_ref()).collect();
            let rows = stmt.query_map(refs.as_slice(), header_from_row)?;
            let mut out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            for r in &mut out {
                r.lines = load_lines(c, &r.id)?;
            }
            Ok(out)
        })
    }

    /// Requests waiting for approval or for issuing.
    pub fn open_request_count(&self) -> Result<u32> {
        self.read(|c| {
            Ok(c.query_row("SELECT COUNT(*) FROM requests WHERE status IN ('SUBMITTED', 'APPROVED')", [], |r| {
                r.get::<_, i64>(0)
            })? as u32)
        })
    }

    /// Department head (own department) or warehouse approves.
    pub fn approve_request(&self, id: &str, actor: Actor<'_>, note: &str) -> Result<ItemRequest> {
        if let Actor::Web(u) = &actor {
            if u.role != UserRole::Head {
                return invalid("Only the department head can approve requests.");
            }
        }
        self.write(|c| {
            let req = get_request(c, id)?;
            actor.check_department(&req)?;
            ensure_status(&req, &[RequestStatus::Submitted], "approved")?;
            c.execute(
                "UPDATE requests SET status = 'APPROVED', approved_by = ?2, approved_at = ?3, decision_note = ?4,
                                     updated_at = ?3
                 WHERE id = ?1",
                params![id, actor.name(), now_ms(), note.trim()],
            )?;
            get_request(c, id)
        })
    }

    /// Department head (own department) or warehouse rejects. A reason is required.
    pub fn reject_request(&self, id: &str, actor: Actor<'_>, reason: &str) -> Result<ItemRequest> {
        if let Actor::Web(u) = &actor {
            if u.role != UserRole::Head {
                return invalid("Only the department head can reject requests.");
            }
        }
        if reason.trim().is_empty() {
            return invalid("Please give a reason for rejecting.");
        }
        self.write(|c| {
            let req = get_request(c, id)?;
            actor.check_department(&req)?;
            ensure_status(&req, &[RequestStatus::Submitted, RequestStatus::Approved], "rejected")?;
            c.execute(
                "UPDATE requests SET status = 'REJECTED', approved_by = ?2, approved_at = ?3, decision_note = ?4,
                                     updated_at = ?3
                 WHERE id = ?1",
                params![id, actor.name(), now_ms(), reason.trim()],
            )?;
            get_request(c, id)
        })
    }

    /// The person who filed it, or their department head, cancels it.
    pub fn cancel_request(&self, id: &str, user: &WebUser, reason: &str) -> Result<ItemRequest> {
        self.write(|c| {
            let req = get_request(c, id)?;
            Actor::Web(user).check_department(&req)?;
            let owner: String = c.query_row("SELECT user_id FROM requests WHERE id = ?1", [id], |r| r.get(0))?;
            if owner != user.id && user.role != UserRole::Head {
                return invalid("Only the person who filed this request or the department head can cancel it.");
            }
            ensure_status(&req, &[RequestStatus::Submitted, RequestStatus::Approved], "cancelled")?;
            let note = if reason.trim().is_empty() {
                format!("Cancelled by {}", user.full_name)
            } else {
                format!("Cancelled by {}: {}", user.full_name, reason.trim())
            };
            c.execute(
                "UPDATE requests SET status = 'CANCELLED', decision_note = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, note, now_ms()],
            )?;
            get_request(c, id)
        })
    }

    /// Warehouse releases an approved request. Creates one Stock Out per line
    /// with a quantity, all-or-nothing. Quantities can be lower than requested
    /// (partial issue); a line with 0 is not issued.
    pub fn issue_request(&self, input: &IssueInput) -> Result<ItemRequest> {
        let date = input.date.trim();
        if !crate::valid_date(date) {
            return invalid("Date must be a valid date (YYYY-MM-DD).");
        }
        self.write(|c| {
            let req = get_request(c, &input.request_id)?;
            ensure_status(&req, &[RequestStatus::Approved], "issued")?;
            let mut any = false;
            for line in &req.lines {
                let qty = input.lines.iter().find(|l| l.line_id == line.id).map(|l| l.qty).unwrap_or(0.0);
                if !qty.is_finite() || qty < 0.0 {
                    return invalid(format!("Quantity for {} cannot be negative.", line.item_name));
                }
                let qm = to_milli(qty);
                if qm == 0 {
                    continue;
                }
                if qm > to_milli(line.qty_requested) {
                    return invalid(format!(
                        "{}: cannot issue {} — only {} {} was requested.",
                        line.item_name,
                        fq(qty),
                        fq(line.qty_requested),
                        line.unit
                    ));
                }
                let remarks = if line.remarks.is_empty() {
                    input.remarks.trim().to_string()
                } else if input.remarks.trim().is_empty() {
                    line.remarks.clone()
                } else {
                    format!("{} / {}", line.remarks, input.remarks.trim())
                };
                // insert_movement checks that there is enough stock.
                let m = insert_movement(
                    c,
                    &MovementInput {
                        item_id: line.item_id.clone(),
                        kind: MovementKind::Out,
                        qty,
                        date: date.to_string(),
                        reference: req.number.clone(),
                        party: format!("{} - {}", req.department_name, req.requested_by),
                        remarks,
                        recorded_by: input.issued_by.clone(),
                    },
                )?;
                c.execute(
                    "UPDATE request_lines SET issued_milli = ?2, movement_id = ?3 WHERE id = ?1",
                    params![line.id, qm, m.id],
                )?;
                any = true;
            }
            if !any {
                return invalid("Enter the quantity to issue for at least one item.");
            }
            c.execute(
                "UPDATE requests SET status = 'ISSUED', issued_by = ?2, issued_at = ?3, updated_at = ?3 WHERE id = ?1",
                params![req.id, Actor::Warehouse(&input.issued_by).name(), now_ms()],
            )?;
            get_request(c, &req.id)
        })
    }

    /// The department confirms it received the items.
    pub fn mark_request_received(&self, id: &str, user: &WebUser) -> Result<ItemRequest> {
        self.write(|c| {
            let req = get_request(c, id)?;
            Actor::Web(user).check_department(&req)?;
            ensure_status(&req, &[RequestStatus::Issued], "marked as received")?;
            c.execute(
                "UPDATE requests SET status = 'RECEIVED', received_by = ?2, received_at = ?3, updated_at = ?3
                 WHERE id = ?1",
                params![id, user.full_name, now_ms()],
            )?;
            get_request(c, id)
        })
    }
}
