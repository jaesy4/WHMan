//! Calls into the Rust backend (Tauri commands) from WebAssembly.

use inventory_types::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

fn js_err(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| {
        js_sys::JSON::stringify(&e).ok().and_then(|s| s.as_string()).unwrap_or_else(|| "Unknown error".into())
    })
}

async fn raw<A: Serialize>(cmd: &str, args: &A) -> Result<JsValue, String> {
    let args = args
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| e.to_string())?;
    invoke(cmd, args).await.map_err(js_err)
}

async fn call<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let v = raw(cmd, args).await?;
    serde_wasm_bindgen::from_value(v).map_err(|e| format!("{cmd}: {e}"))
}

async fn call_unit<A: Serialize>(cmd: &str, args: &A) -> Result<(), String> {
    raw(cmd, args).await.map(|_| ())
}

fn none() -> serde_json::Value {
    json!({})
}

pub async fn device_info() -> Result<DeviceInfo, String> {
    call("device_info", &none()).await
}
pub async fn get_settings() -> Result<Settings, String> {
    call("get_settings", &none()).await
}
pub async fn save_settings(settings: &Settings) -> Result<(), String> {
    call_unit("save_settings", &json!({ "settings": settings })).await
}
pub async fn dashboard() -> Result<Dashboard, String> {
    call("dashboard", &none()).await
}
pub async fn stock_list(search: &str, low_only: bool) -> Result<Vec<StockRow>, String> {
    call("stock_list", &json!({ "search": search, "low_only": low_only })).await
}
pub async fn create_item(input: &ItemInput) -> Result<Item, String> {
    call("create_item", &json!({ "input": input })).await
}
pub async fn update_item(id: &str, input: &ItemInput) -> Result<Item, String> {
    call("update_item", &json!({ "id": id, "input": input })).await
}
pub async fn delete_item(id: &str) -> Result<(), String> {
    call_unit("delete_item", &json!({ "id": id })).await
}
pub async fn record_movement(input: &MovementInput) -> Result<Movement, String> {
    call("record_movement", &json!({ "input": input })).await
}
pub async fn void_movement(id: &str, reason: &str) -> Result<Movement, String> {
    call("void_movement", &json!({ "id": id, "reason": reason })).await
}
pub async fn list_movements(filter: &MovementFilter) -> Result<Vec<Movement>, String> {
    call("list_movements", &json!({ "filter": filter })).await
}
pub async fn stock_card(item_id: &str, from: Option<String>, to: Option<String>) -> Result<StockCard, String> {
    call("stock_card", &json!({ "item_id": item_id, "from": from, "to": to })).await
}
pub async fn inout_summary(from: Option<String>, to: Option<String>) -> Result<Vec<SummaryRow>, String> {
    call("inout_summary", &json!({ "from": from, "to": to })).await
}
pub async fn import_pick_preview() -> Result<Option<ImportPreview>, String> {
    call("import_pick_preview", &none()).await
}
pub async fn import_commit() -> Result<ImportResult, String> {
    call("import_commit", &none()).await
}
pub async fn import_cancel() -> Result<(), String> {
    call_unit("import_cancel", &none()).await
}
pub async fn save_template(kind: ImportKind) -> Result<Option<String>, String> {
    call("save_template", &json!({ "kind": kind })).await
}
pub async fn export_report(request: &ReportRequest) -> Result<Option<String>, String> {
    call("export_report", &json!({ "request": request })).await
}
pub async fn start_pairing() -> Result<PairingOffer, String> {
    call("start_pairing", &none()).await
}
pub async fn cancel_pairing() -> Result<(), String> {
    call_unit("cancel_pairing", &none()).await
}
pub async fn pair_with_hub(qr_text: &str) -> Result<PeerInfo, String> {
    call("pair_with_hub", &json!({ "qr_text": qr_text })).await
}
pub async fn sync_now() -> Result<SyncReport, String> {
    call("sync_now", &none()).await
}
pub async fn list_peers() -> Result<Vec<PeerInfo>, String> {
    call("list_peers", &none()).await
}
pub async fn remove_peer(peer_id: &str) -> Result<(), String> {
    call_unit("remove_peer", &json!({ "peer_id": peer_id })).await
}
pub async fn hub_address() -> Result<String, String> {
    call("hub_address", &none()).await
}

#[derive(Deserialize)]
struct ScanResult {
    content: String,
}

/// Opens the camera (Android) and returns the QR code text.
pub async fn scan_qr() -> Result<String, String> {
    let r: ScanResult =
        call("plugin:barcode-scanner|scan", &json!({ "windowed": false, "formats": ["QR_CODE"] })).await?;
    Ok(r.content)
}
