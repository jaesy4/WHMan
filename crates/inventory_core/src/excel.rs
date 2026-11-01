//! Excel import (calamine) and export (rust_xlsxwriter).
//!
//! **Import** is two steps so nothing is saved by mistake:
//! 1. [`parse`] reads the file and returns an [`ImportPreview`] (row counts,
//!    errors per Excel row, a sample of the data) plus the parsed rows.
//! 2. [`Db::commit_import`] saves the valid rows in one transaction.
//!
//! Two kinds of sheet are detected automatically from the header row:
//! * **Items** — Item Code, Item Name, Unit, Location, Reorder Level,
//!   Beginning Balance
//! * **Movements** — Date, Item Code, Type (IN/OUT), Quantity, Reference,
//!   Received From / Issued To, Remarks — *or* separate "In" and "Out" columns
//!
//! Column order does not matter and common header names are recognised.

use std::collections::HashMap;
use std::io::Cursor;

use calamine::{open_workbook_auto_from_rs, Data, Reader};
use chrono::{Duration, NaiveDate};
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook, Worksheet};

use crate::db::Db;
use crate::error::{invalid, Error, Result};
use crate::items::{find_by_code, insert_item, update_item_row};
use crate::movements::insert_movement;
use crate::qty::{from_milli, to_milli};
use crate::types::{
    ImportIssue, ImportKind, ImportPreview, ImportResult, ItemInput, Movement, MovementInput, MovementKind,
    StockCard, StockRow, SummaryRow,
};

// ================================================================ import

#[derive(Debug, Clone)]
pub struct ItemImportRow {
    pub excel_row: u32,
    pub input: ItemInput,
    pub beginning: f64,
}

#[derive(Debug, Clone)]
pub struct MovementImportRow {
    pub excel_row: u32,
    pub item_code: String,
    pub kind: MovementKind,
    pub qty: f64,
    pub date: String,
    pub reference: String,
    pub party: String,
    pub remarks: String,
    pub recorded_by: String,
}

#[derive(Debug, Clone)]
pub enum ParsedRows {
    Items(Vec<ItemImportRow>),
    Movements(Vec<MovementImportRow>),
}

