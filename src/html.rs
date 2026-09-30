//! Small helpers for building HTML by hand.

use std::fmt::Write;

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

pub fn human_size(n: u64) -> String {
    const UNITS: [&str; 6] = ["", "K", "M", "G", "T", "P"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n}")
    } else if v < 10.0 {
        format!("{v:.1}{}", UNITS[i])
    } else {
        format!("{v:.0}{}", UNITS[i])
    }
}

const STYLE: &str = r#"
body { font-family: -apple-system, system-ui, sans-serif; margin: 1em 2em; color: #222; }
h1 { font-size: 1.3em; font-weight: 600; word-break: break-all; }
table { border-collapse: collapse; }
th { text-align: left; border-bottom: 1px solid #ccc; padding: 4px 12px 4px 0; }
th a { color: inherit; }
td { padding: 3px 12px 3px 0; vertical-align: middle; }
td.size, td.mtime { white-space: nowrap; color: #555; font-variant-numeric: tabular-nums; }
td.size { text-align: right; }
td.icon { width: 164px; text-align: center; }
td.icon img { width: 160px; height: 90px; object-fit: contain; display: block; margin: auto; background: #eee; }
tr:hover { background: #f4f4f4; }
a { color: #0645ad; text-decoration: none; }
a:hover { text-decoration: underline; }
a.orig { font-size: 0.85em; color: #777; margin-left: 0.5em; }
video { width: 100%; max-width: 1280px; background: #000; }
#status { color: #555; font-variant-numeric: tabular-nums; }
footer { margin-top: 2em; font-size: 0.8em; }
footer a { color: #999; }
"#;

pub fn page(title: &str, body: &str) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><style>{STYLE}</style></head><body>\n{body}\n</body></html>\n",
        escape(title)
    );
    s
}
