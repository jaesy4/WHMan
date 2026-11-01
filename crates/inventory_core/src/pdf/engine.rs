//! A small, dependency-free PDF writer for tabular reports.
//!
//! Uses the built-in PDF fonts (Helvetica / Helvetica-Bold, WinAnsi), so no
//! font files are embedded and the output is tiny. Supports Latin-1 text
//! (e.g. "ñ", "é"); other characters are printed as "?".
//!
//! Layout: title block, optional key/value info block, a table that flows
//! over as many pages as needed (header row repeated), an optional note,
//! signature lines, and "Page X of Y" footers.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone)]
pub struct Col {
    pub header: String,
    /// Relative width (columns share the page width by weight).
    pub weight: f32,
    pub align: Align,
}

impl Col {
    pub fn new(header: &str, weight: f32, align: Align) -> Self {
        Self { header: header.to_string(), weight, align }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStyle {
    Normal,
    /// Totals / emphasis.
    Bold,
    /// Highlighted (e.g. low stock).
    Warn,
    /// Greyed out (e.g. voided records).
    Muted,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub cells: Vec<String>,
    pub style: RowStyle,
}

impl Row {
    pub fn new(cells: Vec<String>) -> Self {
        Self { cells, style: RowStyle::Normal }
    }
    pub fn styled(cells: Vec<String>, style: RowStyle) -> Self {
        Self { cells, style }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Doc {
    /// Small line above the title (warehouse / company name).
    pub org: String,
    pub title: String,
    pub subtitle: Vec<String>,
    /// Label / value pairs shown in two columns under the title.
    pub info: Vec<(String, String)>,
    pub columns: Vec<Col>,
    pub rows: Vec<Row>,
    pub landscape: bool,
    pub note: Option<String>,
    /// e.g. ["Prepared by", "Checked by", "Approved by"]
    pub signatures: Vec<String>,
    /// Printed in the footer, e.g. "2026-11-01 14:05".
    pub generated: String,
    /// Text shown when there are no rows.
    pub empty_text: Option<String>,
}

// ------------------------------------------------------------- metrics

/// Helvetica advance widths (1/1000 em) for ASCII 32..=126.
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // ' '..'/'
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0-9
    278, 278, 584, 584, 584, 556, 1015, // :;<=>?@
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, // A-M
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, // N-Z
    278, 278, 278, 469, 556, 333, // [\]^_`
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, // a-m
    556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, // n-z
    334, 260, 334, 584, // {|}~
];

/// Helvetica-Bold advance widths for ASCII 32..=126.
const HELV_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, //
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, //
    333, 333, 584, 584, 584, 611, 975, //
    722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, //
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, //
    333, 278, 333, 584, 556, 333, //
    556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, //
    611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, //
    389, 280, 389, 584, //
];

/// Maps a char to its WinAnsi byte (Latin-1 range is identical).
fn win_ansi(c: char) -> u8 {
    let u = c as u32;
    match u {
        0x20..=0x7E => u as u8,
        0xA0..=0xFF => u as u8,
        0x2013 => 0x96, // en dash
        0x2014 => 0x97, // em dash
        0x2018 => 0x91,
        0x2019 => 0x92,
        0x201C => 0x93,
        0x201D => 0x94,
        0x2022 => 0x95, // bullet
        0x2026 => 0x85, // ellipsis
        0x20AC => 0x80, // euro
        _ => b'?',
    }
}

fn char_width(b: u8, bold: bool) -> u16 {
    if (32..=126).contains(&b) {
        let t = if bold { &HELV_BOLD } else { &HELV };
        t[(b - 32) as usize]
    } else {
        556
    }
}

pub fn text_width(s: &str, size: f32, bold: bool) -> f32 {
    s.chars().map(|c| char_width(win_ansi(c), bold) as f32).sum::<f32>() * size / 1000.0
}

/// Shortens `s` with "..." so it fits in `max` points.
pub fn fit(s: &str, size: f32, bold: bool, max: f32) -> String {
    if text_width(s, size, bold) <= max {
        return s.to_string();
    }
    let ell = text_width("...", size, bold);
    let mut out = String::new();
    let mut w = 0.0;
    for c in s.chars() {
        let cw = char_width(win_ansi(c), bold) as f32 * size / 1000.0;
        if w + cw + ell > max {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.trim_end().to_string() + "..."
}

// ------------------------------------------------------------- canvas

#[derive(Default)]
struct Canvas {
    buf: Vec<u8>,
}

impl Canvas {
    fn op(&mut self, s: &str) {
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(b'\n');
    }

    fn text(&mut self, x: f32, y: f32, size: f32, bold: bool, s: &str) {
        let font = if bold { "F2" } else { "F1" };
        self.buf
            .extend_from_slice(format!("BT /{font} {size:.2} Tf {x:.2} {y:.2} Td (").as_bytes());
        for c in s.chars() {
            match win_ansi(c) {
                b'(' => self.buf.extend_from_slice(b"\\("),
                b')' => self.buf.extend_from_slice(b"\\)"),
                b'\\' => self.buf.extend_from_slice(b"\\\\"),
                b => self.buf.push(b),
            }
        }
        self.buf.extend_from_slice(b") Tj ET\n");
    }

    fn text_aligned(&mut self, x: f32, w: f32, y: f32, size: f32, bold: bool, align: Align, s: &str) {
        let tw = text_width(s, size, bold);
        let tx = match align {
            Align::Left => x,
            Align::Center => x + (w - tw) / 2.0,
            Align::Right => x + w - tw,
        };
        self.text(tx, y, size, bold, s);
    }

    fn fill_rgb(&mut self, r: f32, g: f32, b: f32) {
        self.op(&format!("{r:.3} {g:.3} {b:.3} rg"));
    }

    fn stroke_rgb(&mut self, r: f32, g: f32, b: f32) {
        self.op(&format!("{r:.3} {g:.3} {b:.3} RG"));
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.op(&format!("{x:.2} {y:.2} {w:.2} {h:.2} re f"));
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32) {
        self.op(&format!("{width:.2} w {x1:.2} {y1:.2} m {x2:.2} {y2:.2} l S"));
    }
}

// ------------------------------------------------------------- layout

const MARGIN: f32 = 36.0;
const FOOTER_H: f32 = 26.0;
const HEAD_H: f32 = 18.0;
const ROW_H: f32 = 15.0;
const BODY_SIZE: f32 = 8.5;
const PAD: f32 = 4.0;
const NAVY: (f32, f32, f32) = (0.15, 0.27, 0.42);
const TEXT: (f32, f32, f32) = (0.12, 0.12, 0.14);
const GREY: (f32, f32, f32) = (0.45, 0.45, 0.48);

struct Layout<'a> {
    doc: &'a Doc,
    w: f32,
    h: f32,
    col_x: Vec<f32>,
    col_w: Vec<f32>,
    pages: Vec<Canvas>,
    y: f32,
}

impl<'a> Layout<'a> {
    fn new(doc: &'a Doc) -> Self {
        let (w, h) = if doc.landscape { (841.89, 595.28) } else { (595.28, 841.89) };
        let cw = w - 2.0 * MARGIN;
        let total: f32 = doc.columns.iter().map(|c| c.weight.max(0.1)).sum::<f32>().max(0.1);
        let mut col_x = Vec::new();
        let mut col_w = Vec::new();
        let mut x = MARGIN;
        for c in &doc.columns {
            let cwid = c.weight.max(0.1) / total * cw;
            col_x.push(x);
            col_w.push(cwid);
            x += cwid;
        }
        Self { doc, w, h, col_x, col_w, pages: Vec::new(), y: 0.0 }
    }

    fn page(&mut self) -> &mut Canvas {
        self.pages.last_mut().expect("page started")
    }

    fn bottom(&self) -> f32 {
        MARGIN + FOOTER_H
    }

    fn content_w(&self) -> f32 {
        self.w - 2.0 * MARGIN
    }

    fn new_page(&mut self, first: bool) {
        self.pages.push(Canvas::default());
        self.y = self.h - MARGIN;
        let doc = self.doc;
        let cw = self.content_w();
        if first {
            if !doc.org.is_empty() {
                let org = fit(&doc.org, 10.0, true, cw);
                let y = self.y - 10.0;
                let p = self.page();
                p.fill_rgb(GREY.0, GREY.1, GREY.2);
                p.text(MARGIN, y, 10.0, true, &org);
                self.y -= 16.0;
            }
            let title = fit(&doc.title, 16.0, true, cw);
            let y = self.y - 16.0;
            let p = self.page();
            p.fill_rgb(NAVY.0, NAVY.1, NAVY.2);
            p.text(MARGIN, y, 16.0, true, &title);
            self.y -= 22.0;
            for line in &doc.subtitle {
                let s = fit(line, 9.0, false, cw);
                let y = self.y - 9.0;
                let p = self.page();
                p.fill_rgb(GREY.0, GREY.1, GREY.2);
                p.text(MARGIN, y, 9.0, false, &s);
                self.y -= 13.0;
            }
            // Accent rule under the title block.
            let y = self.y - 4.0;
            let p = self.page();
            p.stroke_rgb(NAVY.0, NAVY.1, NAVY.2);
            p.line(MARGIN, y, MARGIN + cw, y, 1.2);
            self.y -= 12.0;

            if !doc.info.is_empty() {
                let half = cw / 2.0;
                let label_w = 92.0;
                for (i, (label, value)) in doc.info.iter().enumerate() {
                    let col = (i % 2) as f32;
                    let x = MARGIN + col * half;
                    let y = self.y - 10.0;
                    let l = fit(&format!("{label}:"), 9.0, true, label_w - 4.0);
                    let v = fit(value, 9.0, false, half - label_w - 8.0);
                    let p = self.page();
                    p.fill_rgb(GREY.0, GREY.1, GREY.2);
                    p.text(x, y, 9.0, true, &l);
                    p.fill_rgb(TEXT.0, TEXT.1, TEXT.2);
                    p.text(x + label_w, y, 9.0, false, &v);
                    if i % 2 == 1 || i == doc.info.len() - 1 {
                        self.y -= 15.0;
                    }
                }
                self.y -= 6.0;
            }
        } else {
            let t = fit(&format!("{} (continued)", doc.title), 10.0, true, cw);
            let y = self.y - 10.0;
            let p = self.page();
            p.fill_rgb(NAVY.0, NAVY.1, NAVY.2);
            p.text(MARGIN, y, 10.0, true, &t);
            self.y -= 20.0;
        }
        if !doc.columns.is_empty() {
            self.table_header();
        }
    }

    fn table_header(&mut self) {
        let top = self.y;
        let cw = self.content_w();
        let cols: Vec<(f32, f32, Align, String)> = self
            .doc
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| (self.col_x[i], self.col_w[i], c.align, c.header.clone()))
            .collect();
        let p = self.page();
        p.fill_rgb(NAVY.0, NAVY.1, NAVY.2);
        p.rect(MARGIN, top - HEAD_H, cw, HEAD_H);
        p.fill_rgb(1.0, 1.0, 1.0);
        for (x, w, align, header) in cols {
            let s = fit(&header, BODY_SIZE, true, w - 2.0 * PAD);
            p.text_aligned(x + PAD, w - 2.0 * PAD, top - HEAD_H + 6.0, BODY_SIZE, true, align, &s);
        }
        self.y -= HEAD_H;
    }

    fn row(&mut self, idx: usize, row: &Row) {
        if self.y - ROW_H < self.bottom() {
            self.new_page(false);
        }
        let top = self.y;
        let cw = self.content_w();
        let bold = row.style == RowStyle::Bold;
        let cells: Vec<(f32, f32, Align, String)> = self
            .doc
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let raw = row.cells.get(i).map(String::as_str).unwrap_or("");
                let w = self.col_w[i];
                (self.col_x[i], w, c.align, fit(raw, BODY_SIZE, bold, w - 2.0 * PAD))
            })
            .collect();
        let p = self.page();
        match row.style {
            RowStyle::Warn => {
                p.fill_rgb(1.0, 0.93, 0.85);
                p.rect(MARGIN, top - ROW_H, cw, ROW_H);
            }
            RowStyle::Bold => {
                p.fill_rgb(0.90, 0.92, 0.95);
                p.rect(MARGIN, top - ROW_H, cw, ROW_H);
            }
            _ if idx % 2 == 1 => {
                p.fill_rgb(0.965, 0.97, 0.975);
                p.rect(MARGIN, top - ROW_H, cw, ROW_H);
            }
            _ => {}
        }
        if row.style == RowStyle::Muted {
            p.fill_rgb(0.6, 0.6, 0.62);
        } else {
            p.fill_rgb(TEXT.0, TEXT.1, TEXT.2);
        }
        for (x, w, align, s) in cells {
            p.text_aligned(x + PAD, w - 2.0 * PAD, top - ROW_H + 4.6, BODY_SIZE, bold, align, &s);
        }
        p.stroke_rgb(0.82, 0.84, 0.87);
        p.line(MARGIN, top - ROW_H, MARGIN + cw, top - ROW_H, 0.4);
        self.y -= ROW_H;
    }

