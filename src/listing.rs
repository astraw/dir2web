//! Apache-style directory index pages.

use std::{
    ffi::OsString,
    fmt::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::response::{Html, IntoResponse, Response};

use crate::{
    AppError, AppState,
    html::{escape, human_size, page},
    markdown::{self, is_markdown},
    media::is_video,
    paths::encode_segment,
    query_get,
};

struct Entry {
    name: OsString,
    is_dir: bool,
    size: u64,
    mtime: Option<SystemTime>,
}

#[derive(Clone, Copy, PartialEq)]
enum SortKey {
    Name,
    Mtime,
    Size,
}

impl SortKey {
    fn as_str(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Mtime => "mtime",
            SortKey::Size => "size",
        }
    }
}

fn read_entries(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for de in std::fs::read_dir(dir)? {
        let de = de?;
        let name = de.file_name();
        // Follow symlinks; fall back to the link itself if it dangles.
        let meta = match std::fs::metadata(de.path()) {
            Ok(m) => m,
            Err(_) => match de.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            },
        };
        out.push(Entry {
            name,
            is_dir: meta.is_dir(),
            size: meta.len(),
            mtime: meta.modified().ok(),
        });
    }
    Ok(out)
}

fn fmt_mtime(t: Option<SystemTime>) -> String {
    match t {
        Some(t) => chrono::DateTime::<chrono::Local>::from(t)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => "-".into(),
    }
}

/// Cache-busting token for thumbnail URLs, so browsers may cache them forever.
fn version_token(e: &Entry) -> String {
    let secs = e
        .mtime
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}-{}", e.size)
}

/// The rendered README section for a listing, if the directory has a
/// README.md (or other Markdown extension) inside the root.
async fn readme(st: &AppState, dir: &Path, entries: &[Entry]) -> Option<String> {
    let e = entries.iter().find(|e| {
        !e.is_dir
            && is_markdown(&e.name)
            && Path::new(&e.name)
                .file_stem()
                .is_some_and(|s| s.eq_ignore_ascii_case("readme"))
    })?;
    let path = tokio::fs::canonicalize(dir.join(&e.name)).await.ok()?;
    if !path.starts_with(&st.root) {
        return None;
    }
    let meta = tokio::fs::metadata(&path).await.ok()?;
    let html = markdown::render_file(&path, &meta).await.ok()??;
    Some(markdown::readme_section(
        &e.name.to_string_lossy(),
        &encode_segment(&e.name),
        &html,
    ))
}

pub async fn render(
    st: &AppState,
    dir: &Path,
    uri_path: &str,
    query: Option<&str>,
) -> Result<Response, AppError> {
    let dir2 = dir.to_path_buf();
    let mut entries = tokio::task::spawn_blocking(move || read_entries(&dir2))
        .await
        .map_err(anyhow::Error::from)??;

    let key = match query_get(query, "sort") {
        Some("mtime") => SortKey::Mtime,
        Some("size") => SortKey::Size,
        _ => SortKey::Name,
    };
    let desc = query_get(query, "order") == Some("desc");
    entries.sort_by(|a, b| {
        let ord = match key {
            SortKey::Name => std::cmp::Ordering::Equal,
            SortKey::Mtime => a.mtime.cmp(&b.mtime),
            SortKey::Size => a.size.cmp(&b.size),
        }
        .then_with(|| {
            let an = a.name.to_string_lossy().to_lowercase();
            let bn = b.name.to_string_lossy().to_lowercase();
            an.cmp(&bn).then_with(|| a.name.cmp(&b.name))
        });
        let ord = if desc { ord.reverse() } else { ord };
        // Directories always first.
        b.is_dir.cmp(&a.is_dir).then(ord)
    });

    let display_path = String::from_utf8_lossy(
        &percent_encoding::percent_decode_str(uri_path).collect::<Vec<u8>>(),
    )
    .into_owned();
    let title = format!("Index of {display_path}");

    let header_link = |k: SortKey, label: &str| {
        let order = if k == key && !desc { "desc" } else { "asc" };
        let arrow = if k != key {
            ""
        } else if desc {
            " ▾"
        } else {
            " ▴"
        };
        format!(
            "<a href=\"?sort={}&amp;order={order}\">{label}</a>{arrow}",
            k.as_str()
        )
    };

    let mut body = String::new();
    let _ = write!(
        body,
        "<h1>{}</h1>\n<table>\n<tr><th></th><th>{}</th><th>{}</th><th>{}</th></tr>\n",
        escape(&title),
        header_link(SortKey::Name, "Name"),
        header_link(SortKey::Mtime, "Last modified"),
        header_link(SortKey::Size, "Size"),
    );
    if uri_path != "/" {
        body.push_str(
            "<tr><td class=\"icon\">⬆︎</td><td><a href=\"../\">Parent Directory</a></td>\
             <td class=\"mtime\"></td><td class=\"size\">-</td></tr>\n",
        );
    }
    for e in &entries {
        let enc = encode_segment(&e.name);
        let name = escape(&e.name.to_string_lossy());
        if e.is_dir {
            let _ = writeln!(
                body,
                "<tr><td class=\"icon\">📁</td><td><a href=\"{enc}/\">{name}/</a></td>\
                 <td class=\"mtime\">{}</td><td class=\"size\">-</td></tr>",
                fmt_mtime(e.mtime)
            );
        } else if is_video(&e.name) {
            let _ = writeln!(
                body,
                "<tr><td class=\"icon\"><a href=\"{enc}?play\"><img loading=\"lazy\" width=\"160\" height=\"90\" \
                 src=\"{enc}?thumb&amp;v={}\" alt=\"\"></a></td>\
                 <td><a href=\"{enc}?play\">{name}</a><a class=\"orig\" href=\"{enc}\">original</a></td>\
                 <td class=\"mtime\">{}</td><td class=\"size\">{}</td></tr>",
                version_token(e),
                fmt_mtime(e.mtime),
                human_size(e.size)
            );
        } else if is_markdown(&e.name) {
            let _ = writeln!(
                body,
                "<tr><td class=\"icon\">📝</td>\
                 <td><a href=\"{enc}?view\">{name}</a><a class=\"orig\" href=\"{enc}\">original</a></td>\
                 <td class=\"mtime\">{}</td><td class=\"size\">{}</td></tr>",
                fmt_mtime(e.mtime),
                human_size(e.size)
            );
        } else {
            let _ = writeln!(
                body,
                "<tr><td class=\"icon\">📄</td><td><a href=\"{enc}\">{name}</a></td>\
                 <td class=\"mtime\">{}</td><td class=\"size\">{}</td></tr>",
                fmt_mtime(e.mtime),
                human_size(e.size)
            );
        }
    }
    body.push_str("</table>\n");
    if let Some(readme) = readme(st, dir, &entries).await {
        body.push_str(&readme);
    }
    Ok(Html(page(&title, &body)).into_response())
}
