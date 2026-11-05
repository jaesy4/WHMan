//! Shared data types. No logic, no heavy dependencies — this crate is used by
//! the native core library *and* the WASM user interface.
//!
//! Quantities are `f64` at this boundary (so the UI can show `2.5 kg`), but the
//! database stores them as integer thousandths to avoid rounding drift.
//! Dates are ISO strings: `YYYY-MM-DD`.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- items

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub code: String,
    pub name: String,
    pub unit: String,
    pub location: String,
    pub reorder_level: f64,
    pub remarks: String,
}

/// Used for both "create" and "edit" forms.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemInput {
    pub code: String,
    pub name: String,
    pub unit: String,
    pub location: String,
    pub reorder_level: f64,
    pub remarks: String,
}

/// One line of the inventory list: an item plus its In / Out totals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StockRow {
    pub item: Item,
    pub total_in: f64,
    pub total_out: f64,
    pub balance: f64,
    /// `balance <= reorder_level` (and reorder level is set).
    pub low: bool,
}

// ------------------------------------------------------------ movements

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MovementKind {
    In,
    Out,
}

impl MovementKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MovementKind::In => "IN",
            MovementKind::Out => "OUT",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "IN" | "STOCK IN" | "RECEIVE" | "RECEIVED" | "R" | "I" => Some(MovementKind::In),
            "OUT" | "STOCK OUT" | "ISSUE" | "ISSUED" | "O" => Some(MovementKind::Out),
            _ => None,
        }
    }
}

