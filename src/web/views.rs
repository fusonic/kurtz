//! Template structs and the view models they render.

use askama::Template;
use chrono::{DateTime, Utc};

use crate::{csvio::RowError, db::Link};

pub struct Layout {
    pub css: String,
    pub app_css: String,
    /// SRI hash when the stylesheet comes from the CDN.
    pub css_integrity: Option<String>,
    pub tabler_js: String,
    /// SRI hash when Tabler's script comes from the CDN.
    pub tabler_js_integrity: Option<String>,
    pub htmx: String,
    pub js: String,
    pub csrf: String,
    pub logged_in: bool,
}

/// A point in time: RFC 3339 for the `<time>` element, which the browser
/// reformats for the visitor's locale and time zone, and a UTC fallback text.
pub struct Stamp {
    pub iso: String,
    pub text: String,
}

pub fn stamp(d: &DateTime<Utc>) -> Stamp {
    Stamp {
        iso: d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        text: d.format("%Y-%m-%d %H:%M UTC").to_string(),
    }
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginPage {
    pub layout: Layout,
    pub error: String,
}

pub struct RowView {
    pub id: i64,
    pub slug: String,
    /// (tag, list URL filtered by it)
    pub tags: Vec<(String, String)>,
    pub short_display: String,
    pub target: String,
    pub created: Stamp,
    pub last_used: Option<Stamp>,
    pub enabled: bool,
}

impl RowView {
    pub fn new(l: &Link, q: &str) -> Self {
        let enc = |s: &str| -> String { url::form_urlencoded::byte_serialize(s.as_bytes()).collect() };
        let tags = l
            .tags
            .iter()
            .map(|t| {
                let href = if q.is_empty() {
                    format!("/admin?tag={}", enc(t))
                } else {
                    format!("/admin?q={}&tag={}", enc(q), enc(t))
                };
                (t.clone(), href)
            })
            .collect();
        Self {
            id: l.id,
            slug: l.slug.clone(),
            tags,
            short_display: format!("/{}", l.slug),
            target: l.target_url.clone(),
            created: stamp(&l.created_at),
            last_used: l.last_used_at.as_ref().map(stamp),
            enabled: l.enabled,
        }
    }
}

pub struct TagFilter {
    pub name: String,
    pub count: i64,
    pub active: bool,
}

#[derive(Template)]
#[template(path = "list.html")]
pub struct ListPage {
    pub layout: Layout,
    pub q: String,
    /// Active tag filter, empty for none.
    pub tag: String,
    pub tag_filters: Vec<TagFilter>,
    pub rows: Vec<RowView>,
}

pub struct DetailView {
    pub id: i64,
    pub slug: String,
    pub target: String,
    pub enabled: bool,
    pub tags: Vec<String>,
    /// Short URL as the admin reaches it (scheme, host and port of the request).
    pub url: String,
    pub created: Stamp,
    pub updated: Stamp,
    pub last_used: Option<Stamp>,
    pub qr_svg: String,
    pub qr_png: String,
}

#[derive(Template)]
#[template(path = "detail.html")]
pub struct DetailPartial {
    pub d: DetailView,
}

pub struct DuplicateView {
    pub id: i64,
    pub url: String,
    pub created: Stamp,
}

pub struct FormView {
    pub action: String,
    pub title: String,
    pub submit_label: String,
    pub cancel_url: String,
    pub is_edit: bool,
    pub slug: String,
    pub slug_length: usize,
    pub target: String,
    /// Current tags, comma-separated.
    pub tags: String,
    /// All existing tags, comma-separated, for autosuggest.
    pub tag_suggestions: String,
    pub enabled: bool,
    pub errors: Vec<String>,
    pub duplicates: Vec<DuplicateView>,
    pub csrf: String,
}

#[derive(Template)]
#[template(path = "form_body.html")]
pub struct FormPartial {
    pub f: FormView,
}

#[derive(Template)]
#[template(path = "delete.html")]
pub struct DeletePartial {
    pub csrf: String,
    pub id: i64,
    pub slug: String,
    pub target: String,
}

pub struct ReportView {
    pub created: usize,
    pub skipped: usize,
    pub overwritten: usize,
    pub duplicate_targets: usize,
    pub committed: bool,
    pub skipped_rows: Vec<RowError>,
    /// Ignored CSV columns, comma-separated; empty if none.
    pub ignored_columns: String,
}

pub struct ColumnHelp {
    pub key: &'static str,
    pub hint: &'static str,
    pub required: bool,
    /// All accepted header names, for the tooltip.
    pub headers: String,
}

/// The importable columns for the help text of the import dialog.
pub fn column_help() -> Vec<ColumnHelp> {
    crate::csvio::COLUMNS
        .iter()
        .map(|c| ColumnHelp {
            key: c.key,
            hint: c.hint,
            required: c.required,
            headers: c.headers(),
        })
        .collect()
}

#[derive(Template)]
#[template(path = "import.html")]
pub struct ImportPartial {
    pub csrf: String,
    pub columns: Vec<ColumnHelp>,
    pub report: Option<ReportView>,
    pub errors: Vec<RowError>,
    pub overwrite: bool,
}

/// Wraps a modal body in the page layout for requests without htmx.
#[derive(Template)]
#[template(path = "page.html")]
pub struct Page {
    pub layout: Layout,
    pub title: String,
    pub body: String,
}
