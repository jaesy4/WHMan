use inventory_core::types::*;
use inventory_core::{excel, pdf, Db, Error};

fn item(code: &str, name: &str, unit: &str, reorder: f64) -> ItemInput {
    ItemInput {
        code: code.into(),
        name: name.into(),
        unit: unit.into(),
        location: "Rack A".into(),
        reorder_level: reorder,
        remarks: String::new(),
    }
}

fn mv(item_id: &str, kind: MovementKind, qty: f64, date: &str) -> MovementInput {
    MovementInput {
        item_id: item_id.into(),
        kind,
        qty,
        date: date.into(),
        reference: "REF".into(),
        party: "Someone".into(),
        remarks: String::new(),
        recorded_by: "Tester".into(),
    }
}

#[test]
fn stock_in_out_balance_and_rules() {
    let db = Db::open_in_memory().unwrap();
    let cement = db.create_item(&item("CEM-001", "Cement 40kg", "bag", 50.0)).unwrap();

    // Duplicate codes are rejected (case-insensitive).
    assert!(matches!(db.create_item(&item("cem-001", "Other", "bag", 0.0)), Err(Error::Validation(_))));

    db.record_movement(&mv(&cement.id, MovementKind::In, 200.0, "2026-10-01")).unwrap();
    db.record_movement(&mv(&cement.id, MovementKind::Out, 45.0, "2026-10-05")).unwrap();
    db.record_movement(&mv(&cement.id, MovementKind::Out, 0.5, "2026-10-06")).unwrap();
    assert_eq!(db.item_balance(&cement.id).unwrap(), 154.5);

    // Cannot issue more than the balance.
    let err = db.record_movement(&mv(&cement.id, MovementKind::Out, 1000.0, "2026-10-07")).unwrap_err();
    assert!(err.to_string().contains("Not enough stock"), "{err}");

    // Zero / negative quantity and bad dates are rejected.
    assert!(db.record_movement(&mv(&cement.id, MovementKind::In, 0.0, "2026-10-07")).is_err());
    assert!(db.record_movement(&mv(&cement.id, MovementKind::In, 1.0, "2026-13-45")).is_err());

    // Inventory list
    let list = db.stock_list("", false).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].total_in, 200.0);
    assert_eq!(list[0].total_out, 45.5);
    assert!(!list[0].low);

    // Cannot delete an item that still has stock.
    assert!(db.delete_item(&cement.id).is_err());
}

#[test]
fn void_keeps_record_but_fixes_balance() {
    let db = Db::open_in_memory().unwrap();
    let it = db.create_item(&item("A", "Item A", "pc", 5.0)).unwrap();
    db.record_movement(&mv(&it.id, MovementKind::In, 10.0, "2026-10-01")).unwrap();
    let out = db.record_movement(&mv(&it.id, MovementKind::Out, 8.0, "2026-10-02")).unwrap();
    assert!(db.stock_list("", true).unwrap().len() == 1, "2 left <= reorder 5 -> low");

    let voided = db.void_movement(&out.id, "wrong item").unwrap();
    assert!(voided.voided);
    assert!(voided.remarks.contains("VOID: wrong item"));
    assert_eq!(db.item_balance(&it.id).unwrap(), 10.0);
    assert!(db.void_movement(&out.id, "again").is_err());

    // Voided records are hidden by default, visible on request.
    let f = MovementFilter::default();
    assert_eq!(db.list_movements(&f).unwrap().len(), 1);
    let f = MovementFilter { include_voided: true, ..Default::default() };
    assert_eq!(db.list_movements(&f).unwrap().len(), 2);
}