/// Result of reading a file; keep it until the user confirms the import.
#[derive(Debug, Clone)]
pub struct ParsedImport {
    pub preview: ImportPreview,
    pub rows: ParsedRows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Field {
    Code,
    Name,
    Unit,
    Location,
    Reorder,
    Beginning,
    Date,
    Type,
    Qty,
    QtyIn,
    QtyOut,
    Reference,
    Party,
    Remarks,
    RecordedBy,
}

fn norm(h: &str) -> String {
    h.trim()
        .trim_end_matches(':')
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn field_for(header: &str) -> Option<Field> {
    let h = norm(header);
    let f = match h.as_str() {
        "item code" | "code" | "sku" | "item no" | "item no." | "item #" | "item number" | "part no"
        | "part no." | "part number" | "stock no" | "stock no." | "stock code" | "product code" | "barcode" => {
            Field::Code
        }
        "item name" | "name" | "description" | "item description" | "item" | "particulars" | "product"
        | "product name" | "article" => Field::Name,
        "unit" | "uom" | "unit of measure" | "units" | "u/m" => Field::Unit,
        "location" | "bin" | "bin location" | "rack" | "shelf" | "area" => Field::Location,
        "reorder level" | "reorder" | "reorder point" | "re-order level" | "min" | "minimum" | "min qty"
        | "minimum qty" | "safety stock" => Field::Reorder,
        "beginning balance" | "beginning" | "opening balance" | "opening" | "balance" | "on hand"
        | "qty on hand" | "stock on hand" | "current stock" | "stock" | "beginning inventory" => Field::Beginning,
        "date" | "transaction date" | "trans date" | "date received" | "date issued" => Field::Date,
        "type" | "in/out" | "in / out" | "transaction" | "trans type" | "transaction type" | "movement" => {
            Field::Type
        }
        "quantity" | "qty" => Field::Qty,
        "in" | "qty in" | "stock in" | "received" | "quantity in" => Field::QtyIn,
        "out" | "qty out" | "stock out" | "issued" | "quantity out" => Field::QtyOut,
        "reference" | "ref" | "ref no" | "ref no." | "ref #" | "reference no" | "reference no." | "dr no"
        | "dr no." | "rs no" | "rs no." | "document no" | "document no." | "doc no" | "doc no." | "slip no"
        | "slip no." => Field::Reference,
        "received from / issued to" | "received from/issued to" | "party" | "from/to" | "from / to"
        | "received from" | "issued to" | "supplier" | "requested by" | "supplier / requester" | "department"
        | "project" => Field::Party,
        "remarks" | "notes" | "note" | "comment" | "comments" => Field::Remarks,
        "recorded by" | "encoded by" | "warehouseman" | "prepared by" | "checked by" => Field::RecordedBy,
        _ => return None,
    };
    Some(f)
}

fn cell_str(c: &Data) -> String {
    match c {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        other => other.to_string().trim().to_string(),
    }
}

fn cell_num(c: &Data) -> Option<f64> {
    let v = match c {
        Data::Float(f) => Some(*f),
        Data::Int(i) => Some(*i as f64),
        Data::Empty => Some(0.0),
        Data::String(s) => {
            let s = s.trim().replace(',', "");
            if s.is_empty() || s == "-" {
                Some(0.0)
            } else {
                s.parse::<f64>().ok()
            }
        }
        _ => None,
    };
    v.filter(|v| v.is_finite())
}

fn serial_to_date(serial: f64) -> Option<String> {
    if !(1.0..=2_958_465.0).contains(&serial) {
        return None;
    }
    let base = NaiveDate::from_ymd_opt(1899, 12, 30)?;
    let d = base.checked_add_signed(Duration::days(serial.floor() as i64))?;
    Some(d.format("%Y-%m-%d").to_string())
}

fn parse_date_text(s: &str) -> Option<String> {
    let s = s.trim();
    let head = s.get(..10).unwrap_or(s);
    if let Ok(d) = NaiveDate::parse_from_str(head, "%Y-%m-%d") {
        return Some(d.format("%Y-%m-%d").to_string());
    }
    for fmt in ["%m/%d/%Y", "%m/%d/%y", "%m-%d-%Y", "%d-%b-%Y", "%d-%b-%y", "%b %d, %Y", "%B %d, %Y", "%Y/%m/%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return Some(d.format("%Y-%m-%d").to_string());
        }
    }
    None
}

fn cell_date(c: &Data) -> Option<String> {
    match c {
        Data::DateTime(dt) => serial_to_date(dt.as_f64()),
        Data::Float(f) => serial_to_date(*f),
        Data::Int(i) => serial_to_date(*i as f64),
        Data::DateTimeIso(s) | Data::String(s) => parse_date_text(s),
        _ => None,
    }
}

struct Sheet {
    name: String,
    header_row: usize,
    cols: HashMap<Field, usize>,
    rows: Vec<Vec<Data>>,
}

/// Finds the first sheet with a recognisable header row (within its first
/// 15 rows) that has an item code column.
fn find_sheet(bytes: &[u8]) -> Result<Sheet> {
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|e| Error::Excel(format!("Cannot open the file as Excel (.xlsx/.xls): {e}")))?;
    for name in wb.sheet_names() {
        let range = match wb.worksheet_range(&name) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rows: Vec<Vec<Data>> = range.rows().map(|r| r.to_vec()).collect();
        for (i, row) in rows.iter().enumerate().take(15) {
            let mut cols = HashMap::new();
            for (j, cell) in row.iter().enumerate() {
                if let Some(f) = field_for(&cell_str(cell)) {
                    cols.entry(f).or_insert(j);
                }
            }
            if cols.contains_key(&Field::Code) && cols.len() >= 2 {
                return Ok(Sheet { name, header_row: i, cols, rows });
            }
        }
    }
    invalid(
        "No sheet with an \"Item Code\" header was found. Use the template from Import > Download Template.",
    )
}

/// Reads an Excel file (bytes) and validates it against the current data.
/// Nothing is saved yet.
pub fn parse(db: &Db, bytes: &[u8], file_name: &str) -> Result<ParsedImport> {
    let sheet = find_sheet(bytes)?;
    let is_movements = sheet.cols.contains_key(&Field::Type)
        || (sheet.cols.contains_key(&Field::QtyIn) && sheet.cols.contains_key(&Field::QtyOut))
        || (sheet.cols.contains_key(&Field::Date) && sheet.cols.contains_key(&Field::Qty));
    if is_movements {
        parse_movements(db, sheet, file_name)
    } else {
        parse_items(db, sheet, file_name)
    }
}

