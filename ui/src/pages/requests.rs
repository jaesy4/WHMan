use inventory_types::{IssueInput, IssueLineInput, ItemRequest, RequestFilter, RequestStatus};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::util::{fq, parse_qty, today};
use crate::{api, ctx, Page};

fn status_class(s: RequestStatus) -> &'static str {
    match s {
        RequestStatus::Submitted => "badge warn",
        RequestStatus::Approved => "badge info",
        RequestStatus::Issued => "badge in",
        RequestStatus::Received => "badge",
        RequestStatus::Rejected | RequestStatus::Cancelled => "badge out",
    }
}

/// Requests sent by departments from the web portal.
#[component]
pub fn RequestsPage() -> impl IntoView {
    let ctx = ctx();
    let rows = RwSignal::new(Vec::<ItemRequest>::new());
    // "open" | "SUBMITTED" | "APPROVED" | "ISSUED" | "all"
    let view_mode = RwSignal::new("open".to_string());
    let dept = RwSignal::new(String::new());
    let search = RwSignal::new(String::new());
    let depts = RwSignal::new(Vec::new());
    let issuing = RwSignal::new(None::<ItemRequest>);
    let deciding = RwSignal::new(None::<(ItemRequest, bool)>); // (request, approve?)
    let tick = RwSignal::new(0u32);

    spawn_local(async move {
        if let Ok(d) = api::list_departments().await {
            depts.set(d);
        }
    });

    Effect::new(move |_| {
        let _ = ctx.refresh.get();
        let _ = tick.get();
        let mode = view_mode.get();
        let filter = RequestFilter {
            status: RequestStatus::parse(&mode),
            open_only: mode == "open",
            department_id: Some(dept.get()).filter(|s| !s.is_empty()),
            search: Some(search.get()).filter(|s| !s.trim().is_empty()),
            limit: Some(200),
        };
        spawn_local(async move {
            match api::list_requests(&filter).await {
                Ok(v) => rows.set(v),
                Err(e) => ctx.err(e),
            }
        });
    });

    // Check for new requests every 30 seconds while this screen is open.
    let handle = set_interval_with_handle(move || tick.update(|n| *n = n.wrapping_add(1)), std::time::Duration::from_secs(30)).ok();
    on_cleanup(move || {
        if let Some(h) = handle {
            h.clear();
        }
    });

    let chip = move |id: &'static str, label: &'static str| {
        view! {
            <button class="chip" class:active=move || view_mode.get() == id on:click=move |_| view_mode.set(id.to_string())>
                {label}
            </button>
        }
    };

    view! {
        <div class="stack">
            <div class="panel">
                <div class="toolbar">
                    <div class="chips">
                        {chip("open", "Open")}
                        {chip("SUBMITTED", "Waiting approval")}
                        {chip("APPROVED", "To issue")}
                        {chip("ISSUED", "Issued")}
                        {chip("all", "All")}
                    </div>
                    <select class="input" prop:value=move || dept.get() on:change=move |ev| dept.set(event_target_value(&ev))>
                        <option value="">"All departments"</option>
                        {move || depts.get().into_iter().map(|d| view! { <option value={d.id}>{d.name}</option> }).collect_view()}
                    </select>
                    <input class="input grow" placeholder="Search request no., name, purpose or item…"
                        prop:value=move || search.get() on:input=move |ev| search.set(event_target_value(&ev))/>
                    <button class="btn ghost" on:click=move |_| ctx.go(Page::Portal)>"Portal & users"</button>
                </div>
            </div>

            {move || {
                let list = rows.get();
                if list.is_empty() {
                    return view! { <div class="panel empty">"No requests for this filter."</div> }.into_any();
                }
                list.into_iter().map(|r| {
                    let for_issue = r.clone();
                    let for_ok = r.clone();
                    let for_no = r.clone();
                    let status = r.status;
                    view! {
                        <section class="panel request-card">
                            <div class="panel-head">
                                <div>
                                    <h2>{r.number.clone()}" · "{r.department_name.clone()}</h2>
                                    <div class="muted small">
                                        {format!("{} · {}", r.requested_by, r.created_at)}
                                        {(!r.needed_by.is_empty()).then(|| format!(" · needed by {}", r.needed_by))}
                                    </div>
                                </div>
                                <span class={status_class(status)}>{status.label()}</span>
                            </div>
                            <p class="small">{r.purpose.clone()}</p>
                            <div class="table-wrap">
                                <table class="table">
                                    <thead><tr>
                                        <th>"Item"</th>
                                        <th class="num">"Requested"</th>
                                        <th class="num">"In stock"</th>
                                        <th class="num">"Issued"</th>
                                    </tr></thead>
                                    <tbody>
                                        {r.lines.iter().map(|l| {
                                            let short = status == RequestStatus::Approved && l.balance < l.qty_requested;
                                            view! {
                                                <tr class:low=short>
                                                    <td>
                                                        <span class="code">{l.item_code.clone()}</span>" "{l.item_name.clone()}
                                                        {(!l.remarks.is_empty()).then(|| view! { <div class="muted small">{l.remarks.clone()}</div> })}
                                                    </td>
                                                    <td class="num strong">{fq(l.qty_requested)}" "<span class="muted">{l.unit.clone()}</span></td>
                                                    <td class="num">{fq(l.balance)}</td>
                                                    <td class="num">{if matches!(status, RequestStatus::Issued | RequestStatus::Received) { fq(l.qty_issued) } else { "—".to_string() }}</td>
                                                </tr>
                                            }
                                        }).collect_view()}
                                    </tbody>
                                </table>
                            </div>
                            {(!r.approved_by.is_empty()).then(|| view! {
                                <div class="muted small">
                                    {format!("{} by {} · {}",
                                        if status == RequestStatus::Rejected { "Rejected" } else { "Approved" },
                                        r.approved_by, r.approved_at)}
                                </div>
                            })}
                            {(!r.issued_by.is_empty()).then(|| view! {
                                <div class="muted small">{format!("Issued by {} · {}", r.issued_by, r.issued_at)}</div>
                            })}
                            {(!r.received_by.is_empty()).then(|| view! {
                                <div class="muted small">{format!("Received by {} · {}", r.received_by, r.received_at)}</div>
                            })}
                            {(!r.decision_note.is_empty()).then(|| view! { <div class="note small">{r.decision_note.clone()}</div> })}
                            <div class="btn-row">
                                {(status == RequestStatus::Submitted).then(|| view! {
                                    <button class="btn primary" on:click=move |_| deciding.set(Some((for_ok.clone(), true)))>"✓ Approve"</button>
                                })}
                                {(status == RequestStatus::Approved).then(|| view! {
                                    <button class="btn out" on:click=move |_| issuing.set(Some(for_issue.clone()))>"↑ Issue items"</button>
                                })}
                                {status.is_open().then(|| view! {
                                    <button class="btn danger" on:click=move |_| deciding.set(Some((for_no.clone(), false)))>"Reject"</button>
                                })}
                            </div>
                        </section>
                    }
                }).collect_view().into_any()
            }}

            {move || issuing.get().map(|r| view! { <IssueDialog request=r close=issuing/> })}
            {move || deciding.get().map(|(r, approve)| view! { <DecideDialog request={r} approve={approve} close={deciding}/> })}
        </div>
    }
}