#[test]
fn summary_and_stock_card() {
    let db = Db::open_in_memory().unwrap();
    let it = db.create_item(&item("A", "Item A", "pc", 0.0)).unwrap();
    db.record_movement(&mv(&it.id, MovementKind::In, 100.0, "2026-09-15")).unwrap();
    db.record_movement(&mv(&it.id, MovementKind::Out, 30.0, "2026-09-20")).unwrap();
    db.record_movement(&mv(&it.id, MovementKind::In, 50.0, "2026-10-03")).unwrap();
    db.record_movement(&mv(&it.id, MovementKind::Out, 20.0, "2026-10-10")).unwrap();
    db.record_movement(&mv(&it.id, MovementKind::Out, 5.0, "2026-11-02")).unwrap();

    let s = db.inout_summary(Some("2026-10-01"), Some("2026-10-31")).unwrap();
    assert_eq!(s.len(), 1);
    assert_eq!((s[0].beginning, s[0].total_in, s[0].total_out, s[0].ending), (70.0, 50.0, 20.0, 100.0));

    let card = db.stock_card(&it.id, Some("2026-10-01"), Some("2026-10-31")).unwrap();
    assert_eq!(card.beginning, 70.0);
    assert_eq!(card.lines.len(), 2);
    assert_eq!(card.lines[0].balance, 120.0);
    assert_eq!(card.ending, 100.0);

    let d = db.dashboard("2026-10-10").unwrap();
    assert_eq!((d.item_count, d.today_out, d.today_out_count), (1, 20.0, 1));
}

#[test]
fn excel_template_round_trip_and_reports() {
    let db = Db::open_in_memory().unwrap();

    // Items template contains 2 sample items with beginning balances.
    let bytes = excel::template(ImportKind::Items).unwrap();
    let parsed = excel::parse(&db, &bytes, "items.xlsx").unwrap();
    assert_eq!(parsed.preview.kind, ImportKind::Items);
    assert!(parsed.preview.errors.is_empty(), "{:?}", parsed.preview.errors);
    assert_eq!(parsed.preview.new_items, 2);
    let res = db.commit_import(&parsed, "Tester").unwrap();
    assert_eq!((res.items_created, res.movements_created), (2, 2));
    let cement = db.find_item_by_code("CEM-001").unwrap().unwrap();
    assert_eq!(db.item_balance(&cement.id).unwrap(), 120.0);

    // Movements template: IN 200 then OUT 45 for CEM-001.
    let bytes = excel::template(ImportKind::Movements).unwrap();
    let parsed = excel::parse(&db, &bytes, "moves.xlsx").unwrap();
    assert_eq!(parsed.preview.kind, ImportKind::Movements);
    assert!(parsed.preview.errors.is_empty(), "{:?}", parsed.preview.errors);
    db.commit_import(&parsed, "Tester").unwrap();
    assert_eq!(db.item_balance(&cement.id).unwrap(), 275.0);

    // Re-importing the item list updates instead of duplicating.
    let bytes = excel::template(ImportKind::Items).unwrap();
    let parsed = excel::parse(&db, &bytes, "items.xlsx").unwrap();
    assert_eq!((parsed.preview.new_items, parsed.preview.updated_items), (0, 2));
    assert_eq!(parsed.preview.warnings.len(), 2);

    // Exports produce valid files.
    let rows = db.stock_list("", false).unwrap();
    let x = excel::export_inventory("Test WH", &rows, "2026-11-01").unwrap();
    assert_eq!(&x[..2], b"PK");
    let back = excel::parse(&db, &x, "export.xlsx").unwrap();
    assert_eq!(back.preview.valid_rows, 2, "exported inventory can be re-imported as an item list");

    let meta = pdf::ReportMeta { org: "Test WH".into(), generated: "now".into(), prepared_by: "Me".into() };
    let p = pdf::inventory_report(&meta, &rows, "2026-11-01");
    assert!(p.starts_with(b"%PDF"));
    let card = db.stock_card(&cement.id, None, None).unwrap();
    assert!(pdf::stock_card_report(&meta, &card).starts_with(b"%PDF"));
    assert_eq!(&excel::export_stock_card("Test WH", &card).unwrap()[..2], b"PK");
}

#[test]
fn file_database_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data").join("inventory.db");
    let id = {
        let db = Db::open(&path).unwrap();
        db.create_item(&item("X", "Persisted", "pc", 0.0)).unwrap().id
    };
    let db = Db::open(&path).unwrap();
    assert_eq!(db.get_item(&id).unwrap().name, "Persisted");
}