fn get<'a>(row: &'a [Data], cols: &HashMap<Field, usize>, f: Field) -> Option<&'a Data> {
    cols.get(&f).and_then(|&i| row.get(i))
}

fn get_str(row: &[Data], cols: &HashMap<Field, usize>, f: Field) -> String {
    get(row, cols, f).map(cell_str).unwrap_or_default()
}

fn is_blank(row: &[Data]) -> bool {
    row.iter().all(|c| cell_str(c).is_empty())
}

fn parse_items(db: &Db, sheet: Sheet, file_name: &str) -> Result<ParsedImport> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut out: Vec<ItemImportRow> = Vec::new();
    let mut seen: HashMap<String, u32> = HashMap::new();
    let (mut new_items, mut updated) = (0u32, 0u32);
    if !sheet.cols.contains_key(&Field::Name) {
        return invalid("The item sheet needs an \"Item Name\" column.");
    }

    db.read(|c| {
        for (idx, row) in sheet.rows.iter().enumerate().skip(sheet.header_row + 1) {
            let excel_row = idx as u32 + 1;
            if is_blank(row) {
                continue;
            }
            let code = get_str(row, &sheet.cols, Field::Code);
            let name = get_str(row, &sheet.cols, Field::Name);
            let err = |m: String| ImportIssue { row: excel_row, message: m };
            if code.is_empty() {
                errors.push(err("Item code is empty.".into()));
                continue;
            }
            if name.is_empty() {
                errors.push(err(format!("{code}: item name is empty.")));
                continue;
            }
            if let Some(first) = seen.get(&code.to_lowercase()) {
                errors.push(err(format!("{code} appears again (first on row {first}).")));
                continue;
            }
            let reorder = match get(row, &sheet.cols, Field::Reorder).map(cell_num) {
                Some(None) => {
                    errors.push(err(format!("{code}: reorder level is not a number.")));
                    continue;
                }
                Some(Some(v)) if v < 0.0 => {
                    errors.push(err(format!("{code}: reorder level cannot be negative.")));
                    continue;
                }
                Some(Some(v)) => v,
                None => 0.0,
            };
            let beginning = match get(row, &sheet.cols, Field::Beginning).map(cell_num) {
                Some(None) => {
                    errors.push(err(format!("{code}: beginning balance is not a number.")));
                    continue;
                }
                Some(Some(v)) if v < 0.0 => {
                    errors.push(err(format!("{code}: beginning balance cannot be negative.")));
                    continue;
                }
                Some(Some(v)) => v,
                None => 0.0,
            };
            if find_by_code(c, &code)?.is_some() {
                updated += 1;
                if to_milli(beginning) != 0 {
                    warnings.push(err(format!(
                        "{code} already exists: details will be updated, beginning balance ignored (use Stock In instead)."
                    )));
                }
            } else {
                new_items += 1;
            }
            seen.insert(code.to_lowercase(), excel_row);
            out.push(ItemImportRow {
                excel_row,
                input: ItemInput {
                    code,
                    name,
                    unit: get_str(row, &sheet.cols, Field::Unit),
                    location: get_str(row, &sheet.cols, Field::Location),
                    reorder_level: reorder,
                    remarks: get_str(row, &sheet.cols, Field::Remarks),
                },
                beginning,
            });
        }
        Ok(())
    })?;

    let sample_headers = ["Item Code", "Item Name", "Unit", "Location", "Reorder", "Beginning"]
        .map(String::from)
        .to_vec();
    let sample = out
        .iter()
        .take(20)
        .map(|r| {
            vec![
                r.input.code.clone(),
                r.input.name.clone(),
                r.input.unit.clone(),
                r.input.location.clone(),
                crate::qty::format(r.input.reorder_level),
                crate::qty::format(r.beginning),
            ]
        })
        .collect();
    let movements = out.iter().filter(|r| to_milli(r.beginning) > 0).count() as u32;
    Ok(ParsedImport {
        preview: ImportPreview {
            kind: ImportKind::Items,
            sheet: sheet.name,
            file_name: file_name.to_string(),
            valid_rows: out.len() as u32,
            new_items,
            updated_items: updated,
            movements,
            errors,
            warnings,
            sample_headers,
            sample,
        },
        rows: ParsedRows::Items(out),
    })
}

