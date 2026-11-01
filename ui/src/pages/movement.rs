use inventory_types::{FileFormat, Movement, MovementInput, MovementKind, ReportKind};
use leptos::prelude::*;
use leptos::task::spawn_local;

use super::{export, request, ItemPicker};
use crate::util::{fq, kind_label, parse_qty, today};
use crate::{api, ctx};

/// Stock In / Stock Out entry form.
#[component]
pub fn MovementPage(kind: MovementKind) -> impl IntoView {
    let ctx = ctx();
    let item = RwSignal::new(ctx.focus_item.get_untracked());
    ctx.focus_item.set(None);
    let qty = RwSignal::new(String::new());
    let date = RwSignal::new(today());
    let reference = RwSignal::new(String::new());
    let party = RwSignal::new(String::new());
    let remarks = RwSignal::new(String::new());
    let recorded_by = RwSignal::new(ctx.settings.get_untracked().recorded_by);
    let busy = RwSignal::new(false);
    let last = RwSignal::new(None::<Movement>);

    let (party_label, party_ph, ref_ph) = match kind {
        MovementKind::In => ("Received From", "Supplier / source", "Delivery receipt no., e.g. DR-10021"),
        MovementKind::Out => ("Issued To", "Person, department or site", "Requisition / issue slip no., e.g. RS-0311"),
    };

    let save = move |_| {
        let Some(item_id) = item.get_untracked() else {
            return ctx.err("Choose an item first.");
        };
        let Some(q) = parse_qty(&qty.get_untracked()).filter(|q| *q > 0.0) else {
            return ctx.err("Enter a quantity greater than zero.");
        };
        let input = MovementInput {
            item_id,
            kind,
            qty: q,
            date: date.get_untracked(),
            reference: reference.get_untracked(),
            party: party.get_untracked(),
            remarks: remarks.get_untracked(),
            recorded_by: recorded_by.get_untracked(),
        };
        busy.set(true);
        spawn_local(async move {
            let res = api::record_movement(&input).await;
            busy.set(false);
            match res {
                Ok(m) => {
                    ctx.ok(format!("{} saved: {} {} {}", kind_label(kind), fq(m.qty), m.unit, m.item_code));
                    last.set(Some(m));
                    // Keep date / party / recorded by for fast repeated entry.
                    qty.set(String::new());
                    remarks.set(String::new());
                    item.set(None);
                    ctx.changed();
                }
                Err(e) => ctx.err(e),
            }
        });
    };

    let print_slip = move |_| {
        if let Some(m) = last.get_untracked() {
            let mut r = request(ReportKind::Slip, FileFormat::Pdf);
            r.movement_id = Some(m.id);
            export(ctx, r);
        }
    };

    let text = move |label: &'static str, sig: RwSignal<String>, ph: &'static str| {
        view! {
            <label class="field">
                <span>{label}</span>
                <input class="input" placeholder=ph prop:value=move || sig.get()
                    on:input=move |ev| sig.set(event_target_value(&ev))/>
            </label>
        }
    };

    view! {
        <div class="stack narrow">
            <section class={if kind == MovementKind::In { "panel accent-in" } else { "panel accent-out" }}>
                <div class="panel-head">
                    <h2>{kind_label(kind)}</h2>
                    <span class={if kind == MovementKind::In { "badge in" } else { "badge out" }}>{kind.as_str()}</span>
                </div>

                <div class="field">
                    <span>"Item *"</span>
                    <ItemPicker selected=item/>
                </div>

                <div class="grid2">
                    <label class="field">
                        <span>"Quantity *"</span>
                        <input class="input qty" inputmode="decimal" placeholder="0"
                            prop:value=move || qty.get()
                            on:input=move |ev| qty.set(event_target_value(&ev))/>
                    </label>
                    <label class="field">
                        <span>"Date *"</span>
                        <input class="input" type="date" prop:value=move || date.get()
                            on:input=move |ev| date.set(event_target_value(&ev))/>
                    </label>
                </div>
                <div class="grid2">
                    {text("Reference No.", reference, ref_ph)}
                    {text(party_label, party, party_ph)}
                </div>
                <div class="grid2">
                    {text("Remarks", remarks, "Optional")}
                    {text("Recorded By", recorded_by, "Warehouse man")}
                </div>

                <button class={if kind == MovementKind::In { "btn big in full" } else { "btn big out full" }}
                    disabled=move || busy.get() on:click=save>
                    {move || if busy.get() { "Saving…".to_string() } else { format!("Save {}", kind_label(kind)) }}
                </button>
            </section>

            {move || last.get().map(|m| view! {
                <section class="panel saved">
                    <div>
                        <div class="muted small">"Last saved"</div>
                        <div><span class="code">{m.item_code}</span>" "{m.item_name}</div>
                        <div class="muted small">{format!("{} {} {} · {}", m.kind.as_str(), fq(m.qty), m.unit, m.date)}</div>
                    </div>
                    <button class="btn" on:click=print_slip>
                        {if m.kind == MovementKind::In { "⎙ Receiving Slip" } else { "⎙ Issue Slip" }}
                    </button>
                </section>
            })}
        </div>
    }
}
