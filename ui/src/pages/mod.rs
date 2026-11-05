//! Screens and small shared components.

pub mod home;
pub mod import;
pub mod inventory;
pub mod movement;
pub mod portal;
pub mod reports;
pub mod requests;
pub mod settings;
pub mod stock_card;
pub mod sync;
pub mod transactions;

use inventory_types::{FileFormat, ReportKind, ReportRequest, StockRow};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::util::{fq, month_start, today};
use crate::{api, ctx, Ctx};

/// Asks the backend to build a report and shows a Save dialog.
pub fn export(ctx: Ctx, req: ReportRequest) {
    spawn_local(async move {
        match api::export_report(&req).await {
            Ok(Some(path)) => ctx.ok(format!("Saved: {path}")),
            Ok(None) => {}
            Err(e) => ctx.err(e),
        }
    });
}

pub fn request(report: ReportKind, format: FileFormat) -> ReportRequest {
    ReportRequest {
        report,
        format,
        from: None,
        to: None,
        item_id: None,
        movement_id: None,
        kind: None,
        low_only: false,
    }
}

/// Searchable item selector showing the current balance.
#[component]
pub fn ItemPicker(selected: RwSignal<Option<String>>) -> impl IntoView {
    let ctx = ctx();
    let items = RwSignal::new(Vec::<StockRow>::new());
    let query = RwSignal::new(String::new());
    let open = RwSignal::new(false);

    Effect::new(move |_| {
        let _ = ctx.refresh.get();
        spawn_local(async move {
            match api::stock_list("", false).await {
                Ok(v) => items.set(v),
                Err(e) => ctx.err(e),
            }
        });
    });

    let current = move || selected.get().and_then(|id| items.get().into_iter().find(|r| r.item.id == id));
    let matches = move || {
        let q = query.get().to_lowercase();
        items
            .get()
            .into_iter()
            .filter(|r| {
                q.is_empty()
                    || r.item.code.to_lowercase().contains(&q)
                    || r.item.name.to_lowercase().contains(&q)
                    || r.item.location.to_lowercase().contains(&q)
            })
            .take(40)
            .collect::<Vec<_>>()
    };

    view! {
        <div class="picker">
            {move || match current() {
                Some(r) => view! {
                    <div class="picked">
                        <div class="picked-main">
                            <div><span class="code">{r.item.code}</span>" "{r.item.name}</div>
                            <div class="muted">
                                "Balance: " <b>{fq(r.balance)}</b> " " {r.item.unit.clone()}
                                {(!r.item.location.is_empty()).then(|| format!("  ·  {}", r.item.location))}
                            </div>
                        </div>
                        <button class="btn ghost small" on:click=move |_| { selected.set(None); open.set(true); }>
                            "Change"
                        </button>
                    </div>
                }.into_any(),
                None => view! {
                    <div>
                        <input
                            class="input"
                            placeholder="Search item code, name or location…"
                            prop:value=move || query.get()
                            on:input=move |ev| { query.set(event_target_value(&ev)); open.set(true); }
                            on:focus=move |_| open.set(true)
                        />
                        <Show when=move || open.get()>
                            <div class="picker-list">
                                {move || {
                                    let list = matches();
                                    if list.is_empty() {
                                        view! { <div class="empty small">"No matching items."</div> }.into_any()
                                    } else {
                                        list.into_iter().map(|r| {
                                            let id = r.item.id.clone();
                                            view! {
                                                <button class="picker-row" on:click=move |_| {
                                                    selected.set(Some(id.clone()));
                                                    open.set(false);
                                                    query.set(String::new());
                                                }>
                                                    <span class="code">{r.item.code}</span>
                                                    <span class="name">{r.item.name}</span>
                                                    <span class="bal">{fq(r.balance)}" "{r.item.unit}</span>
                                                </button>
                                            }
                                        }).collect_view().into_any()
                                    }
                                }}
                            </div>
                        </Show>
                    </div>
                }.into_any(),
            }}
        </div>
    }
}

/// From / To date inputs with quick ranges.
#[component]
pub fn DateRange(from: RwSignal<String>, to: RwSignal<String>) -> impl IntoView {
    view! {
        <div class="date-range">
            <label class="field compact">
                <span>"From"</span>
                <input class="input" type="date" prop:value=move || from.get()
                    on:input=move |ev| from.set(event_target_value(&ev))/>
            </label>
            <label class="field compact">
                <span>"To"</span>
                <input class="input" type="date" prop:value=move || to.get()
                    on:input=move |ev| to.set(event_target_value(&ev))/>
            </label>
            <div class="chips">
                <button class="chip" on:click=move |_| { from.set(today()); to.set(today()); }>"Today"</button>
                <button class="chip" on:click=move |_| { from.set(month_start()); to.set(today()); }>"This month"</button>
                <button class="chip" on:click=move |_| { from.set(String::new()); to.set(String::new()); }>"All"</button>
            </div>
        </div>
    }
}

/// "PDF" + "Excel" buttons.
#[component]
pub fn ExportButtons<F>(make: F) -> impl IntoView
where
    F: Fn(FileFormat) -> ReportRequest + Copy + Send + Sync + 'static,
{
    let ctx = ctx();
    view! {
        <div class="btn-row">
            <button class="btn" on:click=move |_| export(ctx, make(FileFormat::Pdf))>"⎙ PDF"</button>
            <button class="btn" on:click=move |_| export(ctx, make(FileFormat::Xlsx))>"▦ Excel"</button>
        </div>
    }
}