#[component]
fn DecideDialog(request: ItemRequest, approve: bool, close: RwSignal<Option<(ItemRequest, bool)>>) -> impl IntoView {
    let ctx = ctx();
    let note = RwSignal::new(String::new());
    let id = request.id.clone();
    let confirm = move |_| {
        let id = id.clone();
        let text = note.get_untracked();
        if !approve && text.trim().is_empty() {
            return ctx.err("Please give a reason for rejecting.");
        }
        spawn_local(async move {
            let res = if approve { api::approve_request(&id, &text).await } else { api::reject_request(&id, &text).await };
            match res {
                Ok(r) => {
                    ctx.ok(format!("{}: {}", r.number, r.status.label()));
                    ctx.changed();
                    close.set(None);
                }
                Err(e) => ctx.err(e),
            }
        });
    };
    view! {
        <div class="modal-back" on:click=move |_| close.set(None)>
            <div class="modal" on:click=|ev| ev.stop_propagation()>
                <div class="modal-head">
                    <h2>{format!("{} {}", if approve { "Approve" } else { "Reject" }, request.number)}</h2>
                    <button class="icon-btn" on:click=move |_| close.set(None)>"✕"</button>
                </div>
                <p class="muted small">
                    {if approve {
                        "Normally the department head approves. Approve here only if the head asked you to."
                    } else {
                        "The department will see the reason in the web portal."
                    }}
                </p>
                <label class="field">
                    <span>{if approve { "Note (optional)" } else { "Reason *" }}</span>
                    <input class="input" prop:value=move || note.get() on:input=move |ev| note.set(event_target_value(&ev))/>
                </label>
                <div class="modal-foot">
                    <div class="grow"></div>
                    <button class="btn ghost" on:click=move |_| close.set(None)>"Cancel"</button>
                    <button class={if approve { "btn primary" } else { "btn danger" }} on:click=confirm>
                        {if approve { "Approve" } else { "Reject" }}
                    </button>
                </div>
            </div>
        </div>
    }
}

