use inventory_core::types::*;
use inventory_core::{Actor, Db, Error};

fn item(code: &str, name: &str) -> ItemInput {
    ItemInput {
        code: code.into(),
        name: name.into(),
        unit: "pc".into(),
        location: String::new(),
        reorder_level: 0.0,
        remarks: String::new(),
    }
}

fn stock_in(db: &Db, item_id: &str, qty: f64) {
    db.record_movement(&MovementInput {
        item_id: item_id.into(),
        kind: MovementKind::In,
        qty,
        date: "2026-11-01".into(),
        reference: "DR-1".into(),
        party: "Supplier".into(),
        remarks: String::new(),
        recorded_by: "Tester".into(),
    })
    .unwrap();
}

fn user(db: &Db, username: &str, dept: &str, role: UserRole) -> WebUser {
    db.save_web_user(
        None,
        &WebUserInput {
            username: username.into(),
            full_name: format!("{username} name"),
            department_id: dept.into(),
            role,
            password: "secret123".into(),
            active: true,
        },
    )
    .unwrap()
}

fn line(item_id: &str, qty: f64) -> RequestLineInput {
    RequestLineInput { item_id: item_id.into(), qty, remarks: String::new() }
}

#[test]
fn departments_are_seeded() {
    let db = Db::open_in_memory().unwrap();
    let names: Vec<String> = db.list_departments().unwrap().into_iter().map(|d| d.name).collect();
    assert_eq!(names, vec!["Engineering".to_string(), "Solar Team".to_string()]);
}

#[test]
fn login_sessions_and_accounts() {
    let db = Db::open_in_memory().unwrap();
    let u = user(&db, "juan.eng", "engineering", UserRole::Requester);

    // Usernames are unique (case-insensitive) and validated.
    let dup = WebUserInput {
        username: "JUAN.ENG".into(),
        full_name: "Other".into(),
        department_id: "solar".into(),
        role: UserRole::Requester,
        password: "secret123".into(),
        active: true,
    };
    assert!(matches!(db.save_web_user(None, &dup), Err(Error::Validation(_))));
    let bad = WebUserInput { username: "has space".into(), ..dup.clone() };
    assert!(db.save_web_user(None, &bad).is_err());
    let short_pw = WebUserInput { username: "ana".into(), password: "123".into(), ..dup.clone() };
    assert!(db.save_web_user(None, &short_pw).is_err());

    assert!(db.web_login("juan.eng", "wrong").is_err());
    assert!(db.web_login("nobody", "secret123").is_err());
    let (token, logged) = db.web_login("Juan.Eng", "secret123").unwrap();
    assert_eq!(logged.id, u.id);
    assert_eq!(db.web_session_user(&token).unwrap().unwrap().department_name, "Engineering");
    assert!(db.web_session_user("not-a-token").unwrap().is_none());

    // Changing the password logs out everywhere; empty password on edit keeps it.
    let edit = WebUserInput {
        username: "juan.eng".into(),
        full_name: "Juan Dela Cruz".into(),
        department_id: "engineering".into(),
        role: UserRole::Requester,
        password: String::new(),
        active: true,
    };
    db.save_web_user(Some(&u.id), &edit).unwrap();
    assert!(db.web_session_user(&token).unwrap().is_some());
    db.save_web_user(Some(&u.id), &WebUserInput { password: "newpass1".into(), ..edit.clone() }).unwrap();
    assert!(db.web_session_user(&token).unwrap().is_none());
    assert!(db.web_login("juan.eng", "secret123").is_err());
    let (token, _) = db.web_login("juan.eng", "newpass1").unwrap();

    // Self-service password change.
    assert!(db.web_change_password(&u.id, "wrong", "another1").is_err());
    db.web_change_password(&u.id, "newpass1", "another1").unwrap();
    assert!(db.web_login("juan.eng", "another1").is_ok());

    // Disabled accounts cannot log in and lose their sessions.
    db.save_web_user(Some(&u.id), &WebUserInput { active: false, ..edit.clone() }).unwrap();
    assert!(db.web_session_user(&token).unwrap().is_none());
    assert!(db.web_login("juan.eng", "another1").is_err());

    db.web_logout(&token).unwrap();
    db.delete_web_user(&u.id).unwrap();
    assert!(db.list_web_users().unwrap().is_empty());
}

