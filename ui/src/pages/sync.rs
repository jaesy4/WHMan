use inventory_types::{PairingOffer, PeerInfo, SyncReport};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::{api, ctx};

#[component]
pub fn SyncPage() -> impl IntoView {
    let ctx = ctx();
    view! {
        {move || match ctx.device.get() {
            Some(d) if !d.is_hub => view! { <PhoneSync/> }.into_any(),
            _ => view! { <HubSync/> }.into_any(),
        }}
    }
}

#[component]
fn PeerList(peers: RwSignal<Vec<PeerInfo>>, label: &'static str) -> impl IntoView {
    let ctx = ctx();
    let remove = move |id: String| {
        spawn_local(async move {
            match api::remove_peer(&id).await {
                Ok(()) => {
                    ctx.ok("Removed");
                    if let Ok(p) = api::list_peers().await {
                        peers.set(p);
                    }
                }
                Err(e) => ctx.err(e),
            }
        });
    };
    view! {
        <section class="panel">
            <h2>{label}</h2>
            {move || {
                let list = peers.get();
                if list.is_empty() {
                    view! { <div class="empty small">"None yet."</div> }.into_any()
                } else {
                    view! {
                        <ul class="feed">
                            {list.into_iter().map(|p| {
                                let id = p.peer_id.clone();
                                view! {
                                    <li class="feed-row">
                                        <div class="feed-main">
                                            <div class="strong">{p.name}</div>
                                            <div class="muted small">
                                                {format!("Last sync: {}", p.last_sync_at.unwrap_or_else(|| "never".into()))}
                                                {(!p.address.is_empty()).then(|| format!(" · {}", p.address))}
                                            </div>
                                        </div>
                                        <button class="btn ghost small danger" on:click=move |_| remove(id.clone())>"Remove"</button>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }
            }}
        </section>
    }
}

/// Desktop: show the pairing QR code and the list of paired phones.
#[component]
fn HubSync() -> impl IntoView {
    let ctx = ctx();
    let offer = RwSignal::new(None::<PairingOffer>);
    let peers = RwSignal::new(Vec::<PeerInfo>::new());
    let address = RwSignal::new(String::new());

    let load = move || {
        spawn_local(async move {
            if let Ok(p) = api::list_peers().await {
                peers.set(p);
            }
            if let Ok(a) = api::hub_address().await {
                address.set(a);
            }
        });
    };
    load();
    // Refresh the list every few seconds while the page is open (to show
    // new pairings and sync times from phones).
    let handle = set_interval_with_handle(move || load(), std::time::Duration::from_secs(4)).ok();
    on_cleanup(move || {
        if let Some(h) = handle {
            h.clear();
        }
    });

    let start = move |_| {
        spawn_local(async move {
            match api::start_pairing().await {
                Ok(o) => offer.set(Some(o)),
                Err(e) => ctx.err(e),
            }
        });
    };
    let stop = move |_| {
        offer.set(None);
        spawn_local(async move {
            let _ = api::cancel_pairing().await;
        });
    };

    view! {
        <div class="stack narrow">
            <section class="panel">
                <h2>"Phone sync (local Wi-Fi)"</h2>
                <p class="muted">
                    "This PC is the hub. Phones on the same Wi-Fi send their Stock In / Stock Out records here and "
                    "receive the latest items and balances. No internet needed."
                </p>
                <div class="kv"><span>"Hub address"</span><b>{move || address.get()}</b></div>
                <p class="muted small">
                    "Keep this app open while phones sync. If Windows asks, allow access on Private networks."
                </p>
                {move || match offer.get() {
                    None => view! { <button class="btn primary" on:click=start>"Pair a phone"</button> }.into_any(),
                    Some(o) => view! {
                        <div class="pairing">
                            <div class="qr" inner_html=o.qr_svg.clone()></div>
                            <div>
                                <ol class="steps">
                                    <li>"Open the app on the phone."</li>
                                    <li>"Go to Sync and tap " <b>"Scan PC QR code"</b> "."</li>
                                    <li>"Point the camera at this code."</li>
                                </ol>
                                <p class="muted small">{format!("Code is valid for {} minutes and works once.", o.expires_in_secs / 60)}</p>
                                <details>
                                    <summary class="muted small">"Camera not working? Copy the code instead"</summary>
                                    <textarea class="input mono" readonly rows="4">{o.payload.clone()}</textarea>
                                </details>
                                <button class="btn ghost" on:click=stop>"Done"</button>
                            </div>
                        </div>
                    }.into_any(),
                }}
            </section>
            <PeerList peers=peers label="Paired phones"/>
        </div>
    }
}

/// Phone: scan the PC's QR code, then "Sync now".
#[component]
fn PhoneSync() -> impl IntoView {
    let ctx = ctx();
    let peers = RwSignal::new(Vec::<PeerInfo>::new());
    let busy = RwSignal::new(false);
    let last = RwSignal::new(None::<SyncReport>);
    let manual = RwSignal::new(String::new());

    let load = move || {
        spawn_local(async move {
            if let Ok(p) = api::list_peers().await {
                peers.set(p);
            }
        });
    };
    load();

    let pair_with = move |text: String| {
        busy.set(true);
        spawn_local(async move {
            let res = api::pair_with_hub(&text).await;
            busy.set(false);
            match res {
                Ok(p) => {
                    ctx.ok(format!("Paired with {}", p.name));
                    manual.set(String::new());
                    load();
                }
                Err(e) => ctx.err(e),
            }
        });
    };

    let scan = move |_| {
        spawn_local(async move {
            match api::scan_qr().await {
                Ok(text) => pair_with(text),
                Err(e) => ctx.err(format!("Scan failed: {e}")),
            }
        });
    };

    let sync = move |_| {
        busy.set(true);
        spawn_local(async move {
            let res = api::sync_now().await;
            busy.set(false);
            match res {
                Ok(r) => {
                    ctx.ok(format!("Synced: sent {}, received {}", r.pushed, r.pulled));
                    last.set(Some(r));
                    ctx.changed();
                    load();
                }
                Err(e) => ctx.err(e),
            }
        });
    };

    view! {
        <div class="stack narrow">
            <section class="panel">
                <h2>"Sync with the PC"</h2>
                {move || if peers.get().is_empty() {
                    view! {
                        <div>
                            <p class="muted">
                                "First, pair this phone: on the PC open Sync → Pair a phone, then scan the QR code."
                            </p>
                            <button class="btn primary big full" disabled=move || busy.get() on:click=scan>"Scan PC QR code"</button>
                            <details>
                                <summary class="muted small">"Paste the code instead"</summary>
                                <textarea class="input mono" rows="4" prop:value=move || manual.get()
                                    on:input=move |ev| manual.set(event_target_value(&ev))></textarea>
                                <button class="btn" on:click=move |_| pair_with(manual.get_untracked())>"Pair"</button>
                            </details>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div>
                            <p class="muted">"Both devices must be on the same Wi-Fi and the PC app must be open."</p>
                            <button class="btn primary big full" disabled=move || busy.get() on:click=sync>
                                {move || if busy.get() { "Syncing…" } else { "⇄ Sync now" }}
                            </button>
                        </div>
                    }.into_any()
                }}
                {move || last.get().map(|r| view! {
                    <div class="kv-list">
                        <div class="kv"><span>"Sent to PC"</span><b>{r.pushed}</b></div>
                        <div class="kv"><span>"Received"</span><b>{r.pulled}</b></div>
                        <div class="kv"><span>"PC address"</span><b>{r.address}</b></div>
                        <div class="kv"><span>"Finished"</span><b>{r.finished_at}</b></div>
                    </div>
                })}
            </section>
            <PeerList peers=peers label="Paired PC"/>
            {move || (!peers.get().is_empty()).then(|| view! {
                <section class="panel">
                    <h2>"Pair again"</h2>
                    <p class="muted small">"Use this if the PC app was reinstalled or you want to switch to another PC."</p>
                    <button class="btn" disabled=move || busy.get() on:click=scan>"Scan PC QR code"</button>
                </section>
            })}
        </div>
    }
}
