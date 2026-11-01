use inventory_core::types::*;
use inventory_core::Db;

fn item(code: &str) -> ItemInput {
    ItemInput { code: code.into(), name: format!("Item {code}"), unit: "pc".into(), ..Default::default() }
}

fn mv(item_id: &str, kind: MovementKind, qty: f64) -> MovementInput {
    MovementInput {
        item_id: item_id.into(),
        kind,
        qty,
        date: "2026-10-01".into(),
        reference: String::new(),
        party: String::new(),
        remarks: String::new(),
        recorded_by: String::new(),
    }
}

/// Moves everything `from` has after `cursor` into `to`; returns new cursor.
fn exchange(from: &Db, to: &Db, cursor: i64) -> i64 {
    let mut c = cursor;
    loop {
        let (changes, next, more) = from.changes_since(c, 2, None).unwrap(); // tiny pages on purpose
        to.apply_changes(&changes).unwrap();
        c = next;
        if !more {
            return c;
        }
    }
}

#[test]
fn offline_edits_on_both_sides_merge() {
    let pc = Db::open_in_memory().unwrap();
    let phone = Db::open_in_memory().unwrap();

    let a = pc.create_item(&item("A")).unwrap();
    pc.record_movement(&mv(&a.id, MovementKind::In, 100.0)).unwrap();
    let (mut pc_cur, mut ph_cur) = (0, 0);
    pc_cur = exchange(&pc, &phone, pc_cur);
    assert_eq!(phone.item_balance(&a.id).unwrap(), 100.0);

    // Both work offline on the same item.
    pc.record_movement(&mv(&a.id, MovementKind::In, 50.0)).unwrap();
    phone.record_movement(&mv(&a.id, MovementKind::Out, 30.0)).unwrap();
    let b = phone.create_item(&item("B")).unwrap();
    phone.record_movement(&mv(&b.id, MovementKind::In, 7.0)).unwrap();

    ph_cur = exchange(&phone, &pc, ph_cur);
    pc_cur = exchange(&pc, &phone, pc_cur);
    let _ = (ph_cur, pc_cur);

    for db in [&pc, &phone] {
        assert_eq!(db.item_balance(&a.id).unwrap(), 120.0, "nothing lost: 100 + 50 - 30");
        assert_eq!(db.item_balance(&b.id).unwrap(), 7.0);
        assert_eq!(db.stock_list("", false).unwrap().len(), 2);
    }
}

#[test]
fn latest_item_edit_wins_and_void_sticks() {
    let pc = Db::open_in_memory().unwrap();
    let phone = Db::open_in_memory().unwrap();
    let a = pc.create_item(&item("A")).unwrap();
    let m = pc.record_movement(&mv(&a.id, MovementKind::In, 10.0)).unwrap();
    exchange(&pc, &phone, 0);

    let mut edit = item("A");
    edit.name = "Edited on PC".into();
    pc.update_item(&a.id, &edit).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    edit.name = "Edited on phone (later)".into();
    phone.update_item(&a.id, &edit).unwrap();
    phone.void_movement(&m.id, "duplicate").unwrap();

    exchange(&pc, &phone, 0);
    exchange(&phone, &pc, 0);
    for db in [&pc, &phone] {
        assert_eq!(db.get_item(&a.id).unwrap().name, "Edited on phone (later)");
        assert_eq!(db.item_balance(&a.id).unwrap(), 0.0);
    }

    // Re-applying the same changes is harmless.
    let (changes, _, _) = pc.changes_since(0, 500, None).unwrap();
    assert_eq!(phone.apply_changes(&changes).unwrap(), 0);
}

#[cfg(all(feature = "server", feature = "client"))]
#[tokio::test(flavor = "multi_thread")]
async fn pair_and_sync_over_http() {
    use inventory_core::sync::{client, server};
    use std::sync::Arc;

    let pc = Arc::new(Db::open_in_memory().unwrap());
    let phone = Arc::new(Db::open_in_memory().unwrap());
    let port = 48_100 + (std::process::id() % 200) as u16;
    let hub = server::Hub::new(pc.clone(), port);
    tokio::spawn(server::serve(hub.clone()));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let a = pc.create_item(&item("A")).unwrap();
    pc.record_movement(&mv(&a.id, MovementKind::In, 10.0)).unwrap();

    // Pair using the QR payload, pointed at localhost for the test.
    let offer = hub.start_pairing().unwrap();
    let mut qr: serde_json::Value = serde_json::from_str(&offer.payload).unwrap();
    qr["host"] = "127.0.0.1".into();
    let peer = client::pair(phone.clone(), &qr.to_string()).await.unwrap();
    assert_eq!(peer.peer_id, pc.device_id().unwrap());

    phone.record_movement(&mv(&a.id, MovementKind::Out, 1.0)).unwrap_err(); // phone has no stock yet
    let r = client::sync_with_hub(phone.clone()).await.unwrap();
    assert!(r.pulled >= 2);
    phone.record_movement(&mv(&a.id, MovementKind::Out, 4.0)).unwrap();
    client::sync_with_hub(phone.clone()).await.unwrap();
    assert_eq!(pc.item_balance(&a.id).unwrap(), 6.0);

    // The pairing code is single-use.
    assert!(client::pair(phone.clone(), &qr.to_string()).await.is_err());
}