    fn ensure(&mut self, needed: f32) {
        if self.y - needed < self.bottom() {
            self.new_page(false);
        }
    }

    fn note(&mut self, text: &str) {
        self.ensure(16.0);
        let s = fit(text, 8.0, false, self.content_w());
        let y = self.y - 12.0;
        let p = self.page();
        p.fill_rgb(GREY.0, GREY.1, GREY.2);
        p.text(MARGIN, y, 8.0, false, &s);
        self.y -= 16.0;
    }

    fn signatures(&mut self) {
        let n = self.doc.signatures.len();
        if n == 0 {
            return;
        }
        self.ensure(70.0);
        let cw = self.content_w();
        let block = cw / n as f32;
        let top = self.y - 18.0;
        let labels = self.doc.signatures.clone();
        let p = self.page();
        for (i, label) in labels.iter().enumerate() {
            let x = MARGIN + i as f32 * block;
            let lw = block - 24.0;
            p.fill_rgb(TEXT.0, TEXT.1, TEXT.2);
            p.text(x, top, 9.0, true, &fit(&format!("{label}:"), 9.0, true, lw));
            p.stroke_rgb(0.3, 0.3, 0.32);
            p.line(x, top - 30.0, x + lw, top - 30.0, 0.6);
            p.fill_rgb(GREY.0, GREY.1, GREY.2);
            p.text_aligned(x, lw, top - 40.0, 7.0, false, Align::Center, "Signature over printed name / date");
        }
        self.y = top - 50.0;
    }

