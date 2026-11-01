//! Warehouse Inventory — user interface (Leptos, compiled to WebAssembly).

mod api;
mod pages;
mod util;

use inventory_types::{DeviceInfo, MovementKind, Settings};
use leptos::prelude::*;
use leptos::task::spawn_local;
use pages::home::HomePage;
use pages::import::ImportPage;
use pages::inventory::InventoryPage;
use pages::movement::MovementPage;
use pages::reports::ReportsPage;
use pages::settings::SettingsPage;
use pages::stock_card::StockCardPage;
use pages::sync::SyncPage;
use pages::transactions::TransactionsPage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Inventory,
    StockIn,
    StockOut,
    Transactions,
    StockCard,
    Reports,
    Import,
    Sync,
    Settings,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::Inventory => "Inventory",
            Page::StockIn => "Stock In",
            Page::StockOut => "Stock Out",
            Page::Transactions => "Transactions",
            Page::StockCard => "Stock Card",
            Page::Reports => "Reports",
            Page::Import => "Import Excel",
            Page::Sync => "Sync",
            Page::Settings => "Settings",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Page::Home => "⌂",
            Page::Inventory => "▤",
            Page::StockIn => "↓",
            Page::StockOut => "↑",
            Page::Transactions => "≡",
            Page::StockCard => "▦",
            Page::Reports => "⎙",
            Page::Import => "⇪",
            Page::Sync => "⇄",
            Page::Settings => "⚙",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub id: u32,
}

/// App-wide state, shared through Leptos context. All fields are `Copy` signals.
#[derive(Clone, Copy)]
pub struct Ctx {
    pub page: RwSignal<Page>,
    pub toast: RwSignal<Option<Toast>>,
    pub device: RwSignal<Option<DeviceInfo>>,
    pub settings: RwSignal<Settings>,
    /// Bumped after every change so lists reload.
    pub refresh: RwSignal<u32>,
    /// Item to preselect when opening Stock In / Out / Stock Card.
    pub focus_item: RwSignal<Option<String>>,
}

impl Ctx {
    pub fn ok(&self, text: impl Into<String>) {
        self.show(text.into(), false);
    }
    pub fn err(&self, text: impl Into<String>) {
        self.show(text.into(), true);
    }
    fn show(&self, text: String, error: bool) {
        let id = (js_sys::Math::random() * 4_000_000_000.0) as u32;
        self.toast.set(Some(Toast { text, error, id }));
        let toast = self.toast;
        let ms = if error { 6000 } else { 3000 };
        set_timeout(
            move || {
                if toast.get_untracked().map(|t| t.id) == Some(id) {
                    toast.set(None);
                }
            },
            std::time::Duration::from_millis(ms),
        );
    }
    pub fn changed(&self) {
        self.refresh.update(|n| *n = n.wrapping_add(1));
    }
    pub fn go(&self, page: Page) {
        self.page.set(page);
    }
    pub fn is_hub(&self) -> bool {
        self.device.get_untracked().map(|d| d.is_hub).unwrap_or(true)
    }
    pub fn open_movement(&self, kind: MovementKind, item_id: Option<String>) {
        self.focus_item.set(item_id);
        self.go(if kind == MovementKind::In { Page::StockIn } else { Page::StockOut });
    }
}

pub fn ctx() -> Ctx {
    expect_context::<Ctx>()
}

const SIDE: [Page; 10] = [
    Page::Home,
    Page::Inventory,
    Page::StockIn,
    Page::StockOut,
    Page::Transactions,
    Page::StockCard,
    Page::Reports,
    Page::Import,
    Page::Sync,
    Page::Settings,
];
const BOTTOM: [Page; 5] = [Page::Home, Page::Inventory, Page::StockIn, Page::StockOut, Page::Sync];

#[component]
fn App() -> impl IntoView {
    let ctx = Ctx {
        page: RwSignal::new(Page::Home),
        toast: RwSignal::new(None),
        device: RwSignal::new(None),
        settings: RwSignal::new(Settings::default()),
        refresh: RwSignal::new(0),
        focus_item: RwSignal::new(None),
    };
    provide_context(ctx);
    let menu_open = RwSignal::new(false);

    spawn_local(async move {
        if let Ok(d) = api::device_info().await {
            ctx.device.set(Some(d));
        }
        if let Ok(s) = api::get_settings().await {
            ctx.settings.set(s);
        }
    });

    let nav_button = move |p: Page| {
        view! {
            <button
                class="nav-item"
                class:active=move || ctx.page.get() == p
                on:click=move |_| { ctx.go(p); menu_open.set(false); }
            >
                <span class="nav-icon">{p.icon()}</span>
                <span class="nav-label">{p.label()}</span>
            </button>
        }
    };

    view! {
        <div class="shell">
            <aside class="sidebar" class:open=move || menu_open.get()>
                <div class="brand">
                    <div class="brand-mark">"W"</div>
                    <div>
                        <div class="brand-name">"Warehouse"</div>
                        <div class="brand-sub">
                            {move || {
                                let s = ctx.settings.get();
                                if s.warehouse_name.is_empty() { "Inventory".to_string() } else { s.warehouse_name }
                            }}
                        </div>
                    </div>
                </div>
                <nav class="side-nav">{SIDE.into_iter().map(nav_button).collect_view()}</nav>
                <div class="device-tag">
                    {move || ctx.device.get().map(|d| format!(
                        "{} · {}", d.device_name, if d.is_hub { "PC hub" } else { "Phone" }
                    ))}
                </div>
            </aside>
            <div class="scrim" class:open=move || menu_open.get() on:click=move |_| menu_open.set(false)></div>

            <main class="main">
                <header class="topbar">
                    <button class="icon-btn menu-btn" on:click=move |_| menu_open.update(|o| *o = !*o)>"☰"</button>
                    <h1>{move || ctx.page.get().label()}</h1>
                </header>
                <div class="content">
                    {move || match ctx.page.get() {
                        Page::Home => view! { <HomePage/> }.into_any(),
                        Page::Inventory => view! { <InventoryPage/> }.into_any(),
                        Page::StockIn => view! { <MovementPage kind=MovementKind::In/> }.into_any(),
                        Page::StockOut => view! { <MovementPage kind=MovementKind::Out/> }.into_any(),
                        Page::Transactions => view! { <TransactionsPage/> }.into_any(),
                        Page::StockCard => view! { <StockCardPage/> }.into_any(),
                        Page::Reports => view! { <ReportsPage/> }.into_any(),
                        Page::Import => view! { <ImportPage/> }.into_any(),
                        Page::Sync => view! { <SyncPage/> }.into_any(),
                        Page::Settings => view! { <SettingsPage/> }.into_any(),
                    }}
                </div>
            </main>

            <nav class="bottom-nav">{BOTTOM.into_iter().map(nav_button).collect_view()}</nav>

            {move || ctx.toast.get().map(|t| view! {
                <div class="toast" class:error=t.error on:click=move |_| ctx.toast.set(None)>{t.text}</div>
            })}
        </div>
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
