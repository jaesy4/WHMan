use inventory_types::{Dashboard, Movement, MovementFilter, MovementKind};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::util::{fq, today};
use crate::{api, ctx, Page};

#[component]
pub fn HomePage() -> impl IntoView {
    let ctx = ctx();
    let dash = RwSignal::new(None::<Dashboard>);
    let recent = RwSignal::new(Vec::<Movement>::new());

    Effect::new(move |_| {
        let _ = ctx.refresh.get();
        spawn_local(async move {
            match api::dashboard().await {
                Ok(d) => dash.set(Some(d)),
                Err(e) => ctx.err(e),
            }
            let f = MovementFilter { limit: Some(10), ..Default::default() };
            if let Ok(list) = api::list_movements(&f).await {
                recent.set(list);
            }
        });
    });

    view! {
        <div class="stack">
            <div class="hero">
                <div>
                    <div class="muted">{today()}</div>
                    <div class="hero-title">"What are we recording?"</div>
                </div>
                <div class="hero-actions">
                    <button class="btn big in" on:click=move |_| ctx.open_movement(MovementKind::In, None)>
                        "↓ Stock In"
                    </button>
                    <button class="btn big out" on:click=move |_| ctx.open_movement(MovementKind::Out, None)>
                        "↑ Stock Out"
                    </button>
                </div>
            </div>

            <div class="cards">
                {move || {
                    let d = dash.get();
                    let v = |f: fn(&Dashboard) -> String| d.as_ref().map(f).unwrap_or_else(|| "…".into());
                    view! {
                        <button class="stat" on:click=move |_| ctx.go(Page::Inventory)>
                            <div class="stat-label">"Items"</div>
                            <div class="stat-value">{v(|d| d.item_count.to_string())}</div>
                        </button>
                        <button class="stat warn" on:click=move |_| ctx.go(Page::Inventory)>
                            <div class="stat-label">"Low stock"</div>
                            <div class="stat-value">{v(|d| d.low_stock_count.to_string())}</div>
                        </button>
                        <div class="stat in">
                            <div class="stat-label">"Stock In today"</div>
                            <div class="stat-value">{v(|d| d.today_in_count.to_string())}</div>
                            <div class="stat-sub">{v(|d| format!("{} units", fq(d.today_in)))}</div>
                        </div>
                        <div class="stat out">
                            <div class="stat-label">"Stock Out today"</div>
                            <div class="stat-value">{v(|d| d.today_out_count.to_string())}</div>
                            <div class="stat-sub">{v(|d| format!("{} units", fq(d.today_out)))}</div>
                        </div>
                    }
                }}
            </div>

            <section class="panel">
                <div class="panel-head">
                    <h2>"Recent records"</h2>
                    <button class="btn ghost small" on:click=move |_| ctx.go(Page::Transactions)>"View all"</button>
                </div>
                {move || {
                    let list = recent.get();
                    if list.is_empty() {
                        view! { <div class="empty">"No records yet. Start with Stock In, or import your item list."</div> }.into_any()
                    } else {
                        view! {
                            <ul class="feed">
                                {list.into_iter().map(|m| view! {
                                    <li class="feed-row">
                                        <span class={if m.kind == MovementKind::In { "badge in" } else { "badge out" }}>
                                            {m.kind.as_str()}
                                        </span>
                                        <div class="feed-main">
                                            <div><span class="code">{m.item_code}</span>" "{m.item_name}</div>
                                            <div class="muted small">
                                                {m.date}" · "{if m.reference.is_empty() { "no ref".to_string() } else { m.reference }}
                                                {(!m.party.is_empty()).then(|| format!(" · {}", m.party))}
                                            </div>
                                        </div>
                                        <div class="feed-qty">{fq(m.qty)}" "<span class="muted">{m.unit}</span></div>
                                    </li>
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    }
                }}
            </section>
        </div>
    }
}