#[test]
fn request_approve_issue_receive() {
    let db = Db::open_in_memory().unwrap();
    let panel = db.create_item(&item("SP-400", "Solar panel 400W")).unwrap();
    let wire = db.create_item(&item("W-6", "PV wire 6mm")).unwrap();
    stock_in(&db, &panel.id, 10.0);
    stock_in(&db, &wire.id, 100.0);

    let tech = user(&db, "tech", "solar", UserRole::Requester);
    let head = user(&db, "boss", "solar", UserRole::Head);
    let eng = user(&db, "eng", "engineering", UserRole::Requester);
    let eng_head = user(&db, "enghead", "engineering", UserRole::Head);

    // Validation
    let empty = RequestInput { purpose: "Site A".into(), needed_by: String::new(), lines: vec![] };
    assert!(db.create_request(&tech, &empty).is_err());
    let dup = RequestInput { lines: vec![line(&panel.id, 1.0), line(&panel.id, 2.0)], ..empty.clone() };
    assert!(db.create_request(&tech, &dup).is_err());
    let zero = RequestInput { lines: vec![line(&panel.id, 0.0)], ..empty.clone() };
    assert!(db.create_request(&tech, &zero).is_err());
    let no_purpose = RequestInput { purpose: " ".into(), lines: vec![line(&panel.id, 1.0)], ..empty.clone() };
    assert!(db.create_request(&tech, &no_purpose).is_err());

    let input = RequestInput {
        purpose: "Rooftop install, JO-1123".into(),
        needed_by: "2026-11-20".into(),
        lines: vec![line(&panel.id, 12.0), line(&wire.id, 30.0)],
    };
    let r = db.create_request(&tech, &input).unwrap();
    assert_eq!(r.status, RequestStatus::Submitted);
    assert!(r.number.starts_with("RQ-") && r.number.ends_with("-0001"), "{}", r.number);
    assert_eq!(r.lines.len(), 2);
    assert_eq!(r.lines[0].balance, 10.0);

    // Cannot issue before approval.
    let issue = |qty_panel: f64, qty_wire: f64| IssueInput {
        request_id: r.id.clone(),
        lines: vec![
            IssueLineInput { line_id: r.lines[0].id.clone(), qty: qty_panel },
            IssueLineInput { line_id: r.lines[1].id.clone(), qty: qty_wire },
        ],
        date: "2026-11-05".into(),
        issued_by: "Warehouse Man".into(),
        remarks: String::new(),
    };
    assert!(db.issue_request(&issue(10.0, 30.0)).is_err());

    // A requester can't approve; another department's head can't even see it.
    assert!(db.approve_request(&r.id, Actor::Web(&tech), "").is_err());
    assert!(matches!(db.approve_request(&r.id, Actor::Web(&eng_head), ""), Err(Error::NotFound(_))));
    assert!(matches!(db.mark_request_received(&r.id, &eng), Err(Error::NotFound(_))));

    let r2 = db.approve_request(&r.id, Actor::Web(&head), "OK").unwrap();
    assert_eq!(r2.status, RequestStatus::Approved);
    assert_eq!(r2.approved_by, "boss name");

    // Can't issue more than requested, or more than in stock (12 requested, 10 in stock).
    assert!(db.issue_request(&issue(13.0, 30.0)).is_err());
    let err = db.issue_request(&issue(12.0, 30.0)).unwrap_err();
    assert!(err.to_string().contains("Not enough stock"), "{err}");
    // Nothing was saved by the failed attempt (all-or-nothing).
    assert_eq!(db.item_balance(&wire.id).unwrap(), 100.0);
    // All zeros is rejected.
    assert!(db.issue_request(&issue(0.0, 0.0)).is_err());

    // Partial issue: 10 of 12 panels.
    let issued = db.issue_request(&issue(10.0, 30.0)).unwrap();
    assert_eq!(issued.status, RequestStatus::Issued);
    assert_eq!(issued.lines[0].qty_issued, 10.0);
    assert_eq!(db.item_balance(&panel.id).unwrap(), 0.0);
    assert_eq!(db.item_balance(&wire.id).unwrap(), 70.0);

    // Stock Out records carry the request number.
    let outs = db
        .list_movements(&MovementFilter { kind: Some(MovementKind::Out), ..Default::default() })
        .unwrap();
    assert_eq!(outs.len(), 2);
    assert!(outs.iter().all(|m| m.reference == issued.number && m.party == "Solar Team - tech name"));

    // Can't cancel after issuing; department confirms receipt.
    assert!(db.cancel_request(&r.id, &tech, "").is_err());
    let done = db.mark_request_received(&r.id, &tech).unwrap();
    assert_eq!(done.status, RequestStatus::Received);
    assert!(db.mark_request_received(&r.id, &tech).is_err());
}

