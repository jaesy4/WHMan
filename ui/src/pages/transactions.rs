use inventory_types::{FileFormat, Movement, MovementFilter, MovementKind, ReportKind};
use leptos::prelude::*;
use leptos::task::spawn_local;

use super::{export, request, DateRange, ExportButtons};
use crate::util::{fq, month_start, opt, today};
use crate::{api, ctx};

#[component]
pub fn TransactionsPage() -> impl IntoView {
    let ctx = ctx();
    let from = RwSignal::new(month_start());
    let to = RwSignal::new(today());
    let kind = RwSignal::new(String::new()); // "", "IN", "OUT"
    let search = RwSignal::new(String::new());
    let show_void = RwSignal::new(false);
    let rows = RwSignal::new(Vec::<Movement>::new());
    let voiding = RwSignal::new(None::<Movement>);

    let kind_val = move || match kind.get().as_str() {
        "IN" => Some(MovementKind::In),
        "OUT" => Some(MovementKind::Out),
        _ => None,
    };

    Effect::new(move |_| {
        let _ = ctx.refresh.get();
        let f = MovementFilter {
            from: opt(from.get()),
            to: opt(to.get()),
            kind: kind_val(),
            item_id: None,
            search: opt(search.get()),
            include_voided: show_void.get(),
            limit: Some(1000),
        };
        spawn_local(async move {
            match api::list_movements(&f).await {
                Ok(v) => rows.set(v),
                Err(e) => ctx.err(e),
            }
        });
    });

    let make = move |fmt: FileFormat| {
        let mut r = request(ReportKind::Movements, fmt);
        r.from = opt(from.get_untracked());
        r.to = opt(to.get_untracked());
        r.kind = match kind.get_untracked().as_str() {
            "IN" => Some(MovementKind::In),
            "OUT" => Some(MovementKind::Out),
            _ => None,
        };
        r
    };

    view! {
        <div class="stack">
            <div class="panel filters">
                <DateRange from=from to=to/>
                <div class="toolbar">
                    <select class="input" prop:value=move || kind.get()
                        on:change=move |ev| kind.set(event_target_value(&ev))>
                        <option value="">"All (IN and OUT)"</option>
                        <option value="IN">"Stock In only"</option>
                        <option value="OUT">"Stock Out only"</option>
                    </select>
                    <input class="input grow" placeholder="Search item, reference, received from / issued to…"
                        prop:value=move || search.get()
                        on:input=move |ev| search.set(event_target_value(&ev))/>
                    <label class="toggle">
                        <input type="checkbox" prop:checked=move || show_void.get()
                            on:change=move |ev| show_void.set(event_target_checked(&ev))/>
                        <span>"Show void"</span>
                    </label>
                    <ExportButtons make=make/>
                </div>
            </div>

            <div class="panel flush">
                <table class="table">
                    <thead>
                        <tr>
                            <th>"Date"</th>
                            <th>"Type"</th>
                            <th>"Item"</th>
                            <th class="num">"Qty"</th>
                            <th class="hide-sm">"Reference"</th>
                            <th class="hide-sm">"Received From / Issued To"</th>
                            <th class="hide-sm">"Recorded By"</th>
                            <th class="actions"></th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || rows.get().into_iter().map(|m| {
                            let slip_id = m.id.clone();
                            let for_void = m.clone();
                            let voided = m.voided;
                            view! {
                                <tr class:void=voided>
                                    <td>{m.date.clone()}</td>
                                    <td>
                                        <span class={if voided { "badge" } else if m.kind == MovementKind::In { "badge in" } else { "badge out" }}>
                                            {if voided { "VOID" } else { m.kind.as_str() }}
                                        </span>
                                    </td>
                                    <td>
                                        <div><span class="code">{m.item_code.clone()}</span>" "{m.item_name.clone()}</div>
                                        <div class="muted small show-sm">{format!("{} · {}", m.reference, m.party)}</div>
                                        {(!m.remarks.is_empty()).then(|| view! { <div class="muted small">{m.remarks.clone()}</div> })}
                                    </td>
                                    <td class="num strong">{fq(m.qty)}" "<span class="muted">{m.unit.clone()}</span></td>
                                    <td class="hide-sm">{m.reference.clone()}</td>
                                    <td class="hide-sm">{m.party.clone()}</td>
                                    <td class="hide-sm">{m.recorded_by.clone()}</td>
                                    <td class="actions">
                                        <button class="mini" title="Print slip" on:click=move |_| {
                                            let mut r = request(ReportKind::Slip, FileFormat::Pdf);
                                            r.movement_id = Some(slip_id.clone());
                                            export(ctx, r);
                                        }>"Slip"</button>
                                        {(!voided).then(|| view! {
                                            <button class="mini danger" title="Cancel this record"
                                                on:click=move |_| voiding.set(Some(for_void.clone()))>"Void"</button>
                                        })}
                                    </td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
                {move || rows.get().is_empty().then(|| view! { <div class="empty">"No records for this filter."</div> })}
            </div>

            {move || voiding.get().map(|m| view! { <VoidDialog movement=m close=voiding/> })}
        </div>
    }
}

#[component]
fn VoidDialog(movement: Movement, close: RwSignal<Option<Movement>>) -> impl IntoView {
    let ctx = ctx();
    let reason = RwSignal::new(String::new());
    let id = movement.id.clone();
    let confirm = move |_| {
        let id = id.clone();
        let why = reason.get_untracked();
        if why.trim().is_empty() {
            return ctx.err("Please give a reason for voiding.");
        }
        spawn_local(async move {
            match api::void_movement(&id, &why).await {
                Ok(_) => {
                    ctx.ok("Record voided");
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
                    <h2>"Void record"</h2>
                    <button class="icon-btn" on:click=move |_| close.set(None)>"✕"</button>
                </div>
                <p>
                    {format!("{} {} {} of {} ({}) on {}",
                        movement.kind.as_str(), fq(movement.qty), movement.unit,
                        movement.item_name, movement.item_code, movement.date)}
                </p>
                <p class="muted small">
                    "The record stays in the history marked VOID and no longer counts in the balance. This cannot be undone — record a new entry if needed."
                </p>
                <label class="field">
                    <span>"Reason *"</span>
                    <input class="input" placeholder="e.g. wrong quantity, duplicate entry"
                        prop:value=move || reason.get()
                        on:input=move |ev| reason.set(event_target_value(&ev))/>
                </label>
                <div class="modal-foot">
                    <div class="grow"></div>
                    <button class="btn ghost" on:click=move |_| close.set(None)>"Cancel"</button>
                    <button class="btn danger" on:click=confirm>"Void record"</button>
                </div>
            </div>
        </div>
    }
}
