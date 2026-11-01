use inventory_types::MovementKind;

/// `12` → "12", `2.5` → "2.5", `1234.125` → "1,234.125"
pub fn fq(q: f64) -> String {
    let m = (q * 1000.0).round() as i64;
    let neg = m < 0;
    let m = m.unsigned_abs();
    let (whole, frac) = (m / 1000, m % 1000);
    let digits = whole.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if frac != 0 {
        out.push('.');
        out.push_str(format!("{frac:03}").trim_end_matches('0'));
    }
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

pub fn today() -> String {
    let d = js_sys::Date::new_0();
    format!("{:04}-{:02}-{:02}", d.get_full_year(), d.get_month() + 1, d.get_date())
}

/// First day of the current month.
pub fn month_start() -> String {
    let d = js_sys::Date::new_0();
    format!("{:04}-{:02}-01", d.get_full_year(), d.get_month() + 1)
}

pub fn parse_qty(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', "");
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

pub fn opt(s: String) -> Option<String> {
    let t = s.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

pub fn kind_label(k: MovementKind) -> &'static str {
    match k {
        MovementKind::In => "Stock In",
        MovementKind::Out => "Stock Out",
    }
}