#[test]
fn reject_cancel_and_lists() {
    let db = Db::open_in_memory().unwrap();
    let bolt = db.create_item(&item("B-10", "Bolt M10")).unwrap();
    stock_in(&db, &bolt.id, 50.0);
    let a = user(&db, "alice", "engineering", UserRole::Requester);
   let b = user(&db, "bob", "engineering", UserRole::Requester);
   let head = user(&db, "head.eng", "engineering", UserRole::Head);
   let solar = user(&db, "solar1", "solar", UserRole::Requester);
    let req = |u: &WebUser| {
        db.create_request(u, &RequestInput { purpose: "Job".into(), needed_by: String::new(), lines: vec![line(&bolt.id, 5.0)] })
            .unwrap()
    };

    // A head's own request is approved right away.
    let own = req(&head);
    assert_eq!(own.status, RequestStatus::Approved);

    // Reject needs a reason; warehouse can reject too.
    let r1 = req(&a);
    assert!(db.reject_request(&r1.id, Actor::Web(&head), " ").is_err());
    let rej = db.reject_request(&r1.id, Actor::Warehouse("WH"), "Out of budget").unwrap();
    assert_eq!(rej.status, RequestStatus::Rejected);
    assert_eq!(rej.decision_note, "Out of budget");
    assert!(db.approve_request(&r1.id, Actor::Warehouse("WH"), "").is_err());

    // Only the owner or the head can cancel.
    let r2 = req(&a);
    assert!(db.cancel_request(&r2.id, &b, "").is_err());
    assert!(matches!(db.cancel_request(&r2.id, &solar, ""), Err(Error::NotFound(_))));
    assert_eq!(db.cancel_request(&r2.id, &a, "Not needed").unwrap().status, RequestStatus::Cancelled);

    let r3 = req(&solar);
    assert_eq!(db.open_request_count().unwrap(), 2); // own (approved) + r3 (submitted)

    let eng = db
        .list_requests(&RequestFilter { department_id: Some("engineering".into()), ..Default::default() })
        .unwrap();
    assert_eq!(eng.len(), 3);
    assert!(eng.iter().all(|r| r.department_name == "Engineering"));
    let open = db.list_requests(&RequestFilter { open_only: true, ..Default::default() }).unwrap();
    assert_eq!(open.len(), 2);
    let found = db.list_requests(&RequestFilter { search: Some("bolt".into()), ..Default::default() }).unwrap();
    assert_eq!(found.len(), 4);
    let sub = db
        .list_requests(&RequestFilter { status: Some(RequestStatus::Submitted), ..Default::default() })
        .unwrap();
    assert_eq!(sub.len(), 1);
    assert_eq!(sub[0].id, r3.id);
    // Numbers keep counting up.
    assert!(r3.number.ends_with("-0004"), "{}", r3.number);
}
