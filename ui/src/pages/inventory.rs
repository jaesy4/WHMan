use inventory_types::{FileFormat, ItemInput, MovementKind, ReportKind, StockRow};
use leptos::prelude::*;
use leptos::task::spawn_local;

use super::{request, ExportButtons};
use crate::util::{fq, parse_qty};
use crate::{api, ctx, Page};

/// `None` = closed, `Some(None)` = new item, `Some(Some(row))` = editing.
type Editing = Option<Option<StockRow>>;

#[component]
pub fn InventoryPage() -> impl IntoView {
    let ctx = ctx();
    let rows = RwSignal::new(Vec::<StockRow>::new());
    let search = RwSignal::new(String::new());
    let low_only = RwSignal::new(false);
    let editing = RwSignal::new(None::<Option<StockRow>>);
    let loaded = RwSignal::new(false);

    Effect::new(move |_| {
        let _ = ctx.refresh.get();
        let q = search.get();
        let low = low_only.get();
        spawn_local(async move {
            match api::stock_list(&q, low).await {
                Ok(v) => {
                    rows.set(v);
                    loaded.set(true);
                }
                Err(e) => ctx.err(e),
            }
        });
    });

    let make = move |f: FileFormat| {
        let mut r = request(ReportKind::Inventory, f);
        r.low_only = low_only.get_untracked();
        r
    };

    view! {
        <div class="stack">
            <div class="toolbar">
                <input class="input grow" placeholder="Search code, name or location…"
                    prop:value=move || search.get()
                    on:input=move |ev| search.set(event_target_value(&ev))/>
                <label class="toggle">
                    <input type="checkbox" prop:checked=move || low_only.get()
                        on:change=move |ev| low_only.set(event_target_checked(&ev))/>
                    <span>"Low stock only"</span>
                </label>
                <ExportButtons make=make/>
                <button class="btn primary" on:click=move |_| editing.set(Some(None))>"+ Add item"</button>
            </div>

            <div class="panel flush">
                <table class="table">
                    <thead>
                        <tr>
                            <th>"Item Code"</th>
                            <th>"Item Name"</th>
                            <th class="hide-sm">"Unit"</th>
                            <th class="hide-sm">"Location"</th>
                            <th class="num hide-sm">"Total In"</th>
                            <th class="num hide-sm">"Total Out"</th>
                            <th class="num">"Balance"</th>
                            <th class="actions"></th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || rows.get().into_iter().map(|r| {
                            let id_in = r.item.id.clone();
                            let id_out = r.item.id.clone();
                            let id_card = r.item.id.clone();
                            let row_for_edit = r.clone();
                            view! {
                                <tr class:low=r.low>
                                    <td class="code">{r.item.code.clone()}</td>
                                    <td>
                                        <button class="link" on:click=move |_| editing.set(Some(Some(row_for_edit.clone())))>
                                            {r.item.name.clone()}
                                        </button>
                                        <div class="muted small show-sm">{format!("{} · {}", r.item.unit, r.item.location)}</div>
                                    </td>
                                    <td class="hide-sm">{r.item.unit.clone()}</td>
                                    <td class="hide-sm">{r.item.location.clone()}</td>
                                    <td class="num hide-sm">{fq(r.total_in)}</td>
                                    <td class="num hide-sm">{fq(r.total_out)}</td>
                                    <td class="num strong">
                                        {fq(r.balance)}
                                        {r.low.then(|| view! { <span class="badge warn">"Reorder"</span> })}
                                    </td>
                                    <td class="actions">
                                        <button class="mini in" title="Stock In"
                                            on:click=move |_| ctx.open_movement(MovementKind::In, Some(id_in.clone()))>"IN"</button>
                                        <button class="mini out" title="Stock Out"
                                            on:click=move |_| ctx.open_movement(MovementKind::Out, Some(id_out.clone()))>"OUT"</button>
                                        <button class="mini" title="Stock Card"
                                            on:click=move |_| { ctx.focus_item.set(Some(id_card.clone())); ctx.go(Page::StockCard); }>"Card"</button>
                                    </td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
                {move || (loaded.get() && rows.get().is_empty()).then(|| view! {
                    <div class="empty">
                        "No items found. Add an item, or use Import Excel to load your item list."
                    </div>
                })}
            </div>

            {move || editing.get().map(|e| view! { <ItemEditor editing=e close=editing/> })}
        </div>
    }
}

#[component]
fn ItemEditor(editing: Option<StockRow>, close: RwSignal<Editing>) -> impl IntoView {
    let ctx = ctx();
    let existing = editing.clone();
    let start = editing
        .as_ref()
        .map(|r| ItemInput {
            code: r.item.code.clone(),
            name: r.item.name.clone(),
            unit: r.item.unit.clone(),
            location: r.item.location.clone(),
            reorder_level: r.item.reorder_level,
            remarks: r.item.remarks.clone(),
        })
        .unwrap_or_default();
    let code = RwSignal::new(start.code);
    let name = RwSignal::new(start.name);
    let unit = RwSignal::new(start.unit);
    let location = RwSignal::new(start.location);
    let reorder = RwSignal::new(if start.reorder_level > 0.0 { fq(start.reorder_level) } else { String::new() });
    let remarks = RwSignal::new(start.remarks);
    let busy = RwSignal::new(false);
    let id = existing.as_ref().map(|r| r.item.id.clone());
    let id_del = id.clone();
    let balance = existing.as_ref().map(|r| r.balance).unwrap_or(0.0);
    let is_new = existing.is_none();

    let save = move |_| {
        let reorder_level = match reorder.get_untracked().trim() {
            "" => 0.0,
            s => match parse_qty(s) {
                Some(v) => v,
                None => return ctx.err("Reorder level must be a number."),
            },
        };
        let input = ItemInput {
            code: code.get_untracked(),
            name: name.get_untracked(),
            unit: unit.get_untracked(),
            location: location.get_untracked(),
            reorder_level,
            remarks: remarks.get_untracked(),
        };
        let id = id.clone();
        busy.set(true);
        spawn_local(async move {
            let res = match &id {
                Some(id) => api::update_item(id, &input).await,
                None => api::create_item(&input).await,
            };
            busy.set(false);
            match res {
                Ok(item) => {
                    ctx.ok(format!("Saved {}", item.code));
                    ctx.changed();
                    close.set(None);
                }
                Err(e) => ctx.err(e),
            }
        });
    };

    let armed = RwSignal::new(false);
    let delete = move |_| {
        let Some(id) = id_del.clone() else { return };
        // Two taps: first arms the button, second deletes.
        if !armed.get_untracked() {
            armed.set(true);
            return;
        }
        spawn_local(async move {
            match api::delete_item(&id).await {
                Ok(()) => {
                    ctx.ok("Item deleted");
                    ctx.changed();
                    close.set(None);
                }
                Err(e) => ctx.err(e),
            }
        });
    };

    let field = move |label: &'static str, sig: RwSignal<String>, ph: &'static str| {
        view! {
            <label class="field">
                <span>{label}</span>
                <input class="input" placeholder=ph prop:value=move || sig.get()
                    on:input=move |ev| sig.set(event_target_value(&ev))/>
            </label>
        }
    };

    view! {
        <div class="modal-back" on:click=move |_| close.set(None)>
            <div class="modal" on:click=|ev| ev.stop_propagation()>
                <div class="modal-head">
                    <h2>{if is_new { "Add item" } else { "Edit item" }}</h2>
                    <button class="icon-btn" on:click=move |_| close.set(None)>"✕"</button>
                </div>
                <div class="grid2">
                    {field("Item Code *", code, "e.g. CEM-001")}
                    {field("Unit", unit, "pc, box, kg, bag…")}
                </div>
                {field("Item Name *", name, "e.g. Portland Cement 40kg")}
                <div class="grid2">
                    {field("Location / Bin", location, "e.g. Rack A1")}
                    {field("Reorder Level", reorder, "alert when balance is at or below")}
                </div>
                {field("Remarks", remarks, "")}
                {(!is_new).then(|| view! {
                    <p class="muted small">"Current balance: " <b>{fq(balance)}</b>
                    ". To change the balance, record a Stock In or Stock Out."</p>
                })}
                <div class="modal-foot">
                    {(!is_new).then(|| view! { <button class="btn danger ghost" on:click=delete>
                        {move || if armed.get() { "Tap again to delete" } else { "Delete" }}
                    </button> })}
                    <div class="grow"></div>
                    <button class="btn ghost" on:click=move |_| close.set(None)>"Cancel"</button>
                    <button class="btn primary" disabled=move || busy.get() on:click=save>"Save"</button>
                </div>
            </div>
        </div>
    }
}
