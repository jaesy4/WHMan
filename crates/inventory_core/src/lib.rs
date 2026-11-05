//! # inventory_core
//!
//! Warehouse In/Out inventory engine shared by the Windows and Android apps.
//!
//! * [`Db`] — SQLite storage (items, Stock In / Stock Out records, reports)
//! * [`excel`] — import item lists / movements, export reports to `.xlsx`
//! * [`pdf`] — printable reports and IN/OUT slips (no external PDF library)
//! * [`sync`] — phone ⇄ desktop sync over local Wi-Fi
//! * department requests and web-portal logins (`Db::create_request`, `Db::web_login`, ...)
//! * `portal` — the department web portal server (desktop, feature `server`)

mod db;
mod error;
mod items;
mod movements;
mod reports;
mod requests;
mod web_users;

pub mod excel;
pub mod pdf;
pub mod sync;

#[cfg(feature = "server")]
pub mod portal;

pub use db::Db;
pub use requests::Actor;
pub use web_users::MIN_PASSWORD_LEN;
pub use error::{Error, Result};
pub use inventory_types as types;

/// Quantities are stored as integer thousandths (`2.5 kg` → `2500`) so sums
/// never drift the way floating point does.
pub mod qty {
    pub fn to_milli(q: f64) -> i64 {
        (q * 1000.0).round() as i64
    }

    pub fn from_milli(m: i64) -> f64 {
        m as f64 / 1000.0
    }

    /// `12` → "12", `2.5` → "2.5", `1234.125` → "1,234.125"
    pub fn format(q: f64) -> String {
        let m = to_milli(q);
        let neg = m < 0;
        let m = m.unsigned_abs();
        let whole = m / 1000;
        let frac = m % 1000;
        let mut digits = whole.to_string();
        let mut grouped = String::new();
        while digits.len() > 3 {
            let tail = digits.split_off(digits.len() - 3);
            grouped = format!(",{tail}{grouped}");
        }
        let mut s = format!("{}{digits}{grouped}", if neg { "-" } else { "" });
        if frac != 0 {
            let f = format!("{frac:03}");
            s.push('.');
            s.push_str(f.trim_end_matches('0'));
        }
        s
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn formats() {
            assert_eq!(format(0.0), "0");
            assert_eq!(format(12.0), "12");
            assert_eq!(format(2.5), "2.5");
            assert_eq!(format(1234567.125), "1,234,567.125");
            assert_eq!(format(-1500.0), "-1,500");
            assert_eq!(to_milli(0.1) + to_milli(0.2), to_milli(0.3));
        }
    }
}

/// Today's date as `YYYY-MM-DD` (local time of the device).
pub fn today() -> String {
    chrono::Local::now().date_naive().format("%Y-%m-%d").to_string()
}

pub(crate) fn valid_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}
