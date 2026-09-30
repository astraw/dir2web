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
section.readme { margin-top: 2em; border-top: 1px solid #ccc; }
.readme-name { margin: 0.6em 0; font-size: 0.85em; color: #777; }
.readme-name a { color: inherit; }
article.markdown { max-width: 52em; line-height: 1.5; }
article.markdown h1 { font-size: 1.8em; border-bottom: 1px solid #ddd; padding-bottom: 0.2em; }
article.markdown h2 { font-size: 1.4em; border-bottom: 1px solid #eee; padding-bottom: 0.2em; }
article.markdown code { background: #f3f4f6; padding: 0.1em 0.3em; border-radius: 3px; font-size: 0.9em; }
article.markdown pre { background: #f3f4f6; padding: 0.8em 1em; overflow: auto; border-radius: 4px; }
article.markdown pre code { background: none; padding: 0; }
article.markdown table { border-collapse: collapse; }
article.markdown th, article.markdown td { border: 1px solid #ddd; padding: 4px 10px; }
article.markdown tr:hover { background: none; }
article.markdown blockquote { margin-left: 0; padding-left: 1em; border-left: 4px solid #ddd; color: #555; }
article.markdown img { max-width: 100%; }
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
