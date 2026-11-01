use inventory_types::{FileFormat, MovementKind, ReportKind, StockCard};
use leptos::prelude::*;
use leptos::task::spawn_local;

use super::{request, DateRange, ExportButtons, ItemPicker};
use crate::util::{fq, opt};
use crate::{api, ctx};

#[component]
pub fn StockCardPage() -> impl IntoView {
    let ctx = ctx();
    let item = RwSignal::new(ctx.focus_item.get_untracked());
    ctx.focus_item.set(None);
    let from = RwSignal::new(String::new());
    let to = RwSignal::new(String::new());
    let card = RwSignal::new(None::<StockCard>);

    Effect::new(move |_| {
        let _ = ctx.refresh.get();
        let Some(id) = item.get() else {
            card.set(None);
            return;
        };
        let (f, t) = (opt(from.get()), opt(to.get()));
        spawn_local(async move {
            match api::stock_card(&id, f, t).await {
                Ok(c) => card.set(Some(c)),
                Err(e) => ctx.err(e),
            }
        });
    });

    let make = move |fmt: FileFormat| {
        let mut r = request(ReportKind::StockCard, fmt);
        r.item_id = item.get_untracked();
        r.from = opt(from.get_untracked());
        r.to = opt(to.get_untracked());
        r
    };

    view! {
        <div class="stack">
            <div class="panel filters">
                <div class="field"><span>"Item"</span><ItemPicker selected=item/></div>
                <DateRange from=from to=to/>
            </div>

            {move || card.get().map(|c| {
                let id_in = c.item.id.clone();
                let id_out = c.item.id.clone();
                let tin: f64 = c.lines.iter().map(|l| l.qty_in).sum();
                let tout: f64 = c.lines.iter().map(|l| l.qty_out).sum();
                view! {
                    <div class="panel flush">
                        <div class="panel-head padded">
                            <div>
                                <h2>{format!("{} — {}", c.item.code, c.item.name)}</h2>
                                <div class="muted small">{format!("Unit: {}  ·  Location: {}", c.item.unit, c.item.location)}</div>
                            </div>
                            <div class="btn-row">
                                <button class="mini in" on:click=move |_| ctx.open_movement(MovementKind::In, Some(id_in.clone()))>"IN"</button>
                                <button class="mini out" on:click=move |_| ctx.open_movement(MovementKind::Out, Some(id_out.clone()))>"OUT"</button>
                                <ExportButtons make=make/>
                            </div>
                        </div>
                        <table class="table">
                            <thead>
                                <tr>
                                    <th>"Date"</th>
                                    <th>"Reference"</th>
                                    <th class="hide-sm">"Received From / Issued To"</th>
                                    <th class="num">"In"</th>
                                    <th class="num">"Out"</th>
                                    <th class="num">"Balance"</th>
                                    <th class="hide-sm">"Remarks"</th>
                                </tr>
                            </thead>
                            <tbody>
                                <tr class="total">
                                    <td>{c.from.clone().unwrap_or_default()}</td>
                                    <td>"Beginning balance"</td>
                                    <td class="hide-sm"></td>
                                    <td class="num"></td>
                                    <td class="num"></td>
                                    <td class="num">{fq(c.beginning)}</td>
                                    <td class="hide-sm"></td>
                                </tr>
                                {c.lines.into_iter().map(|l| view! {
                                    <tr>
                                        <td>{l.date}</td>
                                        <td>{l.reference}</td>
                                        <td class="hide-sm">{l.party}</td>
                                        <td class="num in-text">{(l.qty_in != 0.0).then(|| fq(l.qty_in))}</td>
                                        <td class="num out-text">{(l.qty_out != 0.0).then(|| fq(l.qty_out))}</td>
                                        <td class="num strong">{fq(l.balance)}</td>
                                        <td class="hide-sm muted">{l.remarks}</td>
                                    </tr>
                                }).collect_view()}
                                <tr class="total">
                                    <td>{c.to.clone().unwrap_or_default()}</td>
                                    <td>"Ending balance"</td>
                                    <td class="hide-sm"></td>
                                    <td class="num">{fq(tin)}</td>
                                    <td class="num">{fq(tout)}</td>
                                    <td class="num">{fq(c.ending)}</td>
                                    <td class="hide-sm"></td>
                                </tr>
                            </tbody>
                        </table>
                    </div>
                }
            })}
            {move || item.get().is_none().then(|| view! {
                <div class="empty">"Choose an item to see every Stock In and Stock Out with the running balance."</div>
            })}
        </div>
    }
}