    fn footers(&mut self) {
        let total = self.pages.len();
        let (w, generated) = (self.w, self.doc.generated.clone());
        let title = self.doc.title.clone();
        for (i, p) in self.pages.iter_mut().enumerate() {
            let y = MARGIN;
            p.stroke_rgb(0.82, 0.84, 0.87);
            p.line(MARGIN, y + 12.0, w - MARGIN, y + 12.0, 0.5);
            p.fill_rgb(GREY.0, GREY.1, GREY.2);
            let left = if generated.is_empty() {
                title.clone()
            } else {
                format!("{title}  |  Generated {generated}")
            };
            p.text(MARGIN, y, 7.5, false, &fit(&left, 7.5, false, w - 2.0 * MARGIN - 80.0));
            let right = format!("Page {} of {}", i + 1, total);
            p.text_aligned(w - MARGIN - 80.0, 80.0, y, 7.5, false, Align::Right, &right);
        }
    }
}

/// Renders the document to PDF bytes.
pub fn render(doc: &Doc) -> Vec<u8> {
    let mut l = Layout::new(doc);
    l.new_page(true);
    if doc.rows.is_empty() && !doc.columns.is_empty() {
        let msg = doc.empty_text.clone().unwrap_or_else(|| "No records.".to_string());
        l.ensure(ROW_H);
        let y = l.y - 11.0;
        let p = l.page();
        p.fill_rgb(GREY.0, GREY.1, GREY.2);
        p.text(MARGIN + PAD, y, BODY_SIZE, false, &msg);
        l.y -= ROW_H;
    }
    for (i, row) in doc.rows.iter().enumerate() {
        l.row(i, row);
    }
    if let Some(note) = &doc.note {
        l.y -= 4.0;
        l.note(note);
    }
    l.signatures();
    l.footers();
    assemble(l.w, l.h, &l.pages, &doc.title)
}