fn parse_movements(db: &Db, sheet: Sheet, file_name: &str) -> Result<ParsedImport> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut out: Vec<MovementImportRow> = Vec::new();
    let cols = &sheet.cols;
    let has_type = cols.contains_key(&Field::Type);
    let has_split = cols.contains_key(&Field::QtyIn) || cols.contains_key(&Field::QtyOut);
    if !cols.contains_key(&Field::Date) {
        return invalid("The movement sheet needs a \"Date\" column.");
    }
    if !has_type && !has_split {
        return invalid("The movement sheet needs a \"Type\" (IN/OUT) column, or \"In\" and \"Out\" columns.");
    }
    let today = crate::today();

    db.read(|c| {
        // Running balance per item to catch OUTs that exceed stock.
        let mut balances: HashMap<String, (i64, String, String)> = HashMap::new();
        let mut pending: Vec<MovementImportRow> = Vec::new();
        for (idx, row) in sheet.rows.iter().enumerate().skip(sheet.header_row + 1) {
            let excel_row = idx as u32 + 1;
            if is_blank(row) {
                continue;
            }
            let err = |m: String| ImportIssue { row: excel_row, message: m };
            let code = get_str(row, cols, Field::Code);
            if code.is_empty() {
                errors.push(err("Item code is empty.".into()));
                continue;
            }
            let Some(date) = get(row, cols, Field::Date).and_then(cell_date) else {
                errors.push(err(format!("{code}: date is missing or not a valid date.")));
                continue;
            };
            if date > today {
                warnings.push(err(format!("{code}: date {date} is in the future.")));
            }
            // Either Type + Quantity, or separate In / Out columns.
            let mut entries: Vec<(MovementKind, f64)> = Vec::new();
            if has_type {
                let t = get_str(row, cols, Field::Type);
                let Some(kind) = MovementKind::parse(&t) else {
                    errors.push(err(format!("{code}: type \"{t}\" is not IN or OUT.")));
                    continue;
                };
                match get(row, cols, Field::Qty).and_then(cell_num) {
                    Some(q) if q > 0.0 => entries.push((kind, q)),
                    _ => {
                        errors.push(err(format!("{code}: quantity must be a number more than zero.")));
                        continue;
                    }
                }
            } else {
                let qin = get(row, cols, Field::QtyIn).map(cell_num).unwrap_or(Some(0.0));
                let qout = get(row, cols, Field::QtyOut).map(cell_num).unwrap_or(Some(0.0));
                match (qin, qout) {
                    (Some(i), Some(o)) if i >= 0.0 && o >= 0.0 => {
                        if to_milli(i) > 0 {
                            entries.push((MovementKind::In, i));
                        }
                        if to_milli(o) > 0 {
                            entries.push((MovementKind::Out, o));
                        }
                        if entries.is_empty() {
                            errors.push(err(format!("{code}: both In and Out are empty.")));
                            continue;
                        }
                    }
                    _ => {
                        errors.push(err(format!("{code}: In/Out must be numbers (zero or more).")));
                        continue;
                    }
                }
            }
            if !balances.contains_key(&code.to_lowercase()) {
                match find_by_code(c, &code)? {
                    Some(item) => {
                        let bal = crate::items::balance_milli(c, &item.id)?;
                        balances.insert(code.to_lowercase(), (bal, item.unit, item.name));
                    }
                    None => {
                        errors.push(err(format!("{code}: item not found. Import or add the item first.")));
                        continue;
                    }
                }
            }
            for (kind, qty) in entries {
                pending.push(MovementImportRow {
                    excel_row,
                    item_code: code.clone(),
                    kind,
                    qty,
                    date: date.clone(),
                    reference: get_str(row, cols, Field::Reference),
                    party: get_str(row, cols, Field::Party),
                    remarks: get_str(row, cols, Field::Remarks),
                    recorded_by: get_str(row, cols, Field::RecordedBy),
                });
            }
        }
        // Apply in date order (stable: same-date rows keep sheet order, IN before OUT on the same row).
        pending.sort_by(|a, b| a.date.cmp(&b.date));
        for m in pending {
            let key = m.item_code.to_lowercase();
            let (bal, unit, _name) = balances.get_mut(&key).expect("balance loaded");
            let q = to_milli(m.qty);
            if m.kind == MovementKind::Out && q > *bal {
                errors.push(ImportIssue {
                    row: m.excel_row,
                    message: format!(
                        "{}: OUT {} {} exceeds the balance of {} {} at that point.",
                        m.item_code,
                        crate::qty::format(m.qty),
                        unit,
                        crate::qty::format(from_milli(*bal)),
                        unit
                    ),
                });
                continue;
            }
            *bal += if m.kind == MovementKind::In { q } else { -q };
            out.push(m);
        }
        Ok(())
    })?;

    errors.sort_by_key(|e| e.row);
    let sample_headers = ["Date", "Item Code", "Type", "Qty", "Reference", "From / To"]
        .map(String::from)
        .to_vec();
    let sample = out
        .iter()
        .take(20)
        .map(|m| {
            vec![
                m.date.clone(),
                m.item_code.clone(),
                m.kind.as_str().to_string(),
                crate::qty::format(m.qty),
                m.reference.clone(),
                m.party.clone(),
            ]
        })
        .collect();
    Ok(ParsedImport {
        preview: ImportPreview {
            kind: ImportKind::Movements,
            sheet: sheet.name,
            file_name: file_name.to_string(),
            valid_rows: out.len() as u32,
            new_items: 0,
            updated_items: 0,
            movements: out.len() as u32,
            errors,
            warnings,
            sample_headers,
            sample,
        },
        rows: ParsedRows::Movements(out),
    })
}

