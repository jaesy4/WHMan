//! Tauri shell: exposes `inventory_core` to the Leptos UI as commands.
//! Same code for Windows (hub + sync server) and Android (sync client).

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use inventory_core::excel::{self, ParsedImport};
use inventory_core::pdf::{self, ReportMeta};
use inventory_core::types::*;
use inventory_core::Db;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_fs::{FsExt, OpenOptions};

#[cfg(desktop)]
use inventory_core::portal::{self as web_portal, Portal};
#[cfg(desktop)]
use inventory_core::sync::server::{self, Hub};
use inventory_core::Actor;

struct AppState {
    db: Arc<Db>,
    #[cfg(desktop)]
    hub: Arc<Hub>,
    #[cfg(desktop)]
    portal: Arc<Portal>,
    pending_import: Mutex<Option<ParsedImport>>,
}

type CmdResult<T> = Result<T, String>;

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

// ------------------------------------------------------------ helpers

fn settings(db: &Db) -> Settings {
    Settings {
        warehouse_name: db.setting("warehouse_name").ok().flatten().unwrap_or_default(),
        recorded_by: db.setting("recorded_by").ok().flatten().unwrap_or_default(),
        device_name: db.device_name().unwrap_or_default(),
    }
}

fn report_meta(db: &Db) -> ReportMeta {
    let s = settings(db);
    ReportMeta {
        org: s.warehouse_name,
        generated: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        prepared_by: s.recorded_by,
    }
}

fn read_file(app: &AppHandle, path: &FilePath) -> CmdResult<Vec<u8>> {
    if let Some(p) = path.as_path() {
        return std::fs::read(p).map_err(e);
    }
    // Android content:// URI
    let mut opts = OpenOptions::new();
    opts.read(true);
    let mut f = app.fs().open(path.clone(), opts).map_err(e)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(e)?;
    Ok(buf)
}

fn write_file(app: &AppHandle, path: &FilePath, bytes: &[u8]) -> CmdResult<String> {
    if let Some(p) = path.as_path() {
        std::fs::write(p, bytes).map_err(e)?;
        return Ok(p.display().to_string());
    }
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    let mut f = app.fs().open(path.clone(), opts).map_err(e)?;
    f.write_all(bytes).map_err(e)?;
    Ok(path.to_string())
}

fn file_name_of(path: &FilePath) -> String {
    path.as_path()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file.xlsx".to_string())
}

/// Shows a Save dialog and writes the bytes. `None` = user cancelled.
fn save_with_dialog(app: &AppHandle, default_name: &str, format: FileFormat, bytes: &[u8]) -> CmdResult<Option<String>> {
    let (label, ext) = match format {
        FileFormat::Pdf => ("PDF document", "pdf"),
        FileFormat::Xlsx => ("Excel workbook", "xlsx"),
    };
    let picked = app
        .dialog()
        .file()
        .set_file_name(default_name)
        .add_filter(label, &[ext])
        .blocking_save_file();
    match picked {
        Some(path) => write_file(app, &path, bytes).map(Some),
        None => Ok(None),
    }
}

fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

// ------------------------------------------------------------ commands

#[tauri::command(rename_all = "snake_case")]
async fn device_info(state: State<'_, AppState>) -> CmdResult<DeviceInfo> {
    Ok(DeviceInfo {
        device_id: state.db.device_id().map_err(e)?,
        device_name: state.db.device_name().map_err(e)?,
        is_hub: cfg!(desktop),
    })
}

#[tauri::command(rename_all = "snake_case")]
async fn get_settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    Ok(settings(&state.db))
}