fn assemble(w: f32, h: f32, pages: &[Canvas], title: &str) -> Vec<u8> {
    // Object numbers: 1 catalog, 2 pages, 3 F1, 4 F2, 5 info, then per page:
    // page = 6 + 2i, content = 7 + 2i.
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        let n = offsets.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };

    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 6 + 2 * i)).collect();
    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(
        &mut out,
        &mut offsets,
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).as_bytes(),
    );
    obj(
        &mut out,
        &mut offsets,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );
    obj(
        &mut out,
        &mut offsets,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>",
    );
    let mut info = b"<< /Producer (inventory_core) /Title (".to_vec();
    for c in title.chars() {
        match win_ansi(c) {
            b'(' | b')' | b'\\' => {
                info.push(b'\\');
                info.push(win_ansi(c));
            }
            b => info.push(b),
        }
    }
    info.extend_from_slice(b") >>");
    obj(&mut out, &mut offsets, &info);

    for (i, page) in pages.iter().enumerate() {
        obj(
            &mut out,
            &mut offsets,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
                7 + 2 * i
            )
            .as_bytes(),
        );
        let mut body = format!("<< /Length {} >>\nstream\n", page.buf.len()).into_bytes();
        body.extend_from_slice(&page.buf);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &mut offsets, &body);
    }

    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_and_fit() {
        assert!((text_width("A", 10.0, false) - 6.67).abs() < 0.01);
        let s = fit("A very long item name that will not fit", 8.5, false, 60.0);
        assert!(s.ends_with("..."));
        assert!(text_width(&s, 8.5, false) <= 60.0);
    }

    #[test]
    fn renders_multi_page() {
        let doc = Doc {
            title: "Test (report)".into(),
            columns: vec![Col::new("Code", 1.0, Align::Left), Col::new("Qty", 1.0, Align::Right)],
            rows: (0..200).map(|i| Row::new(vec![format!("ITEM-{i}"), i.to_string()])).collect(),
            signatures: vec!["Prepared by".into()],
            ..Default::default()
        };
        let pdf = render(&doc);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Count 5") || text.contains("/Count 4"));
        assert!(text.contains("Test \\(report\\)"));
    }
}
