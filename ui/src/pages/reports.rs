use inventory_types::{FileFormat, MovementKind, ReportKind, SummaryRow};
use leptos::prelude::*;
use leptos::task::spawn_local;

use super::{request, DateRange, ExportButtons};
use crate::util::{fq, month_start, opt, today};
use crate::{api, ctx};

#[component]
pub fn ReportsPage() -> impl IntoView {
    let ctx = ctx();
    let from = RwSignal::new(month_start());
    let to = RwSignal::new(today());
    let rows = RwSignal::new(Vec::<SummaryRow>::new());
    let moved_only = RwSignal::new(false);

    Effect::new(move |_| {
        let _ = ctx.refresh.get();
        let (f, t) = (opt(from.get()), opt(to.get()));
        spawn_local(async move {
            match api::inout_summary(f, t).await {
                Ok(v) => rows.set(v),
                Err(e) => ctx.err(e),
            }
        });
    });

    let ranged = move |report: ReportKind, kind: Option<MovementKind>| {
        move |fmt: FileFormat| {
            let mut r = request(report, fmt);
            r.from = opt(from.get_untracked());
            r.to = opt(to.get_untracked());
            r.kind = kind;
            r
        }
    };
    let inventory = |fmt: FileFormat| request(ReportKind::Inventory, fmt);
    let low = |fmt: FileFormat| {
        let mut r = request(ReportKind::Inventory, fmt);
        r.low_only = true;
        r
    };

    view! {
        <div class="stack">
            <div class="report-grid">
                <div class="panel report">
                    <h3>"Inventory Report"</h3>
                    <p class="muted small">"Current Total In, Total Out and Balance of every item."</p>
                    <ExportButtons make=inventory/>
                </div>
                <div class="panel report">
                    <h3>"Low Stock / Reorder"</h3>
                    <p class="muted small">"Items at or below their reorder level."</p>
                    <ExportButtons make=low/>
                </div>
            </div>

            <div class="panel filters">
                <h3>"Reports for a period"</h3>
                <DateRange from=from to=to/>
                <div class="report-grid">
                    <div class="report">
                        <h4>"In / Out Summary"</h4>
                        <p class="muted small">"Beginning, In, Out and Ending Balance per item."</p>
                        <ExportButtons make=ranged(ReportKind::Summary, None)/>
                    </div>
                    <div class="report">
                        <h4>"Stock In (Receiving) Report"</h4>
                        <p class="muted small">"All deliveries received in the period."</p>
                        <ExportButtons make=ranged(ReportKind::Movements, Some(MovementKind::In))/>
                    </div>
                    <div class="report">
                        <h4>"Stock Out (Issuance) Report"</h4>
                        <p class="muted small">"All items issued in the period."</p>
                        <ExportButtons make=ranged(ReportKind::Movements, Some(MovementKind::Out))/>
                    </div>
                </div>
            </div>

            <div class="panel flush">
                <div class="panel-head padded">
                    <h2>"In / Out Summary preview"</h2>
                    <label class="toggle">
                        <input type="checkbox" prop:checked=move || moved_only.get()
                            on:change=move |ev| moved_only.set(event_target_checked(&ev))/>
                        <span>"Only items with movement"</span>
                    </label>
                </div>
                <table class="table">
                    <thead>
                        <tr>
                            <th>"Item Code"</th>
                            <th>"Item Name"</th>
                            <th class="hide-sm">"Unit"</th>
                            <th class="num">"Beginning"</th>
                            <th class="num">"In"</th>
                            <th class="num">"Out"</th>
                            <th class="num">"Ending"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || {
                            let only = moved_only.get();
                            rows.get().into_iter()
                                .filter(|r| !only || r.total_in != 0.0 || r.total_out != 0.0)
                                .map(|r| view! {
                                    <tr>
                                        <td class="code">{r.code}</td>
                                        <td>{r.name}</td>
                                        <td class="hide-sm">{r.unit}</td>
                                        <td class="num">{fq(r.beginning)}</td>
                                        <td class="num in-text">{fq(r.total_in)}</td>
                                        <td class="num out-text">{fq(r.total_out)}</td>
                                        <td class="num strong">{fq(r.ending)}</td>
                                    </tr>
                                }).collect_view()
                        }}
                    </tbody>
                </table>
            </div>
        </div>
    }
}
