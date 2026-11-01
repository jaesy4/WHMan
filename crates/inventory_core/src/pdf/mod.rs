//! Printable warehouse reports. Every function takes data already loaded from
//! the database and returns PDF bytes, ready to save or share.

pub mod engine;

use engine::{render, Align, Col, Doc, Row, RowStyle};

use crate::qty::format as fq;
use crate::types::{Movement, MovementKind, StockCard, StockRow, SummaryRow};

/// Text shared by every report header/footer.
#[derive(Debug, Clone, Default)]
pub struct ReportMeta {
    /// Warehouse / company name shown above the title.
    pub org: String,
    /// When the report was generated, e.g. "2026-11-01 14:05".
    pub generated: String,
    /// Name printed after "Prepared by" (optional).
    pub prepared_by: String,
}

fn period(from: Option<&str>, to: Option<&str>) -> String {
    match (from.filter(|s| !s.is_empty()), to.filter(|s| !s.is_empty())) {
        (Some(f), Some(t)) if f == t => format!("Date: {f}"),
        (Some(f), Some(t)) => format!("Period: {f} to {t}"),
        (Some(f), None) => format!("Period: from {f}"),
        (None, Some(t)) => format!("Period: up to {t}"),
        (None, None) => "Period: all records".to_string(),
    }
}

fn signatures() -> Vec<String> {
    vec!["Prepared by".into(), "Checked by".into(), "Noted by".into()]
}

fn prepared_line(meta: &ReportMeta) -> Vec<String> {
    if meta.prepared_by.trim().is_empty() {
        vec![]
    } else {
        vec![format!("Prepared by: {}", meta.prepared_by.trim())]
    }
}

/// Current inventory: Total In, Total Out and Balance per item.
pub fn inventory_report(meta: &ReportMeta, rows: &[StockRow], as_of: &str) -> Vec<u8> {
    let low = rows.iter().filter(|r| r.low).count();
    let mut subtitle = vec![format!(
        "As of {as_of}   |   {} items   |   {low} at or below reorder level",
        rows.len()
    )];
    subtitle.extend(prepared_line(meta));
    let doc = Doc {
        org: meta.org.clone(),
        title: "Inventory Report".into(),
        subtitle,
        columns: vec![
            Col::new("Item Code", 1.3, Align::Left),
            Col::new("Item Name", 3.2, Align::Left),
            Col::new("Unit", 0.8, Align::Left),
            Col::new("Location", 1.2, Align::Left),
            Col::new("Total In", 1.1, Align::Right),
            Col::new("Total Out", 1.1, Align::Right),
            Col::new("Balance", 1.1, Align::Right),
            Col::new("Reorder", 0.9, Align::Right),
            Col::new("Status", 0.9, Align::Center),
        ],
        rows: rows
            .iter()
            .map(|r| {
                Row::styled(
                    vec![
                        r.item.code.clone(),
                        r.item.name.clone(),
                        r.item.unit.clone(),
                        r.item.location.clone(),
                        fq(r.total_in),
                        fq(r.total_out),
                        fq(r.balance),
                        if r.item.reorder_level > 0.0 { fq(r.item.reorder_level) } else { "-".into() },
                        if r.low { "REORDER".into() } else { "OK".into() },
                    ],
                    if r.low { RowStyle::Warn } else { RowStyle::Normal },
                )
            })
            .collect(),
        landscape: true,
        note: Some("Highlighted rows are at or below their reorder level.".into()),
        signatures: signatures(),
        generated: meta.generated.clone(),
        empty_text: Some("No items yet.".into()),
        ..Default::default()
    };
    render(&doc)
}