impl Db {
    /// Saves the valid rows of a previewed import, all-or-nothing.
    /// `recorded_by` is used when the sheet has no "Recorded By" column.
    pub fn commit_import(&self, parsed: &ParsedImport, recorded_by: &str) -> Result<ImportResult> {
        let today = crate::today();
        self.write(|c| {
            let mut res = ImportResult { items_created: 0, items_updated: 0, movements_created: 0 };
            match &parsed.rows {
                ParsedRows::Items(rows) => {
                    for r in rows {
                        let fail = |e: Error| Error::Validation(format!("Row {}: {e}", r.excel_row));
                        match find_by_code(c, &r.input.code)? {
                            Some(existing) => {
                                update_item_row(c, &existing.id, &r.input).map_err(fail)?;
                                res.items_updated += 1;
                            }
                            None => {
                                let item = insert_item(c, &r.input).map_err(fail)?;
                                res.items_created += 1;
                                if to_milli(r.beginning) > 0 {
                                    insert_movement(
                                        c,
                                        &MovementInput {
                                            item_id: item.id,
                                            kind: MovementKind::In,
                                            qty: r.beginning,
                                            date: today.clone(),
                                            reference: "BEGINNING BALANCE".into(),
                                            party: String::new(),
                                            remarks: format!("Imported from {}", parsed.preview.file_name),
                                            recorded_by: recorded_by.to_string(),
                                        },
                                    )
                                    .map_err(fail)?;
                                    res.movements_created += 1;
                                }
                            }
                        }
                    }
                }
                ParsedRows::Movements(rows) => {
                    for r in rows {
                        let fail = |e: Error| Error::Validation(format!("Row {}: {e}", r.excel_row));
                        let item = find_by_code(c, &r.item_code)?
                            .ok_or_else(|| fail(Error::NotFound(format!("item {}", r.item_code))))?;
                        insert_movement(
                            c,
                            &MovementInput {
                                item_id: item.id,
                                kind: r.kind,
                                qty: r.qty,
                                date: r.date.clone(),
                                reference: r.reference.clone(),
                                party: r.party.clone(),
                                remarks: r.remarks.clone(),
                                recorded_by: if r.recorded_by.is_empty() {
                                    recorded_by.to_string()
                                } else {
                                    r.recorded_by.clone()
                                },
                            },
                        )
                        .map_err(fail)?;
                        res.movements_created += 1;
                    }
                }
            }
            Ok(res)
        })
    }
}

// ================================================================ export

struct Styles {
    title: Format,
    sub: Format,
    header: Format,
    text: Format,
    int: Format,
    dec: Format,
    warn_text: Format,
    warn_int: Format,
    warn_dec: Format,
    bold_text: Format,
    bold_int: Format,
    bold_dec: Format,
}

