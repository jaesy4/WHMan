use inventory_types::{ImportKind, ImportPreview};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::{api, ctx, Page};

#[component]
pub fn ImportPage() -> impl IntoView {
    let ctx = ctx();
    let preview = RwSignal::new(None::<ImportPreview>);
    let busy = RwSignal::new(false);

    let template = move |kind: ImportKind| {
        spawn_local(async move {
            match api::save_template(kind).await {
                Ok(Some(p)) => ctx.ok(format!("Template saved: {p}")),
                Ok(None) => {}
                Err(e) => ctx.err(e),
            }
        });
    };

    let pick = move |_| {
        busy.set(true);
        spawn_local(async move {
            let res = api::import_pick_preview().await;
            busy.set(false);
            match res {
                Ok(Some(p)) => preview.set(Some(p)),
                Ok(None) => {}
                Err(e) => ctx.err(e),
            }
        });
    };

    let commit = move |_| {
        busy.set(true);
        spawn_local(async move {
            let res = api::import_commit().await;
            busy.set(false);
            match res {
                Ok(r) => {
                    ctx.ok(format!(
                        "Imported: {} new items, {} updated, {} IN/OUT records",
                        r.items_created, r.items_updated, r.movements_created
                    ));
                    preview.set(None);
                    ctx.changed();
                    ctx.go(Page::Inventory);
                }
                Err(e) => ctx.err(e),
            }
        });
    };

    let cancel = move |_| {
        preview.set(None);
        spawn_local(async move {
            let _ = api::import_cancel().await;
        });
    };

    view! {
        <div class="stack narrow">
            <section class="panel">
                <h2>"1. Prepare your Excel file"</h2>
                <p class="muted">
                    "Two kinds of sheet are supported. The app recognises the columns by their header names, in any order."
                </p>
                <div class="report-grid">
                    <div class="report">
                        <h4>"Item list"</h4>
                        <p class="muted small">
                            "Item Code, Item Name, Unit, Location, Reorder Level, Beginning Balance. "
                            "Existing codes are updated; new codes are added with their beginning balance as a Stock In."
                        </p>
                        <button class="btn" on:click=move |_| template(ImportKind::Items)>"Download item template"</button>
                    </div>
                    <div class="report">
                        <h4>"Stock In / Stock Out records"</h4>
                        <p class="muted small">
                            "Date, Item Code, Type (IN/OUT), Quantity, Reference, Received From / Issued To, Remarks. "
                            "Separate \"In\" and \"Out\" columns also work."
                        </p>
                        <button class="btn" on:click=move |_| template(ImportKind::Movements)>"Download IN/OUT template"</button>
                    </div>
                </div>
            </section>

            <section class="panel">
                <h2>"2. Choose the file"</h2>
                <p class="muted small">"Nothing is saved until you confirm the preview."</p>
                <button class="btn primary" disabled=move || busy.get() on:click=pick>
                    {move || if busy.get() { "Reading…" } else { "Choose Excel file (.xlsx)" }}
                </button>
            </section>

            {move || preview.get().map(|p| {
                let can_import = p.valid_rows > 0;
                let has_errors = !p.errors.is_empty();
                let kind_text = match p.kind {
                    ImportKind::Items => "Item list",
                    ImportKind::Movements => "Stock In / Stock Out records",
                };
                view! {
                    <section class="panel">
                        <h2>"3. Check and confirm"</h2>
                        <div class="muted small">{format!("{} · sheet \"{}\" · {}", p.file_name, p.sheet, kind_text)}</div>
                        <div class="cards small">
                            <div class="stat"><div class="stat-label">"Valid rows"</div><div class="stat-value">{p.valid_rows}</div></div>
                            {(p.kind == ImportKind::Items).then(|| view! {
                                <div class="stat in"><div class="stat-label">"New items"</div><div class="stat-value">{p.new_items}</div></div>
                                <div class="stat"><div class="stat-label">"Updated items"</div><div class="stat-value">{p.updated_items}</div></div>
                            })}
                            <div class="stat"><div class="stat-label">"IN/OUT records"</div><div class="stat-value">{p.movements}</div></div>
                            <div class="stat" class:warn=has_errors>
                                <div class="stat-label">"Rows with errors"</div>
                                <div class="stat-value">{p.errors.len()}</div>
                            </div>
                        </div>

                        {has_errors.then(|| view! {
                            <div class="issues error">
                                <b>"These rows will be skipped:"</b>
                                <ul>{p.errors.iter().take(200).map(|i| view! { <li>{format!("Row {}: {}", i.row, i.message)}</li> }).collect_view()}</ul>
                            </div>
                        })}
                        {(!p.warnings.is_empty()).then(|| view! {
                            <div class="issues warn">
                                <b>"Notes:"</b>
                                <ul>{p.warnings.iter().take(200).map(|i| view! { <li>{format!("Row {}: {}", i.row, i.message)}</li> }).collect_view()}</ul>
                            </div>
                        })}

                        <div class="table-wrap">
                            <table class="table">
                                <thead><tr>{p.sample_headers.iter().map(|h| view! { <th>{h.clone()}</th> }).collect_view()}</tr></thead>
                                <tbody>
                                    {p.sample.iter().map(|row| view! {
                                        <tr>{row.iter().map(|c| view! { <td>{c.clone()}</td> }).collect_view()}</tr>
                                    }).collect_view()}
                                </tbody>
                            </table>
                        </div>
                        {(p.valid_rows as usize > p.sample.len()).then(|| view! {
                            <div class="muted small">{format!("Showing first {} of {} rows.", p.sample.len(), p.valid_rows)}</div>
                        })}

                        <div class="modal-foot">
                            <button class="btn ghost" on:click=cancel>"Cancel"</button>
                            <div class="grow"></div>
                            <button class="btn primary" disabled=move || busy.get() || !can_import on:click=commit>
                                {format!("Import {} rows", p.valid_rows)}
                            </button>
                        </div>
                    </section>
                }
            })}
        </div>
    }
}