/// A Stock In or Stock Out record as entered by the warehouse man.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovementInput {
    pub item_id: String,
    pub kind: MovementKind,
    pub qty: f64,
    /// `YYYY-MM-DD`
    pub date: String,
    /// Delivery receipt no., requisition slip no., etc.
    pub reference: String,
    /// "Received from" for IN, "Issued to" for OUT.
    pub party: String,
    pub remarks: String,
    pub recorded_by: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Movement {
    pub id: String,
    pub item_id: String,
    pub item_code: String,
    pub item_name: String,
    pub unit: String,
    pub kind: MovementKind,
    pub qty: f64,
    pub date: String,
    pub reference: String,
    pub party: String,
    pub remarks: String,
    pub recorded_by: String,
    /// Voided records are kept for audit but excluded from balances.
    pub voided: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MovementFilter {
    pub from: Option<String>,
    pub to: Option<String>,
    pub kind: Option<MovementKind>,
    pub item_id: Option<String>,
    pub search: Option<String>,
    pub include_voided: bool,
    pub limit: Option<u32>,
}

// -------------------------------------------------------------- reports

/// One row of the In/Out summary for a date range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryRow {
    pub item_id: String,
    pub code: String,
    pub name: String,
    pub unit: String,
    pub beginning: f64,
    pub total_in: f64,
    pub total_out: f64,
    pub ending: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StockCardLine {
    pub date: String,
    pub reference: String,
    pub party: String,
    pub qty_in: f64,
    pub qty_out: f64,
    pub balance: f64,
    pub remarks: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StockCard {
    pub item: Item,
    pub from: Option<String>,
    pub to: Option<String>,
    pub beginning: f64,
    pub lines: Vec<StockCardLine>,
    pub ending: f64,
}

// --------------------------------------------------------------- import

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportKind {
    /// Item list (+ optional beginning balance).
    Items,
    /// Stock In / Stock Out records.
    Movements,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportIssue {
    /// Excel row number as shown in Excel (1-based, header is row 1).
    pub row: u32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportPreview {
    pub kind: ImportKind,
    pub sheet: String,
    pub file_name: String,
    pub valid_rows: u32,
    pub new_items: u32,
    pub updated_items: u32,
    pub movements: u32,
    pub errors: Vec<ImportIssue>,
    pub warnings: Vec<ImportIssue>,
    /// First rows, already cleaned, for display (column headers + values).
    pub sample_headers: Vec<String>,
    pub sample: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportResult {
    pub items_created: u32,
    pub items_updated: u32,
    pub movements_created: u32,
}

// ----------------------------------------------------------------- sync

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub device_name: String,
    /// True on the Windows build (runs the sync server).
    pub is_hub: bool,
}

/// What the desktop shows as a QR code while pairing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairingOffer {
    /// JSON text encoded in the QR code (also shown for manual copy).
    pub payload: String,
    /// SVG markup of the QR code.
    pub qr_svg: String,
    pub host: String,
    pub port: u16,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: String,
    pub name: String,
    pub address: String,
    pub last_sync_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncReport {
    pub pushed: u32,
    pub pulled: u32,
    pub applied: u32,
    pub address: String,
    pub finished_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
    pub item_count: u32,
    pub low_stock_count: u32,
    pub today_in: f64,
    pub today_out: f64,
    pub today_in_count: u32,
    pub today_out_count: u32,
}

// ------------------------------------------------------------ settings

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Printed on every report, e.g. "ABC Hardware - Main Warehouse".
    pub warehouse_name: String,
    /// Default "Recorded by" for new IN/OUT records.
    pub recorded_by: String,
    pub device_name: String,
}

// ------------------------------------------------------------- exports

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportKind {
    /// Current balance of all items.
    Inventory,
    /// Beginning / In / Out / Ending for a period.
    Summary,
    /// Stock In / Stock Out transaction list.
    Movements,
    /// One item with running balance (needs `item_id`).
    StockCard,
    /// Receiving / Issue slip for one record (needs `movement_id`, PDF only).
    Slip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileFormat {
    Pdf,
    Xlsx,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportRequest {
    pub report: ReportKind,
    pub format: FileFormat,
    pub from: Option<String>,
    pub to: Option<String>,
    pub item_id: Option<String>,
    pub movement_id: Option<String>,
    pub kind: Option<MovementKind>,
    pub low_only: bool,
}

// ------------------------------------------------- department requests

/// A department that can request items from the warehouse
/// (e.g. "Engineering", "Solar Team").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Department {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserRole {
    /// Can create requests for their department.
    Requester,
    /// Can create requests and approve / reject their department's requests.
    Head,
}

impl UserRole {
    pub fn as_str(self) -> &'static str {
        match self {
            UserRole::Requester => "requester",
            UserRole::Head => "head",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "requester" => Some(UserRole::Requester),
            "head" => Some(UserRole::Head),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            UserRole::Requester => "Requester",
            UserRole::Head => "Department Head",
        }
    }
}

/// A login for the department web portal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebUser {
    pub id: String,
    pub username: String,
    pub full_name: String,
    pub department_id: String,
    pub department_name: String,
    pub role: UserRole,
    pub active: bool,
}

/// Create / edit form for a web user. `password` empty on edit = keep it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebUserInput {
    pub username: String,
    pub full_name: String,
    pub department_id: String,
    pub role: UserRole,
    pub password: String,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestStatus {
    /// Sent by the requester, waiting for the department head.
    Submitted,
    /// Approved, waiting for the warehouse to issue.
    Approved,
    Rejected,
    /// Cancelled by the requester before it was issued.
    Cancelled,
    /// Warehouse released the items (Stock Out records were created).
    Issued,
    /// Requester confirmed they received the items.
    Received,
}

impl RequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RequestStatus::Submitted => "SUBMITTED",
            RequestStatus::Approved => "APPROVED",
            RequestStatus::Rejected => "REJECTED",
            RequestStatus::Cancelled => "CANCELLED",
            RequestStatus::Issued => "ISSUED",
            RequestStatus::Received => "RECEIVED",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "SUBMITTED" => Some(RequestStatus::Submitted),
            "APPROVED" => Some(RequestStatus::Approved),
            "REJECTED" => Some(RequestStatus::Rejected),
            "CANCELLED" => Some(RequestStatus::Cancelled),
            "ISSUED" => Some(RequestStatus::Issued),
            "RECEIVED" => Some(RequestStatus::Received),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            RequestStatus::Submitted => "Waiting approval",
            RequestStatus::Approved => "Approved",
            RequestStatus::Rejected => "Rejected",
            RequestStatus::Cancelled => "Cancelled",
            RequestStatus::Issued => "Issued",
            RequestStatus::Received => "Received",
        }
    }
    /// Still needs action from someone.
    pub fn is_open(self) -> bool {
        matches!(self, RequestStatus::Submitted | RequestStatus::Approved)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestLine {
    pub id: String,
    pub item_id: String,
    pub item_code: String,
    pub item_name: String,
    pub unit: String,
    pub qty_requested: f64,
    /// Filled in when issued (may be less than requested).
    pub qty_issued: f64,
    pub remarks: String,
    /// Current warehouse balance of the item (for display).
    pub balance: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemRequest {
    pub id: String,
    /// Human number, e.g. "RQ-2026-0007".
    pub number: String,
    pub department_id: String,
    pub department_name: String,
    pub requested_by: String,
    pub purpose: String,
    /// `YYYY-MM-DD` or empty.
    pub needed_by: String,
    pub status: RequestStatus,
    /// "YYYY-MM-DD HH:MM" (PC local time)
    pub created_at: String,
    pub approved_by: String,
    pub approved_at: String,
    /// Reason for rejecting / cancelling, or a note from the approver.
    pub decision_note: String,
    pub issued_by: String,
    pub issued_at: String,
    pub received_by: String,
    pub received_at: String,
    pub lines: Vec<RequestLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestLineInput {
    pub item_id: String,
    pub qty: f64,
    pub remarks: String,
}

/// What a requester sends from the web portal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestInput {
    pub purpose: String,
    pub needed_by: String,
    pub lines: Vec<RequestLineInput>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequestFilter {
    pub status: Option<RequestStatus>,
    pub department_id: Option<String>,
    /// Only Submitted / Approved.
    pub open_only: bool,
    pub search: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueLineInput {
    pub line_id: String,
    pub qty: f64,
}

/// Warehouse releases an approved request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueInput {
    pub request_id: String,
    pub lines: Vec<IssueLineInput>,
    /// `YYYY-MM-DD`
    pub date: String,
    pub issued_by: String,
    pub remarks: String,
}

/// Status of the department web portal, shown on the PC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortalInfo {
    pub running: bool,
    pub port: u16,
    /// e.g. "http://192.168.1.10:47812" (same Wi-Fi only)
    pub lan_url: String,
    /// The Tailscale Funnel address entered in Settings, if any.
    pub public_url: String,
    pub open_requests: u32,
}
