//! Warehouse reports: In/Out summary, stock card, dashboard.

use rusqlite::params;

use crate::db::Db;
use crate::error::Result;
use crate::items::{get_item, stock_list};
use crate::qty::from_milli;
use crate::types::{Dashboard, StockCard, StockCardLine, SummaryRow};

const MIN_DATE: &str = "0000-01-01";
const MAX_DATE: &str = "9999-12-31";

fn range<'a>(from: Option<&'a str>, to: Option<&'a str>) -> (&'a str, &'a str) {
    (
        from.filter(|s| !s.is_empty()).unwrap_or(MIN_DATE),
        to.filter(|s| !s.is_empty()).unwrap_or(MAX_DATE),
    )
}

impl Db {
    /// Beginning balance, total IN, total OUT and ending balance per item for
    /// a date range. Records dated after `to` are ignored.
    pub fn inout_summary(&self, from: Option<&str>, to: Option<&str>) -> Result<Vec<SummaryRow>> {
        let (from, to) = range(from, to);
        self.read(|c| {
            let mut stmt = c.prepare(
                "SELECT i.id, i.code, i.name, i.unit,
                    COALESCE(SUM(CASE WHEN m.date < ?1
                        THEN (CASE m.kind WHEN 'IN' THEN m.qty_milli ELSE -m.qty_milli END) END), 0),
                    COALESCE(SUM(CASE WHEN m.date >= ?1 AND m.date <= ?2 AND m.kind = 'IN'
                        THEN m.qty_milli END), 0),
                    COALESCE(SUM(CASE WHEN m.date >= ?1 AND m.date <= ?2 AND m.kind = 'OUT'
                        THEN m.qty_milli END), 0)
                 FROM items i
                 LEFT JOIN movements m ON m.item_id = i.id AND m.voided = 0
                 WHERE i.deleted = 0
                 GROUP BY i.id
                 ORDER BY i.code COLLATE NOCASE",
            )?;
            let rows = stmt.query_map(params![from, to], |r| {
                let beg: i64 = r.get(4)?;
                let tin: i64 = r.get(5)?;
                let tout: i64 = r.get(6)?;
                Ok(SummaryRow {
                    item_id: r.get(0)?,
                    code: r.get(1)?,
                    name: r.get(2)?,
                    unit: r.get(3)?,
                    beginning: from_milli(beg),
                    total_in: from_milli(tin),
                    total_out: from_milli(tout),
                    ending: from_milli(beg + tin - tout),
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Stock card of one item: every IN/OUT with a running balance.
    pub fn stock_card(&self, item_id: &str, from: Option<&str>, to: Option<&str>) -> Result<StockCard> {
        let (f, t) = range(from, to);
        self.read(|c| {
            let item = get_item(c, item_id)?;
            let beginning: i64 = c.query_row(
                "SELECT COALESCE(SUM(CASE kind WHEN 'IN' THEN qty_milli ELSE -qty_milli END), 0)
                 FROM movements WHERE item_id = ?1 AND voided = 0 AND date < ?2",
                params![item_id, f],
                |r| r.get(0),
            )?;
            let mut stmt = c.prepare(
                "SELECT date, reference, party, kind, qty_milli, remarks
                 FROM movements
                 WHERE item_id = ?1 AND voided = 0 AND date >= ?2 AND date <= ?3
                 ORDER BY date, created_at",
            )?;
            let rows = stmt.query_map(params![item_id, f, t], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
            let mut bal = beginning;
            let mut lines = Vec::new();
            for row in rows {
                let (date, reference, party, kind, q, remarks) = row?;
                let (qin, qout) = if kind == "IN" { (q, 0) } else { (0, q) };
                bal += qin - qout;
                lines.push(StockCardLine {
                    date,
                    reference,
                    party,
                    qty_in: from_milli(qin),
                    qty_out: from_milli(qout),
                    balance: from_milli(bal),
                    remarks,
                });
            }
            Ok(StockCard {
                item,
                from: from.filter(|s| !s.is_empty()).map(str::to_string),
                to: to.filter(|s| !s.is_empty()).map(str::to_string),
                beginning: from_milli(beginning),
                lines,
                ending: from_milli(bal),
            })
        })
    }

    pub fn dashboard(&self, today: &str) -> Result<Dashboard> {
        self.read(|c| {
            let all = stock_list(c, "", false)?;
            let (tin, tout, nin, nout): (i64, i64, i64, i64) = c.query_row(
                "SELECT COALESCE(SUM(CASE WHEN kind = 'IN'  THEN qty_milli END), 0),
                        COALESCE(SUM(CASE WHEN kind = 'OUT' THEN qty_milli END), 0),
                        COUNT(CASE WHEN kind = 'IN'  THEN 1 END),
                        COUNT(CASE WHEN kind = 'OUT' THEN 1 END)
                 FROM movements m JOIN items i ON i.id = m.item_id
                 WHERE m.voided = 0 AND i.deleted = 0 AND m.date = ?1",
                [today],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            Ok(Dashboard {
                item_count: all.len() as u32,
                low_stock_count: all.iter().filter(|r| r.low).count() as u32,
                today_in: from_milli(tin),
                today_out: from_milli(tout),
                today_in_count: nin as u32,
                today_out_count: nout as u32,
            })
        })
    }
}