/// Beginning / In / Out / Ending per item for a period.
pub fn summary_report(meta: &ReportMeta, rows: &[SummaryRow], from: Option<&str>, to: Option<&str>) -> Vec<u8> {
    let moved = rows.iter().filter(|r| r.total_in != 0.0 || r.total_out != 0.0).count();
    let mut subtitle = vec![format!("{}   |   {} items, {moved} with movement", period(from, to), rows.len())];
    subtitle.extend(prepared_line(meta));
    let doc = Doc {
        org: meta.org.clone(),
        title: "Stock In / Stock Out Summary".into(),
        subtitle,
        columns: vec![
            Col::new("Item Code", 1.3, Align::Left),
            Col::new("Item Name", 3.4, Align::Left),
            Col::new("Unit", 0.8, Align::Left),
            Col::new("Beginning", 1.1, Align::Right),
            Col::new("In", 1.0, Align::Right),
            Col::new("Out", 1.0, Align::Right),
            Col::new("Ending Balance", 1.3, Align::Right),
        ],
        rows: rows
            .iter()
            .map(|r| {
                Row::new(vec![
                    r.code.clone(),
                    r.name.clone(),
                    r.unit.clone(),
                    fq(r.beginning),
                    fq(r.total_in),
                    fq(r.total_out),
                    fq(r.ending),
                ])
            })
            .collect(),
        landscape: true,
        note: Some("Ending Balance = Beginning + In - Out. Quantities are in each item's own unit.".into()),
        signatures: signatures(),
        generated: meta.generated.clone(),
        empty_text: Some("No items yet.".into()),
        ..Default::default()
    };
    render(&doc)
}

/// Stock card of a single item with running balance.
pub fn stock_card_report(meta: &ReportMeta, card: &StockCard) -> Vec<u8> {
    let it = &card.item;
    let mut rows = vec![Row::styled(
        vec![
            card.from.clone().unwrap_or_default(),
            "Beginning balance".into(),
            String::new(),
            String::new(),
            String::new(),
            fq(card.beginning),
            String::new(),
        ],
        RowStyle::Bold,
    )];
    rows.extend(card.lines.iter().map(|l| {
        Row::new(vec![
            l.date.clone(),
            l.reference.clone(),
            l.party.clone(),
            if l.qty_in != 0.0 { fq(l.qty_in) } else { String::new() },
            if l.qty_out != 0.0 { fq(l.qty_out) } else { String::new() },
            fq(l.balance),
            l.remarks.clone(),
        ])
    }));
    let total_in: f64 = card.lines.iter().map(|l| l.qty_in).sum();
    let total_out: f64 = card.lines.iter().map(|l| l.qty_out).sum();
    rows.push(Row::styled(
        vec![
            card.to.clone().unwrap_or_default(),
            "Ending balance".into(),
            String::new(),
            fq(total_in),
            fq(total_out),
            fq(card.ending),
            String::new(),
        ],
        RowStyle::Bold,
    ));
    let doc = Doc {
        org: meta.org.clone(),
        title: "Stock Card".into(),
        subtitle: vec![period(card.from.as_deref(), card.to.as_deref())],
        info: vec![
            ("Item Code".into(), it.code.clone()),
            ("Unit".into(), it.unit.clone()),
            ("Item Name".into(), it.name.clone()),
            ("Location".into(), it.location.clone()),
            ("Reorder Level".into(), if it.reorder_level > 0.0 { fq(it.reorder_level) } else { "-".into() }),
            ("Prepared by".into(), meta.prepared_by.clone()),
        ],
        columns: vec![
            Col::new("Date", 1.1, Align::Left),
            Col::new("Reference", 1.5, Align::Left),
            Col::new("Received From / Issued To", 2.2, Align::Left),
            Col::new("In", 0.9, Align::Right),
            Col::new("Out", 0.9, Align::Right),
            Col::new("Balance", 1.0, Align::Right),
            Col::new("Remarks", 1.6, Align::Left),
        ],
        rows,
        landscape: false,
        signatures: vec!["Prepared by".into(), "Checked by".into()],
        generated: meta.generated.clone(),
        ..Default::default()
    };
    render(&doc)
}