/// Releases an approved request: creates the Stock Out records.
#[component]
fn IssueDialog(request: ItemRequest, close: RwSignal<Option<ItemRequest>>) -> impl IntoView {
    let ctx = ctx();
    // Pre-fill with what can actually be given: min(requested, in stock).
    let qtys: Vec<(String, RwSignal<String>)> = request
        .lines
        .iter()
        .map(|l| (l.id.clone(), RwSignal::new(fq(l.qty_requested.min(l.balance.max(0.0))).replace(',', ""))))
        .collect();
    let date = RwSignal::new(today());
    let issued_by = RwSignal::new(ctx.settings.get_untracked().recorded_by);
    let remarks = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let req_id = request.id.clone();
    let qtys_for_save = qtys.clone();

    let save = move |_| {
        let mut lines = Vec::new();
        for (line_id, sig) in &qtys_for_save {
            let text = sig.get_untracked();
            let qty = if text.trim().is_empty() { Some(0.0) } else { parse_qty(&text) };
            let Some(qty) = qty.filter(|q| *q >= 0.0) else {
                return ctx.err("Quantities must be numbers (0 = don't issue that item).");
            };
            lines.push(IssueLineInput { line_id: line_id.clone(), qty });
        }
        let input = IssueInput {
            request_id: req_id.clone(),
            lines,
            date: date.get_untracked(),
            issued_by: issued_by.get_untracked(),
            remarks: remarks.get_untracked(),
        };
        busy.set(true);
        spawn_local(async move {
            let res = api::issue_request(&input).await;
            busy.set(false);
            match res {
                Ok(r) => {
                    ctx.ok(format!("{} issued. Stock Out records were created (reference {}).", r.number, r.number));
                    ctx.changed();
                    close.set(None);
                }
                Err(e) => ctx.err(e),
            }
        });
    };

    view! {
        <div class="modal-back" on:click=move |_| close.set(None)>
            <div class="modal wide" on:click=|ev| ev.stop_propagation()>
                <div class="modal-head">
                    <h2>{format!("Issue {} · {}", request.number, request.department_name)}</h2>
                    <button class="icon-btn" on:click=move |_| close.set(None)>"✕"</button>
                </div>
                <p class="muted small">
                    "Enter what you are actually releasing. You can give less than requested; 0 means that item is not issued. "
                    "Each item becomes a Stock Out with reference " <b>{request.number.clone()}</b> "."
                </p>
                <div class="table-wrap">
                    <table class="table">
                        <thead><tr>
                            <th>"Item"</th>
                            <th class="num">"Requested"</th>
                            <th class="num">"In stock"</th>
                            <th class="num">"Issue now"</th>
                        </tr></thead>
                        <tbody>
                            {request.lines.iter().zip(qtys.iter()).map(|(l, (_, sig))| {
                                let sig = *sig;
                                view! {
                                    <tr class:low={l.balance < l.qty_requested}>
                                        <td><span class="code">{l.item_code.clone()}</span>" "{l.item_name.clone()}</td>
                                        <td class="num">{fq(l.qty_requested)}" "{l.unit.clone()}</td>
                                        <td class="num">{fq(l.balance)}</td>
                                        <td class="num">
                                            <input class="input qty-cell" inputmode="decimal"
                                                prop:value=move || sig.get() on:input=move |ev| sig.set(event_target_value(&ev))/>
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
                <div class="grid2">
                    <label class="field">
                        <span>"Date *"</span>
                        <input class="input" type="date" prop:value=move || date.get() on:input=move |ev| date.set(event_target_value(&ev))/>
                    </label>
                    <label class="field">
                        <span>"Issued By"</span>
                        <input class="input" prop:value=move || issued_by.get() on:input=move |ev| issued_by.set(event_target_value(&ev))/>
                    </label>
                </div>
                <label class="field">
                    <span>"Remarks"</span>
                    <input class="input" placeholder="Optional" prop:value=move || remarks.get() on:input=move |ev| remarks.set(event_target_value(&ev))/>
                </label>
                <div class="modal-foot">
                    <div class="grow"></div>
                    <button class="btn ghost" on:click=move |_| close.set(None)>"Cancel"</button>
                    <button class="btn out" disabled=move || busy.get() on:click=save>
                        {move || if busy.get() { "Saving…" } else { "Issue & record Stock Out" }}
                    </button>
                </div>
            </div>
        </div>
    }
}
