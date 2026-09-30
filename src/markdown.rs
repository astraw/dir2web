//! Rendering Markdown files, and READMEs below directory listings.
//!
//! Raw HTML in Markdown is passed through: the served files are trusted.

use std::{collections::HashMap, ffi::OsStr, fs::Metadata, path::Path};

use axum::response::{Html, IntoResponse, Response};
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd};

use crate::{
    AppError,
    html::{escape, human_size, page},
};

/// Larger files are not rendered (only offered as originals).
pub const MAX_RENDER_BYTES: u64 = 16 << 20;

const EXTS: &[&str] = &["md", "markdown", "mdown", "mkd"];

pub fn is_markdown(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| EXTS.iter().any(|m| m.eq_ignore_ascii_case(ext)))
}

/// GitHub-style heading anchor: lowercase, punctuation dropped, spaces to `-`.
fn slugify(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Point relative links at other Markdown files to their rendered view.
fn view_link(url: &str) -> Option<String> {
    if url.starts_with('#') {
        return None;
    }
    // Leave anything with a scheme (http:, mailto:, ...) alone.
    if let Some(colon) = url.find(':')
        && !url[..colon].contains('/')
    {
        return None;
    }
    let (path, frag) = match url.find('#') {
        Some(i) => url.split_at(i),
        None => (url, ""),
    };
    if path.contains('?') || !is_markdown(OsStr::new(path)) {
        return None;
    }
    Some(format!("{path}?view{frag}"))
}

/// Render Markdown (GitHub flavoured, raw HTML passed through) to HTML.
pub fn render(src: &str) -> String {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
    let events: Vec<Event> = Parser::new_ext(src, opts).collect();

    // First pass: anchors for headings without an explicit {#id}.
    let mut slugs = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut text: Option<String> = None;
    for ev in &events {
        match ev {
            Event::Start(Tag::Heading { id: None, .. }) => text = Some(String::new()),
            Event::Text(t) | Event::Code(t) => {
                if let Some(s) = text.as_mut() {
                    s.push_str(t);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(t) = text.take() {
                    let base = slugify(&t);
                    let n = seen.entry(base.clone()).or_insert(0);
                    slugs.push(if *n == 0 {
                        base.clone()
                    } else {
                        format!("{base}-{n}")
                    });
                    *n += 1;
                }
            }
            _ => {}
        }
    }

    // Second pass: apply anchors and link rewrites.
    let mut slugs = slugs.into_iter();
    let events = events.into_iter().map(|ev| match ev {
        Event::Start(Tag::Heading {
            level,
            id: None,
            classes,
            attrs,
        }) => Event::Start(Tag::Heading {
            level,
            id: slugs.next().map(CowStr::from),
            classes,
            attrs,
        }),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            let dest_url = view_link(&dest_url).map(CowStr::from).unwrap_or(dest_url);
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            })
        }
        ev => ev,
    });
    let mut out = String::with_capacity(src.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut out, events);
    out
}

/// Read and render a Markdown file, or `None` if it is too large.
pub async fn render_file(path: &Path, meta: &Metadata) -> Result<Option<String>, AppError> {
    if meta.len() > MAX_RENDER_BYTES {
        return Ok(None);
    }
    let bytes = tokio::fs::read(path).await?;
    Ok(Some(render(&String::from_utf8_lossy(&bytes))))
}

pub async fn view_page(path: &Path, meta: &Metadata, uri_path: &str) -> Result<Response, AppError> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let raw_href = uri_path.rsplit('/').next().unwrap_or("");
    let article = match render_file(path, meta).await? {
        Some(html) => format!("<article class=\"markdown\">\n{html}</article>"),
        None => format!(
            "<p>Too large to render ({}); open the original instead.</p>",
            human_size(meta.len())
        ),
    };
    let body = format!(
        "<p><a href=\"./\">⬑ Back to directory</a> · <a href=\"{raw_href}\">original</a></p>\n{article}"
    );
    Ok(Html(page(&name, &body)).into_response())
}

/// The README section shown below a directory listing.
pub fn readme_section(name: &str, href: &str, html: &str) -> String {
    format!(
        "<section class=\"readme\"><div class=\"readme-name\"><a href=\"{href}?view\">{}</a></div>\n\
         <article class=\"markdown\">\n{html}</article></section>\n",
        escape(name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("Größe & `code`_x"), "größe--code_x");
    }

    #[test]
    fn links() {
        assert_eq!(view_link("other.md").as_deref(), Some("other.md?view"));
        assert_eq!(
            view_link("../doc/A.MD#usage").as_deref(),
            Some("../doc/A.MD?view#usage")
        );
        assert_eq!(view_link("https://example.com/x.md"), None);
        assert_eq!(view_link("mailto:a@b.md"), None);
        assert_eq!(view_link("#section"), None);
        assert_eq!(view_link("fig.png"), None);
        assert_eq!(view_link("x.md?raw"), None);
    }

    #[test]
    fn headings_and_raw_html() {
        let html = render("# Intro\n\n## Intro\n\n## Custom {#mine}\n\n<b>raw</b>\n");
        assert!(html.contains(r#"<h1 id="intro">"#), "{html}");
        assert!(html.contains(r#"<h2 id="intro-1">"#), "{html}");
        assert!(html.contains(r#"<h2 id="mine">"#), "{html}");
        assert!(html.contains("<b>raw</b>"), "{html}");
    }
}