impl Styles {
    fn new() -> Self {
        let navy = Color::RGB(0x26456B);
        let warn = Color::RGB(0xFFEDD9);
        let total = Color::RGB(0xE6EBF2);
        let cell = || Format::new().set_border(FormatBorder::Thin).set_border_color(Color::RGB(0xD0D5DC));
        Self {
            title: Format::new().set_bold().set_font_size(15).set_font_color(navy),
            sub: Format::new().set_italic().set_font_color(Color::RGB(0x6B6B70)),
            header: Format::new()
                .set_bold()
                .set_font_color(Color::White)
                .set_background_color(navy)
                .set_align(FormatAlign::Center)
                .set_align(FormatAlign::VerticalCenter)
                .set_text_wrap()
                .set_border(FormatBorder::Thin),
            text: cell(),
            int: cell().set_num_format("#,##0"),
            dec: cell().set_num_format("#,##0.000"),
            warn_text: cell().set_background_color(warn),
            warn_int: cell().set_background_color(warn).set_num_format("#,##0"),
            warn_dec: cell().set_background_color(warn).set_num_format("#,##0.000"),
            bold_text: cell().set_bold().set_background_color(total),
            bold_int: cell().set_bold().set_background_color(total).set_num_format("#,##0"),
            bold_dec: cell().set_bold().set_background_color(total).set_num_format("#,##0.000"),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Normal,
    Warn,
    Bold,
}

enum Cell<'a> {
    Text(&'a str),
    Qty(f64),
    Blank,
}

struct SheetWriter<'w> {
    ws: &'w mut Worksheet,
    st: &'w Styles,
    row: u32,
    ncols: u16,
}

impl<'w> SheetWriter<'w> {
    fn new(ws: &'w mut Worksheet, st: &'w Styles, title: &str, subtitle: &[String], headers: &[(&str, f64)]) -> Result<Self> {
        ws.write_string_with_format(0, 0, title, &st.title)?;
        let mut row = 1u32;
        for s in subtitle {
            ws.write_string_with_format(row, 0, s, &st.sub)?;
            row += 1;
        }
        row += 1;
        for (i, (h, w)) in headers.iter().enumerate() {
            ws.write_string_with_format(row, i as u16, *h, &st.header)?;
            ws.set_column_width(i as u16, *w)?;
        }
        ws.set_row_height(row, 22)?;
        ws.set_freeze_panes(row + 1, 0)?;
        let ncols = headers.len() as u16;
        Ok(Self { ws, st, row: row + 1, ncols })
    }

    fn header_row(&self) -> u32 {
        // The row just above the first data row when nothing was written yet.
        self.row - 1
    }

