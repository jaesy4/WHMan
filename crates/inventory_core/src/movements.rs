//! Stock In / Stock Out records.

use rusqlite::{params, Connection, OptionalExtension, Row, ToSql};

use crate::db::{local_device, new_id, next_seq, next_stamp, Db};
use crate::error::{invalid, Error, Result};
use crate::items::{balance_milli, get_item};
use crate::qty::{format as fq, from_milli, to_milli};
use crate::types::{Movement, MovementFilter, MovementInput, MovementKind};

const MOV_SELECT: &str = "
    SELECT m.id, m.item_id, i.code, i.name, i.unit, m.kind, m.qty_milli, m.date,
           m.reference, m.party, m.remarks, m.recorded_by, m.voided
    FROM movements m
    JOIN items i ON i.id = m.item_id";

fn movement_from_row(r: &Row<'_>) -> rusqlite::Result<Movement> {
    let kind: String = r.get(5)?;
    Ok(Movement {
        id: r.get(0)?,
        item_id: r.get(1)?,
        item_code: r.get(2)?,
        item_name: r.get(3)?,
        unit: r.get(4)?,
        kind: if kind == "OUT" { MovementKind::Out } else { MovementKind::In },
        qty: from_milli(r.get(6)?),
        date: r.get(7)?,
        reference: r.get(8)?,
        party: r.get(9)?,
        remarks: r.get(10)?,
        recorded_by: r.get(11)?,
        voided: r.get::<_, i64>(12)? != 0,
    })
}

pub(crate) fn get_movement(conn: &Connection, id: &str) -> Result<Movement> {
    conn.query_row(&format!("{MOV_SELECT} WHERE m.id = ?1"), [id], movement_from_row)
        .optional()?
        .ok_or_else(|| Error::NotFound("record".into()))
}

/// Validates and saves one IN/OUT record. An OUT may not take the balance
/// below zero.
pub(crate) fn insert_movement(conn: &Connection, input: &MovementInput) -> Result<Movement> {
    let item = get_item(conn, &input.item_id)?;
    if !input.qty.is_finite() || to_milli(input.qty) <= 0 {
        return invalid("Quantity must be more than zero.");
    }
    let date = input.date.trim();
    if !crate::valid_date(date) {
        return invalid("Date must be a valid date (YYYY-MM-DD).");
    }
    let qty = to_milli(input.qty);
    if input.kind == MovementKind::Out {
        let bal = balance_milli(conn, &item.id)?;
        if qty > bal {
            return invalid(format!(
                "Not enough stock for {} ({}). Balance is {} {}, trying to issue {}.",
                item.name,
                item.code,
                fq(from_milli(bal)),
                item.unit,
                fq(input.qty)
            ));
        }
    }
    let id = new_id();
    let stamp = next_stamp(conn)?;
    conn.execute(
        "INSERT INTO movements (id, item_id, kind, qty_milli, date, reference, party, remarks,
                                recorded_by, created_at, voided, updated_at, updated_by, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?10, ?11, ?12)",
        params![
            id,
            item.id,
            input.kind.as_str(),
            qty,
            date,
            input.reference.trim(),
            input.party.trim(),
            input.remarks.trim(),
            input.recorded_by.trim(),
            stamp,
            local_device(conn)?,
            next_seq(conn)?,
        ],
    )?;
    get_movement(conn, &id)
}

impl Db {
    /// Records a Stock In or Stock Out.
    pub fn record_movement(&self, input: &MovementInput) -> Result<Movement> {
        self.write(|c| insert_movement(c, input))
    }

    /// Cancels a wrong entry. The record is kept (marked VOID) for audit.
    pub fn void_movement(&self, id: &str, reason: &str) -> Result<Movement> {
        self.write(|c| {
            let m = get_movement(c, id)?;
            if m.voided {
                return invalid("This record is already void.");
            }
            if m.kind == MovementKind::In {
                let bal = balance_milli(c, &m.item_id)?;
                if bal - to_milli(m.qty) < 0 {
                    return invalid(format!(
                        "Cannot void: part of this delivery was already issued. Balance would become {} {}.",
                        fq(from_milli(bal - to_milli(m.qty))),
                        m.unit
                    ));
                }
            }
            let reason = reason.trim();
            let remarks = match (m.remarks.is_empty(), reason.is_empty()) {
                (_, true) => format!("{} [VOID]", m.remarks).trim().to_string(),
                (true, false) => format!("[VOID: {reason}]"),
                (false, false) => format!("{} [VOID: {reason}]", m.remarks),
            };
            c.execute(
                "UPDATE movements SET voided = 1, remarks = ?2, updated_at = ?3, updated_by = ?4, seq = ?5
                 WHERE id = ?1",
                params![id, remarks, next_stamp(c)?, local_device(c)?, next_seq(c)?],
            )?;
            get_movement(c, id)
        })
    }

    pub fn get_movement(&self, id: &str) -> Result<Movement> {
        self.read(|c| get_movement(c, id))
    }

    /// Movement log, newest first.
    pub fn list_movements(&self, f: &MovementFilter) -> Result<Vec<Movement>> {
        self.read(|c| {
            let mut sql = format!("{MOV_SELECT} WHERE 1 = 1");
            let mut args: Vec<Box<dyn ToSql>> = Vec::new();
            if !f.include_voided {
                sql.push_str(" AND m.voided = 0");
            }
            if let Some(from) = f.from.as_deref().filter(|s| !s.is_empty()) {
                args.push(Box::new(from.to_string()));
                sql.push_str(&format!(" AND m.date >= ?{}", args.len()));
            }
            if let Some(to) = f.to.as_deref().filter(|s| !s.is_empty()) {
                args.push(Box::new(to.to_string()));
                sql.push_str(&format!(" AND m.date <= ?{}", args.len()));
            }
            if let Some(kind) = f.kind {
                args.push(Box::new(kind.as_str()));
                sql.push_str(&format!(" AND m.kind = ?{}", args.len()));
            }
            if let Some(item) = f.item_id.as_deref().filter(|s| !s.is_empty()) {
                args.push(Box::new(item.to_string()));
                sql.push_str(&format!(" AND m.item_id = ?{}", args.len()));
            }
            if let Some(q) = f.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                args.push(Box::new(format!("%{q}%")));
                let n = args.len();
                sql.push_str(&format!(
                    " AND (i.code LIKE ?{n} OR i.name LIKE ?{n} OR m.reference LIKE ?{n} OR m.party LIKE ?{n})"
                ));
            }
            sql.push_str(" ORDER BY m.date DESC, m.created_at DESC");
            if let Some(limit) = f.limit {
                sql.push_str(&format!(" LIMIT {limit}"));
            }
            let mut stmt = c.prepare(&sql)?;
            let refs: Vec<&dyn ToSql> = args.iter().map(|b| b.as_ref()).collect();
            let rows = stmt.query_map(refs.as_slice(), movement_from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }
}
