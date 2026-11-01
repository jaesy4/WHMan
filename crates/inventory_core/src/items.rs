//! Item master list.

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::{local_device, new_id, next_seq, next_stamp, Db};
use crate::error::{invalid, Error, Result};
use crate::qty::{from_milli, to_milli};
use crate::types::{Item, ItemInput, StockRow};

pub(crate) const ITEM_COLS: &str = "id, code, name, unit, location, reorder_milli, remarks";

pub(crate) fn item_from_row(r: &Row<'_>) -> rusqlite::Result<Item> {
    Ok(Item {
        id: r.get(0)?,
        code: r.get(1)?,
        name: r.get(2)?,
        unit: r.get(3)?,
        location: r.get(4)?,
        reorder_level: from_milli(r.get(5)?),
        remarks: r.get(6)?,
    })
}

pub(crate) fn clean(input: &ItemInput) -> Result<ItemInput> {
    let out = ItemInput {
        code: input.code.trim().to_string(),
        name: input.name.trim().to_string(),
        unit: input.unit.trim().to_string(),
        location: input.location.trim().to_string(),
        reorder_level: input.reorder_level,
        remarks: input.remarks.trim().to_string(),
    };
    if out.code.is_empty() {
        return invalid("Item code is required.");
    }
    if out.name.is_empty() {
        return invalid("Item name is required.");
    }
    if !out.reorder_level.is_finite() || out.reorder_level < 0.0 {
        return invalid("Reorder level must be zero or more.");
    }
    Ok(out)
}

pub(crate) fn get_item(conn: &Connection, id: &str) -> Result<Item> {
    conn.query_row(
        &format!("SELECT {ITEM_COLS} FROM items WHERE id = ?1 AND deleted = 0"),
        [id],
        item_from_row,
    )
    .optional()?
    .ok_or_else(|| Error::NotFound("item".into()))
}

pub(crate) fn find_by_code(conn: &Connection, code: &str) -> Result<Option<Item>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {ITEM_COLS} FROM items
                 WHERE deleted = 0 AND code = ?1 COLLATE NOCASE LIMIT 1"
            ),
            [code.trim()],
            item_from_row,
        )
        .optional()?)
}

pub(crate) fn insert_item(conn: &Connection, input: &ItemInput) -> Result<Item> {
    let input = clean(input)?;
    if find_by_code(conn, &input.code)?.is_some() {
        return invalid(format!("Item code \"{}\" already exists.", input.code));
    }
    let id = new_id();
    conn.execute(
        "INSERT INTO items (id, code, name, unit, location, reorder_milli, remarks,
                            deleted, updated_at, updated_by, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?10)",
        params![
            id,
            input.code,
            input.name,
            input.unit,
            input.location,
            to_milli(input.reorder_level),
            input.remarks,
            next_stamp(conn)?,
            local_device(conn)?,
            next_seq(conn)?,
        ],
    )?;
    get_item(conn, &id)
}

pub(crate) fn update_item_row(conn: &Connection, id: &str, input: &ItemInput) -> Result<Item> {
    let input = clean(input)?;
    get_item(conn, id)?;
    if let Some(other) = find_by_code(conn, &input.code)? {
        if other.id != id {
            return invalid(format!("Item code \"{}\" is already used by {}.", input.code, other.name));
        }
    }
    conn.execute(
        "UPDATE items SET code = ?2, name = ?3, unit = ?4, location = ?5, reorder_milli = ?6,
                remarks = ?7, updated_at = ?8, updated_by = ?9, seq = ?10
         WHERE id = ?1",
        params![
            id,
            input.code,
            input.name,
            input.unit,
            input.location,
            to_milli(input.reorder_level),
            input.remarks,
            next_stamp(conn)?,
            local_device(conn)?,
            next_seq(conn)?,
        ],
    )?;
    get_item(conn, id)
}

/// Balance of one item in thousandths, ignoring voided records.
pub(crate) fn balance_milli(conn: &Connection, item_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(CASE kind WHEN 'IN' THEN qty_milli ELSE -qty_milli END), 0)
         FROM movements WHERE item_id = ?1 AND voided = 0",
        [item_id],
        |r| r.get(0),
    )?)
}

impl Db {
    pub fn create_item(&self, input: &ItemInput) -> Result<Item> {
        self.write(|c| insert_item(c, input))
    }

    pub fn update_item(&self, id: &str, input: &ItemInput) -> Result<Item> {
        self.write(|c| update_item_row(c, id, input))
    }

    /// Removes an item from the list. Only allowed when its balance is zero,
    /// so stock can never "disappear" by deleting an item.
    pub fn delete_item(&self, id: &str) -> Result<()> {
        self.write(|c| {
            let item = get_item(c, id)?;
            let bal = balance_milli(c, id)?;
            if bal != 0 {
                return invalid(format!(
                    "{} still has a balance of {} {}. Issue or adjust the stock to zero before deleting.",
                    item.name,
                    crate::qty::format(from_milli(bal)),
                    item.unit
                ));
            }
            c.execute(
                "UPDATE items SET deleted = 1, updated_at = ?2, updated_by = ?3, seq = ?4 WHERE id = ?1",
                params![id, next_stamp(c)?, local_device(c)?, next_seq(c)?],
            )?;
            Ok(())
        })
    }

    pub fn get_item(&self, id: &str) -> Result<Item> {
        self.read(|c| get_item(c, id))
    }

    pub fn find_item_by_code(&self, code: &str) -> Result<Option<Item>> {
        self.read(|c| find_by_code(c, code))
    }

    /// The inventory list: every item with Total In, Total Out and Balance.
    pub fn stock_list(&self, search: &str, low_only: bool) -> Result<Vec<StockRow>> {
        self.read(|c| stock_list(c, search, low_only))
    }

    pub fn item_balance(&self, item_id: &str) -> Result<f64> {
        self.read(|c| Ok(from_milli(balance_milli(c, item_id)?)))
    }
}

pub(crate) fn stock_list(conn: &Connection, search: &str, low_only: bool) -> Result<Vec<StockRow>> {
    let search = search.trim();
    let like = format!("%{search}%");
    let mut stmt = conn.prepare(
        "SELECT i.id, i.code, i.name, i.unit, i.location, i.reorder_milli, i.remarks,
                COALESCE(SUM(CASE WHEN m.kind = 'IN'  THEN m.qty_milli END), 0) AS tin,
                COALESCE(SUM(CASE WHEN m.kind = 'OUT' THEN m.qty_milli END), 0) AS tout
         FROM items i
         LEFT JOIN movements m ON m.item_id = i.id AND m.voided = 0
         WHERE i.deleted = 0
           AND (?1 = '' OR i.code LIKE ?2 OR i.name LIKE ?2 OR i.location LIKE ?2)
         GROUP BY i.id
         ORDER BY i.code COLLATE NOCASE",
    )?;
    let rows = stmt.query_map(params![search, like], |r| {
        let item = item_from_row(r)?;
        let tin: i64 = r.get(7)?;
        let tout: i64 = r.get(8)?;
        Ok((item, tin, tout))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (item, tin, tout) = row?;
        let bal = tin - tout;
        let reorder = to_milli(item.reorder_level);
        let low = reorder > 0 && bal <= reorder;
        if low_only && !low {
            continue;
        }
        out.push(StockRow {
            item,
            total_in: from_milli(tin),
            total_out: from_milli(tout),
            balance: from_milli(bal),
            low,
        });
    }
    Ok(out)
}