    fn write_row(&mut self, cells: &[Cell<'_>], look: Look) -> Result<()> {
        let st = self.st;
        let (t, i, d) = match look {
            Look::Normal => (&st.text, &st.int, &st.dec),
            Look::Warn => (&st.warn_text, &st.warn_int, &st.warn_dec),
            Look::Bold => (&st.bold_text, &st.bold_int, &st.bold_dec),
        };
        for (col, cell) in cells.iter().enumerate() {
            let col = col as u16;
            match cell {
                Cell::Text(s) => {
                    self.ws.write_string_with_format(self.row, col, *s, t)?;
                }
                Cell::Qty(q) => {
                    let f = if to_milli(*q) % 1000 == 0 { i } else { d };
                    self.ws.write_number_with_format(self.row, col, *q, f)?;
                }
                Cell::Blank => {
                    self.ws.write_blank(self.row, col, t)?;
                }
            }
        }
        self.row += 1;
        Ok(())
    }

    fn finish(self, header_row: u32) -> Result<()> {
        if self.row > header_row + 1 {
            self.ws.autofilter(header_row, 0, self.row - 1, self.ncols - 1)?;
        }
        Ok(())
    }
}

fn period_text(from: Option<&str>, to: Option<&str>) -> String {
    match (from.filter(|s| !s.is_empty()), to.filter(|s| !s.is_empty())) {
        (Some(f), Some(t)) => format!("Period: {f} to {t}"),
        (Some(f), None) => format!("Period: from {f}"),
        (None, Some(t)) => format!("Period: up to {t}"),
        (None, None) => "Period: all records".into(),
    }
}

/// Current inventory (Total In / Total Out / Balance) as `.xlsx` bytes.
pub fn export_inventory(org: &str, rows: &[StockRow], as_of: &str) -> Result<Vec<u8>> {
    let st = Styles::new();
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("Inventory")?;
    let sub = vec![org.to_string(), format!("As of {as_of}")];
    let mut w = SheetWriter::new(
        ws,
        &st,
        "Inventory Report",
        &sub,
        &[
            ("Item Code", 14.0),
            ("Item Name", 38.0),
            ("Unit", 8.0),
            ("Location", 14.0),
            ("Total In", 12.0),
            ("Total Out", 12.0),
            ("Balance", 12.0),
            ("Reorder Level", 12.0),
            ("Status", 10.0),
        ],
    )?;
    let hr = w.header_row();
    for r in rows {
        w.write_row(
            &[
                Cell::Text(&r.item.code),
                Cell::Text(&r.item.name),
                Cell::Text(&r.item.unit),
                Cell::Text(&r.item.location),
                Cell::Qty(r.total_in),
                Cell::Qty(r.total_out),
                Cell::Qty(r.balance),
                Cell::Qty(r.item.reorder_level),
                Cell::Text(if r.low { "REORDER" } else { "OK" }),
            ],
            if r.low { Look::Warn } else { Look::Normal },
        )?;
    }
    w.finish(hr)?;
    Ok(wb.save_to_buffer()?)
}

/// Beginning / In / Out / Ending Balance per item for a period.
pub fn export_summary(org: &str, rows: &[SummaryRow], from: Option<&str>, to: Option<&str>) -> Result<Vec<u8>> {
    let st = Styles::new();
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("In-Out Summary")?;
    let sub = vec![org.to_string(), period_text(from, to)];
    let mut w = SheetWriter::new(
        ws,
        &st,
        "Stock In / Stock Out Summary",
        &sub,
        &[
            ("Item Code", 14.0),
            ("Item Name", 38.0),
            ("Unit", 8.0),
            ("Beginning", 12.0),
            ("In", 12.0),
            ("Out", 12.0),
            ("Ending Balance", 14.0),
        ],
    )?;
    let hr = w.header_row();
    for r in rows {
        w.write_row(
            &[
                Cell::Text(&r.code),
                Cell::Text(&r.name),
                Cell::Text(&r.unit),
                Cell::Qty(r.beginning),
                Cell::Qty(r.total_in),
                Cell::Qty(r.total_out),
                Cell::Qty(r.ending),
            ],
            Look::Normal,
        )?;
    }
    w.finish(hr)?;
    Ok(wb.save_to_buffer()?)
}

/// Movement log (Stock In / Stock Out records).
pub fn export_movements(org: &str, movements: &[Movement], from: Option<&str>, to: Option<&str>) -> Result<Vec<u8>> {
    let st = Styles::new();
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("Transactions")?;
    let sub = vec![org.to_string(), period_text(from, to)];
    let mut w = SheetWriter::new(
        ws,
        &st,
        "Stock In / Stock Out Transactions",
        &sub,
        &[
            ("Date", 12.0),
            ("Type", 8.0),
            ("Reference", 14.0),
            ("Item Code", 14.0),
            ("Item Name", 34.0),
            ("Quantity", 11.0),
            ("Unit", 8.0),
            ("Received From / Issued To", 26.0),
            ("Recorded By", 16.0),
            ("Remarks", 28.0),
        ],
    )?;
    let hr = w.header_row();
    for m in movements {
        w.write_row(
            &[
                Cell::Text(&m.date),
                Cell::Text(if m.voided { "VOID" } else { m.kind.as_str() }),
                Cell::Text(&m.reference),
                Cell::Text(&m.item_code),
                Cell::Text(&m.item_name),
                Cell::Qty(m.qty),
                Cell::Text(&m.unit),
                Cell::Text(&m.party),
                Cell::Text(&m.recorded_by),
                Cell::Text(&m.remarks),
            ],
            Look::Normal,
        )?;
    }
    w.finish(hr)?;
    Ok(wb.save_to_buffer()?)
}

/// Stock card of one item.
pub fn export_stock_card(org: &str, card: &StockCard) -> Result<Vec<u8>> {
    let st = Styles::new();
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("Stock Card")?;
    let it = &card.item;
    let sub = vec![
        org.to_string(),
        format!("{} - {} ({})", it.code, it.name, it.unit),
        period_text(card.from.as_deref(), card.to.as_deref()),
    ];
    let mut w = SheetWriter::new(
        ws,
        &st,
        "Stock Card",
        &sub,
        &[
            ("Date", 12.0),
            ("Reference", 16.0),
            ("Received From / Issued To", 28.0),
            ("In", 11.0),
            ("Out", 11.0),
            ("Balance", 12.0),
            ("Remarks", 28.0),
        ],
    )?;
    let hr = w.header_row();
    let from = card.from.clone().unwrap_or_default();
    w.write_row(
        &[
            Cell::Text(&from),
            Cell::Text("Beginning balance"),
            Cell::Blank,
            Cell::Blank,
            Cell::Blank,
            Cell::Qty(card.beginning),
            Cell::Blank,
        ],
        Look::Bold,
    )?;
    for l in &card.lines {
        w.write_row(
            &[
                Cell::Text(&l.date),
                Cell::Text(&l.reference),
                Cell::Text(&l.party),
                if l.qty_in != 0.0 { Cell::Qty(l.qty_in) } else { Cell::Blank },
                if l.qty_out != 0.0 { Cell::Qty(l.qty_out) } else { Cell::Blank },
                Cell::Qty(l.balance),
                Cell::Text(&l.remarks),
            ],
            Look::Normal,
        )?;
    }
    let tin: f64 = card.lines.iter().map(|l| l.qty_in).sum();
    let tout: f64 = card.lines.iter().map(|l| l.qty_out).sum();
    let to = card.to.clone().unwrap_or_default();
    w.write_row(
        &[
            Cell::Text(&to),
            Cell::Text("Ending balance"),
            Cell::Blank,
            Cell::Qty(tin),
            Cell::Qty(tout),
            Cell::Qty(card.ending),
            Cell::Blank,
        ],
        Look::Bold,
    )?;
    w.finish(hr)?;
    Ok(wb.save_to_buffer()?)
}

/// Blank import templates with example rows.
pub fn template(kind: ImportKind) -> Result<Vec<u8>> {
    let st = Styles::new();
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    match kind {
        ImportKind::Items => {
            ws.set_name("Items")?;
            let sub = vec!["Fill one row per item. Item Code and Item Name are required. Delete the sample rows.".into()];
            let mut w = SheetWriter::new(
                ws,
                &st,
                "Item List Import",
                &sub,
                &[
                    ("Item Code", 14.0),
                    ("Item Name", 38.0),
                    ("Unit", 8.0),
                    ("Location", 14.0),
                    ("Reorder Level", 13.0),
                    ("Beginning Balance", 15.0),
                    ("Remarks", 24.0),
                ],
            )?;
            w.write_row(
                &[
                    Cell::Text("CEM-001"),
                    Cell::Text("Portland Cement 40kg"),
                    Cell::Text("bag"),
                    Cell::Text("Rack A1"),
                    Cell::Qty(50.0),
                    Cell::Qty(120.0),
                    Cell::Blank,
                ],
                Look::Normal,
            )?;
            w.write_row(
                &[
                    Cell::Text("NAIL-2"),
                    Cell::Text("Common Nails 2 inch"),
                    Cell::Text("kg"),
                    Cell::Text("Bin B3"),
                    Cell::Qty(10.0),
                    Cell::Qty(25.5),
                    Cell::Blank,
                ],
                Look::Normal,
            )?;
        }
        ImportKind::Movements => {
            ws.set_name("Stock In-Out")?;
            let sub = vec!["Type is IN or OUT. Items must already exist. Delete the sample rows.".into()];
            let mut w = SheetWriter::new(
                ws,
                &st,
                "Stock In / Stock Out Import",
                &sub,
                &[
                    ("Date", 12.0),
                    ("Item Code", 14.0),
                    ("Type", 8.0),
                    ("Quantity", 11.0),
                    ("Reference", 14.0),
                    ("Received From / Issued To", 28.0),
                    ("Remarks", 24.0),
                    ("Recorded By", 16.0),
                ],
            )?;
            let today = crate::today();
            w.write_row(
                &[
                    Cell::Text(&today),
                    Cell::Text("CEM-001"),
                    Cell::Text("IN"),
                    Cell::Qty(200.0),
                    Cell::Text("DR-10021"),
                    Cell::Text("Supplier name"),
                    Cell::Blank,
                    Cell::Blank,
                ],
                Look::Normal,
            )?;
            w.write_row(
                &[
                    Cell::Text(&today),
                    Cell::Text("CEM-001"),
                    Cell::Text("OUT"),
                    Cell::Qty(45.0),
                    Cell::Text("RS-0311"),
                    Cell::Text("Site / requester"),
                    Cell::Blank,
                    Cell::Blank,
                ],
                Look::Normal,
            )?;
        }
    }
    Ok(wb.save_to_buffer()?)
}
