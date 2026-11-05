use inventory_types::{Department, PortalInfo, UserRole, WebUser, WebUserInput};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::{api, ctx, Page};

/// Department web portal: online link, setup help and user accounts.
#[component]
pub fn PortalPage() -> impl IntoView {
    let ctx = ctx();
    let info = RwSignal::new(None::<PortalInfo>);
    let users = RwSignal::new(Vec::<WebUser>::new());
    let depts = RwSignal::new(Vec::<Department>::new());
    let public_url = RwSignal::new(String::new());
    // Some(None) = new user, Some(Some(u)) = edit user.
    let editing = RwSignal::new(None::<Option<WebUser>>);
    let deleting = RwSignal::new(None::<WebUser>);

    Effect::new(move |_| {
        let _ = ctx.refresh.get();
        spawn_local(async move {
            match api::portal_info().await {
                Ok(i) => {
                    public_url.set(i.public_url.clone());
                    info.set(Some(i));
                }
                Err(e) => ctx.err(e),
            }
            match api::list_web_users().await {
                Ok(u) => users.set(u),
                Err(e) => ctx.err(e),
            }
            if let Ok(d) = api::list_departments().await {
                depts.set(d);
            }
        });
    });

    let save_url = move |_| {
        let url = public_url.get_untracked();
        spawn_local(async move {
            match api::save_portal_url(&url).await {
                Ok(()) => {
                    ctx.ok("Online link saved");
                    ctx.changed();
                }
                Err(e) => ctx.err(e),
            }
        });
    };

    view! {
        <div class="stack narrow">
            <section class="panel">
                <div class="panel-head">
                    <h2>"Department web portal"</h2>
                    {move || info.get().map(|i| view! {
                        <span class="small"><span class="dot" class:on={i.running}></span>
                            {if i.running { "Running" } else { "Not running" }}</span>
                    })}
                </div>
                <p class="muted small">
                    "Engineering and Solar Team open this website on their phone or computer to request items. "
                    "Requests arrive in " <b>"Dept. Requests"</b> ". Keep this app open on the PC — when it is closed, the website is offline."
                </p>
                {move || info.get().map(|i| view! {
                    <div class="kv-list">
                        <div class="field">
                            <span>"Same Wi-Fi / office network"</span>
                            <div class="url-box"><code>{if i.lan_url.is_empty() { "Not connected to a network".to_string() } else { i.lan_url.clone() }}</code></div>
                        </div>
                        <div class="field">
                            <span>"Online link (from anywhere)"</span>
                            <div class="url-box">
                                <code>{if i.public_url.is_empty() { "Not set up yet — see the steps below".to_string() } else { i.public_url.clone() }}</code>
                            </div>
                        </div>
                        <div class="muted small">
                            {format!("{} open request(s).", i.open_requests)}" "
                            <button class="link" on:click=move |_| ctx.go(Page::Requests)>"View requests →"</button>
                        </div>
                    </div>
                })}
            </section>

            <section class="panel">
                <h2>"Put it online (Tailscale Funnel, free)"</h2>
                <ol class="steps-list small">
                    <li>"On this PC, install Tailscale from " <b>"tailscale.com/download"</b> " and sign in (a free personal account is enough)."</li>
                    <li>"In the Tailscale admin console, open " <b>"DNS"</b> " and turn on " <b>"MagicDNS"</b> " and " <b>"HTTPS Certificates"</b> "."</li>
                    <li>"Open " <b>"Command Prompt as Administrator"</b> " and run:" <br/>
                        <code>"tailscale funnel --bg 47812"</code></li>
                    <li>"It prints a link like " <code>"https://warehouse-pc.tail1234.ts.net"</code> ". Paste it below and save."</li>
                    <li>"Send that link to the departments. On a phone they can tap " <b>"Share → Add to Home Screen"</b> " so it opens like an app."</li>
                </ol>
                <p class="muted small">
                    "The first time, Tailscale may show a link asking you to allow Funnel; open it, approve, and run the command again. Funnel keeps running after restarts. To turn the online link off: " <code>"tailscale funnel reset"</code> ". "
                    "Only the request website goes online; phone sync stays on your Wi-Fi."
                </p>
                <label class="field">
                    <span>"Online link"</span>
                    <input class="input" placeholder="https://your-pc.tailXXXX.ts.net"
                        prop:value=move || public_url.get() on:input=move |ev| public_url.set(event_target_value(&ev))/>
                </label>
                <button class="btn primary" on:click=save_url>"Save link"</button>
            </section>

            <section class="panel">
                <div class="panel-head">
                    <h2>"Department accounts"</h2>
                    <button class="btn primary" on:click=move |_| editing.set(Some(None))>"+ Add user"</button>
                </div>
                <p class="muted small">
                    <b>"Requester"</b> " can send requests. " <b>"Department Head"</b>
                    " also approves their team's requests (their own requests are approved automatically)."
                </p>
                <div class="table-wrap">
                    <table class="table">
                        <thead><tr>
                            <th>"Name"</th>
                            <th>"Username"</th>
                            <th>"Department"</th>
                            <th>"Role"</th>
                            <th class="actions"></th>
                        </tr></thead>
                        <tbody>
                            {move || users.get().into_iter().map(|u| {
                                let for_edit = u.clone();
                                let for_del = u.clone();
                                view! {
                                    <tr class:void={!u.active}>
                                        <td>{u.full_name.clone()}</td>
                                        <td class="mono">{u.username.clone()}</td>
                                        <td>{u.department_name.clone()}</td>
                                        <td>{u.role.label()}{(!u.active).then_some(" (disabled)")}</td>
                                        <td class="actions">
                                            <button class="mini" on:click=move |_| editing.set(Some(Some(for_edit.clone())))>"Edit"</button>
                                            <button class="mini danger" on:click=move |_| deleting.set(Some(for_del.clone()))>"Delete"</button>
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                    {move || users.get().is_empty().then(|| view! {
                        <div class="empty">"No accounts yet. Add one for each person in Engineering and Solar Team."</div>
                    })}
                </div>
            </section>

            {move || editing.get().map(|u| view! { <UserDialog user={u} depts={depts.get_untracked()} close={editing}/> })}
            {move || deleting.get().map(|u| view! { <DeleteUserDialog user={u} close={deleting}/> })}
        </div>
    }
}

#[component]
fn UserDialog(user: Option<WebUser>, depts: Vec<Department>, close: RwSignal<Option<Option<WebUser>>>) -> impl IntoView {
    let ctx = ctx();
    let is_new = user.is_none();
    let id = user.as_ref().map(|u| u.id.clone());
    let username = RwSignal::new(user.as_ref().map(|u| u.username.clone()).unwrap_or_default());
    let full_name = RwSignal::new(user.as_ref().map(|u| u.full_name.clone()).unwrap_or_default());
    let dept = RwSignal::new(
        user.as_ref()
            .map(|u| u.department_id.clone())
            .or_else(|| depts.first().map(|d| d.id.clone()))
            .unwrap_or_default(),
    );
    let role = RwSignal::new(user.as_ref().map(|u| u.role).unwrap_or(UserRole::Requester).as_str().to_string());
    let active = RwSignal::new(user.as_ref().map(|u| u.active).unwrap_or(true));
    let password = RwSignal::new(String::new());
    let busy = RwSignal::new(false);

    let save = move |_| {
        let input = WebUserInput {
            username: username.get_untracked(),
            full_name: full_name.get_untracked(),
            department_id: dept.get_untracked(),
            role: UserRole::parse(&role.get_untracked()).unwrap_or(UserRole::Requester),
            password: password.get_untracked(),
            active: active.get_untracked(),
        };
        let id = id.clone();
        busy.set(true);
        spawn_local(async move {
            let res = api::save_web_user(id, &input).await;
            busy.set(false);
            match res {
                Ok(u) => {
                    ctx.ok(format!("Saved {} ({})", u.full_name, u.username));
                    ctx.changed();
                    close.set(None);
                }
                Err(e) => ctx.err(e),
            }
        });
    };

    let text = move |label: &'static str, sig: RwSignal<String>, ph: &'static str, kind: &'static str| {
        view! {
            <label class="field">
                <span>{label}</span>
                <input class="input" type=kind placeholder=ph autocomplete="off"
                    prop:value=move || sig.get() on:input=move |ev| sig.set(event_target_value(&ev))/>
            </label>
        }
    };

    view! {
        <div class="modal-back" on:click=move |_| close.set(None)>
            <div class="modal" on:click=|ev| ev.stop_propagation()>
                <div class="modal-head">
                    <h2>{if is_new { "Add user" } else { "Edit user" }}</h2>
                    <button class="icon-btn" on:click=move |_| close.set(None)>"✕"</button>
                </div>
                {text("Full name *", full_name, "e.g. Juan Dela Cruz", "text")}
                {text("Username *", username, "e.g. juan.eng (no spaces)", "text")}
                <div class="grid2">
                    <label class="field">
                        <span>"Department *"</span>
                        <select class="input" prop:value=move || dept.get() on:change=move |ev| dept.set(event_target_value(&ev))>
                            {depts.into_iter().map(|d| {
                                let selected = d.id == dept.get_untracked();
                                view! { <option value={d.id} selected=selected>{d.name}</option> }
                            }).collect_view()}
                        </select>
                    </label>
                    <label class="field">
                        <span>"Role *"</span>
                        <select class="input" prop:value=move || role.get() on:change=move |ev| role.set(event_target_value(&ev))>
                            <option value="requester" selected={role.get_untracked() == "requester"}>"Requester"</option>
                            <option value="head" selected={role.get_untracked() == "head"}>"Department Head"</option>
                        </select>
                    </label>
                </div>
                {text(
                    if is_new { "Password * (at least 6 characters)" } else { "New password (leave empty to keep)" },
                    password, "", "password",
                )}
                <label class="toggle">
                    <input type="checkbox" prop:checked=move || active.get() on:change=move |ev| active.set(event_target_checked(&ev))/>
                    <span>"Account enabled"</span>
                </label>
                <p class="muted small">"Give the username and password to the person. They can change the password themselves after logging in."</p>
                <div class="modal-foot">
                    <div class="grow"></div>
                    <button class="btn ghost" on:click=move |_| close.set(None)>"Cancel"</button>
                    <button class="btn primary" disabled=move || busy.get() on:click=save>
                        {move || if busy.get() { "Saving…" } else { "Save" }}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
fn DeleteUserDialog(user: WebUser, close: RwSignal<Option<WebUser>>) -> impl IntoView {
    let ctx = ctx();
    let id = user.id.clone();
    let confirm = move |_| {
        let id = id.clone();
        spawn_local(async move {
            match api::delete_web_user(&id).await {
                Ok(()) => {
                    ctx.ok("User deleted");
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
                    <h2>"Delete user"</h2>
                    <button class="icon-btn" on:click=move |_| close.set(None)>"✕"</button>
                </div>
                <p>{format!("Delete {} ({}, {})?", user.full_name, user.username, user.department_name)}</p>
                <p class="muted small">"Their past requests are kept. To block someone only for a while, edit the user and untick \"Account enabled\" instead."</p>
                <div class="modal-foot">
                    <div class="grow"></div>
                    <button class="btn ghost" on:click=move |_| close.set(None)>"Cancel"</button>
                    <button class="btn danger" on:click=confirm>"Delete"</button>
                </div>
            </div>
        </div>
    }
}