/// Log of Stock In / Stock Out records (receiving or issuance report).
pub fn movements_report(
    meta: &ReportMeta,
    movements: &[Movement],
    kind: Option<MovementKind>,
    from: Option<&str>,
    to: Option<&str>,
) -> Vec<u8> {
    let title = match kind {
        Some(MovementKind::In) => "Stock In (Receiving) Report",
        Some(MovementKind::Out) => "Stock Out (Issuance) Report",
        None => "Stock In / Stock Out Transactions",
    };
    let party = match kind {
        Some(MovementKind::In) => "Received From",
        Some(MovementKind::Out) => "Issued To",
        None => "Received From / Issued To",
    };
    let active = movements.iter().filter(|m| !m.voided).count();
    let mut subtitle = vec![format!("{}   |   {active} records", period(from, to))];
    subtitle.extend(prepared_line(meta));
    let doc = Doc {
        org: meta.org.clone(),
        title: title.into(),
        subtitle,
        columns: vec![
            Col::new("Date", 1.0, Align::Left),
            Col::new("Type", 0.6, Align::Center),
            Col::new("Reference", 1.3, Align::Left),
            Col::new("Item Code", 1.2, Align::Left),
            Col::new("Item Name", 2.6, Align::Left),
            Col::new("Qty", 0.9, Align::Right),
            Col::new("Unit", 0.7, Align::Left),
            Col::new(party, 1.9, Align::Left),
            Col::new("Recorded By", 1.2, Align::Left),
            Col::new("Remarks", 1.6, Align::Left),
        ],
        rows: movements
            .iter()
            .map(|m| {
                Row::styled(
                    vec![
                        m.date.clone(),
                        if m.voided { "VOID".into() } else { m.kind.as_str().into() },
                        m.reference.clone(),
                        m.item_code.clone(),
                        m.item_name.clone(),
                        fq(m.qty),
                        m.unit.clone(),
                        m.party.clone(),
                        m.recorded_by.clone(),
                        m.remarks.clone(),
                    ],
                    if m.voided { RowStyle::Muted } else { RowStyle::Normal },
                )
            })
            .collect(),
        landscape: true,
        signatures: signatures(),
        generated: meta.generated.clone(),
        empty_text: Some("No records for this period.".into()),
        ..Default::default()
    };
    render(&doc)
}

/// One-page Receiving Slip (IN) or Issue Slip (OUT) for a single record.
pub fn movement_slip(meta: &ReportMeta, m: &Movement, current_balance: f64) -> Vec<u8> {
    let (title, party_label, sigs) = match m.kind {
        MovementKind::In => (
            "Receiving Slip (Stock In)",
            "Received From",
            vec!["Delivered by".to_string(), "Received by".to_string()],
        ),
        MovementKind::Out => (
            "Issue Slip (Stock Out)",
            "Issued To",
            vec!["Issued by".to_string(), "Received by".to_string(), "Approved by".to_string()],
        ),
    };
    let doc = Doc {
        org: meta.org.clone(),
        title: if m.voided { format!("{title} - VOID") } else { title.into() },
        subtitle: vec![],
        info: vec![
            ("Date".into(), m.date.clone()),
            ("Reference No.".into(), m.reference.clone()),
            (party_label.into(), m.party.clone()),
            ("Recorded by".into(), m.recorded_by.clone()),
        ],
        columns: vec![
            Col::new("Item Code", 1.3, Align::Left),
            Col::new("Item Name", 3.0, Align::Left),
            Col::new("Unit", 0.8, Align::Left),
            Col::new("Quantity", 1.0, Align::Right),
            Col::new("Current Balance", 1.2, Align::Right),
        ],
        rows: vec![Row::new(vec![
            m.item_code.clone(),
            m.item_name.clone(),
            m.unit.clone(),
            fq(m.qty),
            fq(current_balance),
        ])],
        note: if m.remarks.is_empty() { None } else { Some(format!("Remarks: {}", m.remarks)) },
        signatures: sigs,
        generated: meta.generated.clone(),
        ..Default::default()
    };
    render(&doc)
}