#[tauri::command(rename_all = "snake_case")]
async fn save_settings(state: State<'_, AppState>, settings: Settings) -> CmdResult<()> {
    let db = &state.db;
    db.set_setting("warehouse_name", settings.warehouse_name.trim()).map_err(e)?;
    db.set_setting("recorded_by", settings.recorded_by.trim()).map_err(e)?;
    if !settings.device_name.trim().is_empty() {
        db.set_device_name(&settings.device_name).map_err(e)?;
    }
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
async fn dashboard(state: State<'_, AppState>) -> CmdResult<Dashboard> {
    state.db.dashboard(&inventory_core::today()).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn stock_list(state: State<'_, AppState>, search: String, low_only: bool) -> CmdResult<Vec<StockRow>> {
    state.db.stock_list(&search, low_only).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn create_item(state: State<'_, AppState>, input: ItemInput) -> CmdResult<Item> {
    state.db.create_item(&input).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn update_item(state: State<'_, AppState>, id: String, input: ItemInput) -> CmdResult<Item> {
    state.db.update_item(&id, &input).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn delete_item(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.db.delete_item(&id).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn record_movement(state: State<'_, AppState>, input: MovementInput) -> CmdResult<Movement> {
    state.db.record_movement(&input).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn void_movement(state: State<'_, AppState>, id: String, reason: String) -> CmdResult<Movement> {
    state.db.void_movement(&id, &reason).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn list_movements(state: State<'_, AppState>, filter: MovementFilter) -> CmdResult<Vec<Movement>> {
    state.db.list_movements(&filter).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn stock_card(
    state: State<'_, AppState>,
    item_id: String,
    from: Option<String>,
    to: Option<String>,
) -> CmdResult<StockCard> {
    state.db.stock_card(&item_id, from.as_deref(), to.as_deref()).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn inout_summary(state: State<'_, AppState>, from: Option<String>, to: Option<String>) -> CmdResult<Vec<SummaryRow>> {
    state.db.inout_summary(from.as_deref(), to.as_deref()).map_err(e)
}

// ---- import

/// Opens a file picker, reads the Excel file and returns a preview.
/// `None` = user cancelled.
#[tauri::command(rename_all = "snake_case")]
async fn import_pick_preview(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<ImportPreview>> {
    let Some(path) = app
        .dialog()
        .file()
        .add_filter("Excel", &["xlsx", "xlsm", "xls", "ods"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let bytes = read_file(&app, &path)?;
    let parsed = excel::parse(&state.db, &bytes, &file_name_of(&path)).map_err(e)?;
    let preview = parsed.preview.clone();
    *state.pending_import.lock().map_err(e)? = Some(parsed);
    Ok(Some(preview))
}

#[tauri::command(rename_all = "snake_case")]
async fn import_commit(state: State<'_, AppState>) -> CmdResult<ImportResult> {
    let parsed = state
        .pending_import
        .lock()
        .map_err(e)?
        .take()
        .ok_or_else(|| "Nothing to import. Choose a file first.".to_string())?;
    let who = settings(&state.db).recorded_by;
    state.db.commit_import(&parsed, &who).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn import_cancel(state: State<'_, AppState>) -> CmdResult<()> {
    *state.pending_import.lock().map_err(e)? = None;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
async fn save_template(app: AppHandle, kind: ImportKind) -> CmdResult<Option<String>> {
    let bytes = excel::template(kind).map_err(e)?;
    let name = match kind {
        ImportKind::Items => "Item_List_Template.xlsx",
        ImportKind::Movements => "Stock_In_Out_Template.xlsx",
    };
    save_with_dialog(&app, name, FileFormat::Xlsx, &bytes)
}

// ---- export

#[tauri::command(rename_all = "snake_case")]
async fn export_report(app: AppHandle, state: State<'_, AppState>, request: ReportRequest) -> CmdResult<Option<String>> {
    let db = &state.db;
    let meta = report_meta(db);
    let today = inventory_core::today();
    let from = request.from.as_deref().filter(|s| !s.is_empty());
    let to = request.to.as_deref().filter(|s| !s.is_empty());
    let range = match (from, to) {
        (Some(f), Some(t)) => format!("_{f}_to_{t}"),
        (Some(f), None) => format!("_from_{f}"),
        (None, Some(t)) => format!("_to_{t}"),
        (None, None) => String::new(),
    };
    let fmt = request.format;
    let ext = if fmt == FileFormat::Pdf { "pdf" } else { "xlsx" };

    let (bytes, name) = match request.report {
        ReportKind::Inventory => {
            let rows = db.stock_list("", request.low_only).map_err(e)?;
            let bytes = match fmt {
                FileFormat::Pdf => pdf::inventory_report(&meta, &rows, &today),
                FileFormat::Xlsx => excel::export_inventory(&meta.org, &rows, &today).map_err(e)?,
            };
            let base = if request.low_only { "Low_Stock" } else { "Inventory" };
            (bytes, format!("{base}_{today}.{ext}"))
        }
        ReportKind::Summary => {
            let rows = db.inout_summary(from, to).map_err(e)?;
            let bytes = match fmt {
                FileFormat::Pdf => pdf::summary_report(&meta, &rows, from, to),
                FileFormat::Xlsx => excel::export_summary(&meta.org, &rows, from, to).map_err(e)?,
            };
            (bytes, format!("In_Out_Summary{range}.{ext}"))
        }
        ReportKind::Movements => {
            let filter = MovementFilter {
                from: from.map(str::to_string),
                to: to.map(str::to_string),
                kind: request.kind,
                include_voided: true,
                ..Default::default()
            };
            let mut rows = db.list_movements(&filter).map_err(e)?;
            rows.reverse(); // oldest first on paper
            let bytes = match fmt {
                FileFormat::Pdf => pdf::movements_report(&meta, &rows, request.kind, from, to),
                FileFormat::Xlsx => excel::export_movements(&meta.org, &rows, from, to).map_err(e)?,
            };
            let base = match request.kind {
                Some(MovementKind::In) => "Stock_In",
                Some(MovementKind::Out) => "Stock_Out",
                None => "Transactions",
            };
            (bytes, format!("{base}{range}.{ext}"))
        }
        ReportKind::StockCard => {
            let id = request.item_id.as_deref().ok_or("Choose an item first.")?;
            let card = db.stock_card(id, from, to).map_err(e)?;
            let bytes = match fmt {
                FileFormat::Pdf => pdf::stock_card_report(&meta, &card),
                FileFormat::Xlsx => excel::export_stock_card(&meta.org, &card).map_err(e)?,
            };
            (bytes, format!("Stock_Card_{}{range}.{ext}", safe_name(&card.item.code)))
        }
        ReportKind::Slip => {
            let id = request.movement_id.as_deref().ok_or("Choose a record first.")?;
            let m = db.get_movement(id).map_err(e)?;
            let bal = db.item_balance(&m.item_id).map_err(e)?;
            let prefix = if m.kind == MovementKind::In { "Receiving_Slip" } else { "Issue_Slip" };
            let refn = if m.reference.is_empty() { m.date.clone() } else { safe_name(&m.reference) };
            (pdf::movement_slip(&meta, &m, bal), format!("{prefix}_{refn}.pdf"))
        }
    };
    let fmt = if request.report == ReportKind::Slip { FileFormat::Pdf } else { fmt };
    save_with_dialog(&app, &name, fmt, &bytes)
}

// ---- sync

#[tauri::command(rename_all = "snake_case")]
async fn start_pairing(state: State<'_, AppState>) -> CmdResult<PairingOffer> {
    #[cfg(desktop)]
    {
        state.hub.start_pairing().map_err(e)
    }
    #[cfg(mobile)]
    {
        let _ = state;
        Err("Pairing is started from the PC app.".into())
    }
}

#[tauri::command(rename_all = "snake_case")]
async fn cancel_pairing(state: State<'_, AppState>) -> CmdResult<()> {
    #[cfg(desktop)]
    state.hub.cancel_pairing();
    let _ = &state;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
async fn pair_with_hub(state: State<'_, AppState>, qr_text: String) -> CmdResult<PeerInfo> {
    #[cfg(mobile)]
    {
        inventory_core::sync::client::pair(state.db.clone(), &qr_text).await.map_err(e)
    }
    #[cfg(desktop)]
    {
        let _ = (state, qr_text);
        Err("The PC is the hub. Scan the PC's QR code from the phone app.".into())
    }
}

#[tauri::command(rename_all = "snake_case")]
async fn sync_now(state: State<'_, AppState>) -> CmdResult<SyncReport> {
    #[cfg(mobile)]
    {
        inventory_core::sync::client::sync_with_hub(state.db.clone()).await.map_err(e)
    }
    #[cfg(desktop)]
    {
        let _ = state;
        Err("Sync is started from the phone. Keep this app open on the PC.".into())
    }
}

#[tauri::command(rename_all = "snake_case")]
async fn list_peers(state: State<'_, AppState>) -> CmdResult<Vec<PeerInfo>> {
    state.db.list_peers().map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn remove_peer(state: State<'_, AppState>, peer_id: String) -> CmdResult<()> {
    state.db.remove_peer(&peer_id).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn hub_address() -> CmdResult<String> {
    #[cfg(desktop)]
    {
        let ip = inventory_core::sync::discovery::local_ip()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "not connected".into());
        Ok(format!("{ip}:{}", inventory_core::sync::DEFAULT_PORT))
    }
    #[cfg(mobile)]
    {
        Ok(String::new())
    }
}

// ---- department requests (web portal)

#[tauri::command(rename_all = "snake_case")]
async fn list_requests(state: State<'_, AppState>, filter: RequestFilter) -> CmdResult<Vec<ItemRequest>> {
    state.db.list_requests(&filter).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn approve_request(state: State<'_, AppState>, id: String, note: String) -> CmdResult<ItemRequest> {
    let who = settings(&state.db).recorded_by;
    state.db.approve_request(&id, Actor::Warehouse(&who), &note).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn reject_request(state: State<'_, AppState>, id: String, reason: String) -> CmdResult<ItemRequest> {
    let who = settings(&state.db).recorded_by;
    state.db.reject_request(&id, Actor::Warehouse(&who), &reason).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn issue_request(state: State<'_, AppState>, input: IssueInput) -> CmdResult<ItemRequest> {
    state.db.issue_request(&input).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn list_departments(state: State<'_, AppState>) -> CmdResult<Vec<Department>> {
    state.db.list_departments().map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn list_web_users(state: State<'_, AppState>) -> CmdResult<Vec<WebUser>> {
    state.db.list_web_users().map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn save_web_user(state: State<'_, AppState>, id: Option<String>, input: WebUserInput) -> CmdResult<WebUser> {
    let db = state.db.clone();
    // Password hashing takes a moment on purpose: keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || db.save_web_user(id.as_deref(), &input))
        .await
        .map_err(e)?
        .map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn delete_web_user(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.db.delete_web_user(&id).map_err(e)
}

#[tauri::command(rename_all = "snake_case")]
async fn portal_info(state: State<'_, AppState>) -> CmdResult<PortalInfo> {
    let public_url = state.db.setting("portal_public_url").map_err(e)?.unwrap_or_default();
    let open_requests = state.db.open_request_count().map_err(e)?;
    #[cfg(desktop)]
    {
        let port = state.portal.port();
        let lan_url = inventory_core::sync::discovery::local_ip()
            .map(|ip| format!("http://{ip}:{port}"))
            .unwrap_or_default();
        Ok(PortalInfo { running: state.portal.is_running(), port, lan_url, public_url, open_requests })
    }
    #[cfg(mobile)]
    {
        Ok(PortalInfo { running: false, port: 0, lan_url: String::new(), public_url, open_requests })
    }
}

#[tauri::command(rename_all = "snake_case")]
async fn save_portal_url(state: State<'_, AppState>, url: String) -> CmdResult<()> {
    let url = url.trim().trim_end_matches('/');
    if !url.is_empty() && !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("The link must start with https://".into());
    }
    state.db.set_setting("portal_public_url", url).map_err(e)
}

// ------------------------------------------------------------ entry

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init());
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());

    builder
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let db = Arc::new(Db::open(dir.join("inventory.db"))?);

            #[cfg(desktop)]
            let hub = {
                let hub = Hub::new(db.clone(), inventory_core::sync::DEFAULT_PORT);
                let h = hub.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = server::serve(h).await {
                        eprintln!("[sync] server stopped: {err}");
                    }
                });
                hub
            };

            // Department web portal (put online with Tailscale Funnel).
            #[cfg(desktop)]
            let portal = {
                let portal = Portal::new(db.clone(), web_portal::PORTAL_PORT);
                let p = portal.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = web_portal::serve(p).await {
                        eprintln!("[portal] server stopped: {err}");
                    }
                });
                portal
            };

            app.manage(AppState {
                db,
                #[cfg(desktop)]
                hub,
                #[cfg(desktop)]
                portal,
                pending_import: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            device_info,
            get_settings,
            save_settings,
            dashboard,
            stock_list,
            create_item,
            update_item,
            delete_item,
            record_movement,
            void_movement,
            list_movements,
            stock_card,
            inout_summary,
            import_pick_preview,
            import_commit,
            import_cancel,
            save_template,
            export_report,
            start_pairing,
            cancel_pairing,
            pair_with_hub,
            sync_now,
            list_peers,
            remove_peer,
            hub_address,
            list_requests,
            approve_request,
            reject_request,
            issue_request,
            list_departments,
            list_web_users,
            save_web_user,
            delete_web_user,
            portal_info,
            save_portal_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
