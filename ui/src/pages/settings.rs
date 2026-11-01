use inventory_types::Settings;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::{api, ctx};

#[component]
pub fn SettingsPage() -> impl IntoView {
    let ctx = ctx();
    let s = ctx.settings.get_untracked();
    let warehouse = RwSignal::new(s.warehouse_name);
    let recorded_by = RwSignal::new(s.recorded_by);
    let device = RwSignal::new(s.device_name);

    let save = move |_| {
        let new = Settings {
            warehouse_name: warehouse.get_untracked(),
            recorded_by: recorded_by.get_untracked(),
            device_name: device.get_untracked(),
        };
        spawn_local(async move {
            match api::save_settings(&new).await {
                Ok(()) => {
                    if let Ok(s) = api::get_settings().await {
                        ctx.settings.set(s);
                    }
                    if let Ok(d) = api::device_info().await {
                        ctx.device.set(Some(d));
                    }
                    ctx.ok("Settings saved");
                }
                Err(e) => ctx.err(e),
            }
        });
    };

    let field = move |label: &'static str, hint: &'static str, sig: RwSignal<String>| {
        view! {
            <label class="field">
                <span>{label}</span>
                <input class="input" prop:value=move || sig.get() on:input=move |ev| sig.set(event_target_value(&ev))/>
                <small class="muted">{hint}</small>
            </label>
        }
    };

    view! {
        <div class="stack narrow">
            <section class="panel">
                <h2>"Settings"</h2>
                {field("Warehouse / company name", "Printed at the top of every report.", warehouse)}
                {field("Default \"Recorded by\"", "Your name, filled in automatically on Stock In / Stock Out.", recorded_by)}
                {field("Device name", "Shown to other devices when syncing (e.g. \"Warehouse PC\", \"Juan's phone\").", device)}
                <button class="btn primary" on:click=save>"Save settings"</button>
            </section>
            <section class="panel">
                <h2>"About"</h2>
                <p class="muted small">
                    "Warehouse In/Out Inventory · data is stored on this device (SQLite) and synced over local Wi-Fi. "
                    "Back up by copying inventory.db from the app data folder."
                </p>
            </section>
        </div>
    }
}
