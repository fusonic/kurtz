//! Admin routes. Every handler checks the session; every state-changing
//! request also checks the CSRF token and the Origin header.

// Handlers return early with a ready `Response`; boxing it buys nothing here.
#![allow(clippy::result_large_err)]

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Extension, Form,
};
use serde::Deserialize;

use super::{not_found_text, views::*, AppState, ClientInfo};
use crate::{
    auth,
    csvio::{self, OnConflict},
    db::{self, Link, ListFilter, NewLink},
    qr, validate,
};

type Pairs = Vec<(String, String)>;

fn field<'a>(pairs: &'a Pairs, name: &str) -> &'a str {
    pairs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

fn render(t: &impl Template) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("template error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Template error\n").into_response()
        }
    }
}

/// Renders a modal body: as is for htmx requests, wrapped in the page layout
/// otherwise.
fn modal(
    app: &AppState,
    admin: &Admin,
    ci: &ClientInfo,
    title: &str,
    body: &impl Template,
    status: StatusCode,
) -> Response {
    let body = match body.render() {
        Ok(html) => html,
        Err(e) => {
            tracing::error!("template error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Template error\n").into_response();
        }
    };
    if ci.htmx {
        // htmx only swaps successful responses, so errors are shown with 200.
        return Html(body).into_response();
    }
    let mut r = render(&Page {
        layout: layout(app, Some(admin)),
        title: title.to_string(),
        body,
    });
    *r.status_mut() = status;
    r
}

fn server_error(e: impl std::fmt::Display) -> Response {
    tracing::error!("{e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Something went wrong. Check the server log.\n",
    )
        .into_response()
}

/// Navigates to `url`: a full redirect for normal requests, `HX-Redirect`
/// for htmx requests.
fn go(ci: &ClientInfo, url: &str) -> Response {
    if ci.htmx {
        let mut r = StatusCode::OK.into_response();
        if let Ok(v) = HeaderValue::from_str(url) {
            r.headers_mut().insert("hx-redirect", v);
        }
        r
    } else {
        Redirect::to(url).into_response()
    }
}

/// An authenticated admin session.
struct Admin {
    session: String,
    csrf: String,
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn session(app: &AppState, headers: &HeaderMap, ci: &ClientInfo) -> Option<Admin> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    let value = auth::cookie_value(cookies, auth::SESSION_COOKIE)?;
    if !auth::verify_session(&app.cfg.session_secret, &ci.host, value, now_secs()) {
        return None;
    }
    Some(Admin {
        session: value.to_string(),
        csrf: auth::csrf_token(&app.cfg.session_secret, value),
    })
}

fn require(app: &AppState, headers: &HeaderMap, ci: &ClientInfo) -> Result<Admin, Response> {
    session(app, headers, ci).ok_or_else(|| go(ci, "/admin"))
}

/// The Origin header, when present, must name this host.
fn same_origin(headers: &HeaderMap, ci: &ClientInfo) -> bool {
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(origin) => url::Url::parse(origin)
            .ok()
            .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|h| h == ci.host),
    }
}

fn require_post(app: &AppState, headers: &HeaderMap, ci: &ClientInfo, token: &str) -> Result<Admin, Response> {
    let admin = require(app, headers, ci)?;
    let token = if token.is_empty() {
        headers.get("x-csrf-token").and_then(|v| v.to_str().ok()).unwrap_or("")
    } else {
        token
    };
    if !same_origin(headers, ci) || !auth::verify_csrf(&app.cfg.session_secret, &admin.session, token) {
        return Err((
            StatusCode::FORBIDDEN,
            "Invalid or missing CSRF token. Reload the page and try again.\n",
        )
            .into_response());
    }
    Ok(admin)
}

fn layout(app: &AppState, admin: Option<&Admin>) -> Layout {
    let (css, css_integrity) = app.assets.stylesheet();
    let (tabler_js, tabler_js_integrity) = app.assets.tabler_script();
    Layout {
        css,
        css_integrity,
        tabler_js,
        tabler_js_integrity,
        app_css: app.assets.app_css.path.clone(),
        htmx: app.assets.htmx.path.clone(),
        js: app.assets.js.path.clone(),
        csrf: admin.map(|a| a.csrf.clone()).unwrap_or_default(),
        logged_in: admin.is_some(),
    }
}

// ---------------------------------------------------------------- login

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    tag: String,
}

async fn all_tags(app: &AppState) -> Result<Vec<(String, i64)>, Response> {
    app.db.tags().await.map_err(server_error)
}

pub async fn index(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Response {
    let Some(admin) = session(&app, &headers, &ci) else {
        return render(&LoginPage {
            layout: layout(&app, None),
            error: String::new(),
        });
    };
    let tag = query.tag.trim().to_lowercase();
    let filter = ListFilter {
        search: Some(&query.q),
        tag: Some(&tag),
    };
    let links = match app.db.list(filter).await {
        Ok(l) => l,
        Err(e) => return server_error(e),
    };
    let tags = match all_tags(&app).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let rows = links.iter().map(|l| RowView::new(l, &query.q)).collect();
    let tag_filters = tags
        .iter()
        .map(|(name, count)| TagFilter {
            active: *name == tag,
            name: name.clone(),
            count: *count,
        })
        .collect();
    render(&ListPage {
        layout: layout(&app, Some(&admin)),
        q: query.q,
        tag,
        tag_filters,
        rows,
    })
}

pub async fn login(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Form(pairs): Form<Pairs>,
) -> Response {
    let page = |status: StatusCode, error: &str| {
        let mut r = render(&LoginPage {
            layout: layout(&app, None),
            error: error.into(),
        });
        *r.status_mut() = status;
        r
    };
    if !same_origin(&headers, &ci) {
        return page(StatusCode::FORBIDDEN, "Login must come from this site.");
    }
    if app.limiter.is_blocked() {
        return page(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many failed attempts. Wait a minute and try again.",
        );
    }
    if !auth::verify_password(&app.cfg.admin_password_hash, field(&pairs, "password")) {
        app.limiter.record_failure();
        return page(StatusCode::UNAUTHORIZED, "Wrong password.");
    }
    app.limiter.reset();
    let value = auth::new_session(&app.cfg.session_secret, &ci.host, now_secs());
    let mut r = Redirect::to("/admin").into_response();
    if let Ok(v) = HeaderValue::from_str(&auth::session_cookie(&value, app.cfg.cookie_secure)) {
        r.headers_mut().insert(header::SET_COOKIE, v);
    }
    r
}

pub async fn logout(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Form(pairs): Form<Pairs>,
) -> Response {
    if let Err(r) = require_post(&app, &headers, &ci, field(&pairs, "_csrf")) {
        return r;
    }
    let mut r = Redirect::to("/admin").into_response();
    if let Ok(v) = HeaderValue::from_str(&auth::clear_session_cookie(app.cfg.cookie_secure)) {
        r.headers_mut().insert(header::SET_COOKIE, v);
    }
    r
}

// ---------------------------------------------------------------- forms

struct Input {
    target: String,
    slug: String,
    tags: String,
    enabled: bool,
    confirm_duplicate: bool,
}

impl Input {
    fn from(pairs: &Pairs) -> Self {
        Self {
            target: field(pairs, "target").trim().to_string(),
            slug: field(pairs, "slug").trim().to_string(),
            tags: field(pairs, "tags").to_string(),
            enabled: field(pairs, "enabled") == "on",
            confirm_duplicate: field(pairs, "confirm_duplicate") == "1",
        }
    }
}

fn suggestions(tags: &[(String, i64)]) -> String {
    tags.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(",")
}

fn new_form_view(app: &AppState, admin: &Admin, input: &Input, tags: &[(String, i64)]) -> FormView {
    FormView {
        action: "/admin/links".into(),
        title: "New link".into(),
        submit_label: "Create link".into(),
        cancel_url: "/admin".into(),
        is_edit: false,
        slug: input.slug.clone(),
        slug_length: app.cfg.slug_length,
        target: input.target.clone(),
        tags: input.tags.clone(),
        tag_suggestions: suggestions(tags),
        enabled: input.enabled,
        errors: vec![],
        duplicates: vec![],
        csrf: admin.csrf.clone(),
    }
}

fn edit_form_view(app: &AppState, admin: &Admin, l: &Link, input: &Input, tags: &[(String, i64)]) -> FormView {
    FormView {
        action: format!("/admin/links/{}/edit", l.id),
        title: format!("Edit /{}", l.slug),
        submit_label: "Save changes".into(),
        cancel_url: format!("/admin/links/{}", l.id),
        is_edit: true,
        slug: l.slug.clone(),
        slug_length: app.cfg.slug_length,
        target: input.target.clone(),
        tags: input.tags.clone(),
        tag_suggestions: suggestions(tags),
        enabled: input.enabled,
        errors: vec![],
        duplicates: vec![],
        csrf: admin.csrf.clone(),
    }
}

/// Modal fragment for htmx, full page otherwise.
fn form_response(app: &AppState, admin: &Admin, ci: &ClientInfo, f: FormView, status: StatusCode) -> Response {
    let title = f.title.clone();
    modal(app, admin, ci, &title, &FormPartial { f }, status)
}

pub async fn new_form(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
) -> Response {
    let admin = match require(&app, &headers, &ci) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let tags = match all_tags(&app).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let input = Input {
        target: String::new(),
        slug: String::new(),
        tags: String::new(),
        enabled: true,
        confirm_duplicate: false,
    };
    let f = new_form_view(&app, &admin, &input, &tags);
    form_response(&app, &admin, &ci, f, StatusCode::OK)
}

/// Message for a slug that another link already uses.
fn slug_conflict(slug: &str, owner: i64) -> String {
    format!("“{slug}” is already used by link #{owner}.")
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().is_some_and(|d| d.is_unique_violation())
}

pub async fn create(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Form(pairs): Form<Pairs>,
) -> Response {
    let admin = match require_post(&app, &headers, &ci, field(&pairs, "_csrf")) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let input = Input::from(&pairs);
    let tags = match all_tags(&app).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let mut f = new_form_view(&app, &admin, &input, &tags);
    let link_tags = validate::tags(&input.tags)
        .map_err(|e| f.errors.push(e))
        .unwrap_or_default();

    let target = validate::target_url(&input.target).map_err(|e| f.errors.push(e)).ok();
    let slug = if input.slug.is_empty() {
        None
    } else {
        validate::slug(&input.slug).map_err(|e| f.errors.push(e)).ok()
    };
    let Some(target) = target else {
        return form_response(&app, &admin, &ci, f, StatusCode::UNPROCESSABLE_ENTITY);
    };
    if !f.errors.is_empty() {
        return form_response(&app, &admin, &ci, f, StatusCode::UNPROCESSABLE_ENTITY);
    }

    if !input.confirm_duplicate {
        match app.db.by_target_hash(&validate::target_hash(&target)).await {
            Ok(dups) if !dups.is_empty() => {
                f.duplicates = dups
                    .iter()
                    .map(|l| DuplicateView {
                        id: l.id,
                        url: ci.short_url(&ci.host, &l.slug),
                        created: stamp(&l.created_at),
                    })
                    .collect();
                return form_response(&app, &admin, &ci, f, StatusCode::OK);
            }
            Ok(_) => {}
            Err(e) => return server_error(e),
        }
    }

    let result: Result<Result<i64, String>, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let slug = match slug {
            Some(s) => {
                if let Some(owner) = tx.slug_owner(&s, None).await? {
                    tx.rollback().await?;
                    return Ok(Err(slug_conflict(&s, owner)));
                }
                s
            }
            None => {
                let mut picked = None;
                for _ in 0..20 {
                    let s = validate::random_slug(app.cfg.slug_length);
                    if validate::RESERVED_SLUGS.contains(&s.as_str()) {
                        continue;
                    }
                    if tx.slug_owner(&s, None).await?.is_none() {
                        picked = Some(s);
                        break;
                    }
                }
                match picked {
                    Some(s) => s,
                    None => {
                        tx.rollback().await?;
                        return Ok(Err("Could not find a free random slug. Increase SLUG_LENGTH.".into()));
                    }
                }
            }
        };
        let ts = db::timestamp();
        let id = tx
            .insert(&NewLink {
                slug,
                target_url: target.clone(),
                enabled: input.enabled,
                created_at: ts,
                updated_at: ts,
                last_used_at: None,
                tags: link_tags.clone(),
            })
            .await?;
        tx.commit().await?;
        Ok(Ok(id))
    }
    .await;

    match result {
        Ok(Ok(_)) => {
            app.cache.clear();
            go(&ci, "/admin")
        }
        Ok(Err(msg)) => {
            f.errors.push(msg);
            form_response(&app, &admin, &ci, f, StatusCode::UNPROCESSABLE_ENTITY)
        }
        Err(e) if is_unique_violation(&e) => {
            f.errors.push("This slug was just taken. Try another.".into());
            form_response(&app, &admin, &ci, f, StatusCode::UNPROCESSABLE_ENTITY)
        }
        Err(e) => server_error(e),
    }
}

async fn load(app: &AppState, id: i64) -> Result<Link, Response> {
    match app.db.get(id).await {
        Ok(Some(l)) => Ok(l),
        Ok(None) => Err(not_found_text()),
        Err(e) => Err(server_error(e)),
    }
}

pub async fn edit_form(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let admin = match require(&app, &headers, &ci) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let l = match load(&app, id).await {
        Ok(l) => l,
        Err(r) => return r,
    };
    let tags = match all_tags(&app).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let input = Input {
        target: l.target_url.clone(),
        slug: l.slug.clone(),
        tags: l.tags.join(", "),
        enabled: l.enabled,
        confirm_duplicate: false,
    };
    let f = edit_form_view(&app, &admin, &l, &input, &tags);
    form_response(&app, &admin, &ci, f, StatusCode::OK)
}

pub async fn update(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(pairs): Form<Pairs>,
) -> Response {
    let admin = match require_post(&app, &headers, &ci, field(&pairs, "_csrf")) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let l = match load(&app, id).await {
        Ok(l) => l,
        Err(r) => return r,
    };
    let input = Input::from(&pairs);
    let tags = match all_tags(&app).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let mut f = edit_form_view(&app, &admin, &l, &input, &tags);
    let link_tags = validate::tags(&input.tags).map_err(|e| f.errors.push(e)).ok();
    let target = validate::target_url(&input.target).map_err(|e| f.errors.push(e)).ok();
    let (Some(target), Some(link_tags)) = (target, link_tags) else {
        return form_response(&app, &admin, &ci, f, StatusCode::UNPROCESSABLE_ENTITY);
    };

    let result: Result<(), sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        tx.update(l.id, &target, input.enabled, &link_tags).await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            app.cache.clear();
            go(&ci, "/admin")
        }
        Err(e) => server_error(e),
    }
}

// ---------------------------------------------------------------- detail & actions

pub async fn detail(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let admin = match require(&app, &headers, &ci) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let l = match load(&app, id).await {
        Ok(l) => l,
        Err(r) => return r,
    };
    let url = ci.short_url(&ci.host, &l.slug);
    let display_url = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest).to_owned();
    let d = DetailView {
        id: l.id,
        target: l.target_url.clone(),
        enabled: l.enabled,
        tags: l.tags.clone(),
        url,
        display_url,
        created: stamp(&l.created_at),
        updated: stamp(&l.updated_at),
        last_used: l.last_used_at.as_ref().map(stamp),
        qr_svg: format!("/admin/links/{}/qr?format=svg", l.id),
        qr_png: format!("/admin/links/{}/qr?format=png", l.id),
    };
    let title = format!("/{}", l.slug);
    modal(&app, &admin, &ci, &title, &DetailPartial { d }, StatusCode::OK)
}

/// Back to the admin page the request came from, if it is one of ours.
fn back_to(headers: &HeaderMap, ci: &ClientInfo, fallback: String) -> String {
    headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(|r| url::Url::parse(r).ok())
        .filter(|u| u.host_str().is_some_and(|h| h.eq_ignore_ascii_case(&ci.host)))
        .filter(|u| u.path() == "/admin" || u.path().starts_with("/admin/"))
        .map(|u| match u.query() {
            Some(q) => format!("{}?{q}", u.path()),
            None => u.path().to_string(),
        })
        .unwrap_or(fallback)
}

pub async fn toggle(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(pairs): Form<Pairs>,
) -> Response {
    if let Err(r) = require_post(&app, &headers, &ci, field(&pairs, "_csrf")) {
        return r;
    }
    let l = match load(&app, id).await {
        Ok(l) => l,
        Err(r) => return r,
    };
    let result = async {
        let mut tx = app.db.begin().await?;
        tx.set_enabled(id, !l.enabled).await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        return server_error(e);
    }
    app.cache.clear();
    // htmx follows this redirect itself and swaps in the refreshed list.
    Redirect::to(&back_to(&headers, &ci, "/admin".to_string())).into_response()
}

pub async fn confirm_delete(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let admin = match require(&app, &headers, &ci) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let l = match load(&app, id).await {
        Ok(l) => l,
        Err(r) => return r,
    };
    let title = format!("Delete /{}", l.slug);
    let body = DeletePartial {
        csrf: admin.csrf.clone(),
        id,
        slug: l.slug,
        target: l.target_url,
    };
    modal(&app, &admin, &ci, &title, &body, StatusCode::OK)
}

pub async fn delete(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(pairs): Form<Pairs>,
) -> Response {
    if let Err(r) = require_post(&app, &headers, &ci, field(&pairs, "_csrf")) {
        return r;
    }
    let result = async {
        let mut tx = app.db.begin().await?;
        tx.delete(id).await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        return server_error(e);
    }
    app.cache.clear();
    go(&ci, "/admin")
}

#[derive(Deserialize)]
pub struct QrQuery {
    #[serde(default)]
    format: String,
    #[serde(default)]
    download: Option<String>,
}

pub async fn qr(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(query): Query<QrQuery>,
) -> Response {
    if session(&app, &headers, &ci).is_none() {
        return (StatusCode::UNAUTHORIZED, "Log in to see QR codes.\n").into_response();
    }
    let l = match load(&app, id).await {
        Ok(l) => l,
        Err(r) => return r,
    };
    let text = ci.short_url(&ci.host, &l.slug);
    let (body, content_type, ext) = match query.format.as_str() {
        "" | "svg" => match qr::svg(&text) {
            Ok(s) => (s.into_bytes(), "image/svg+xml", "svg"),
            Err(e) => return server_error(e),
        },
        "png" => match qr::png(&text) {
            Ok(b) => (b, "image/png", "png"),
            Err(e) => return server_error(e),
        },
        _ => return (StatusCode::BAD_REQUEST, "format must be svg or png\n").into_response(),
    };
    let disposition = format!(
        "{}; filename=\"{}.{ext}\"",
        if query.download.is_some() {
            "attachment"
        } else {
            "inline"
        },
        l.slug
    );
    let mut r = (StatusCode::OK, body).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    r
}

// ---------------------------------------------------------------- CSV

pub async fn export(State(app): State<AppState>, Extension(ci): Extension<ClientInfo>, headers: HeaderMap) -> Response {
    if let Err(r) = require(&app, &headers, &ci) {
        return r;
    }
    let links = match app.db.list(ListFilter::default()).await {
        Ok(l) => l,
        Err(e) => return server_error(e),
    };
    let body = match csvio::export(&links) {
        Ok(b) => b,
        Err(e) => return server_error(e),
    };
    let name = format!(
        "attachment; filename=\"links-{}.csv\"",
        chrono::Utc::now().format("%Y%m%d")
    );
    let mut r = (StatusCode::OK, body).into_response();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    if let Ok(v) = HeaderValue::from_str(&name) {
        r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    r
}

pub async fn import_form(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
) -> Response {
    let admin = match require(&app, &headers, &ci) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let body = ImportPartial {
        csrf: admin.csrf.clone(),
        columns: column_help(),
        report: None,
        errors: vec![],
        overwrite: false,
    };
    modal(&app, &admin, &ci, "Import", &body, StatusCode::OK)
}

pub async fn import(
    State(app): State<AppState>,
    Extension(ci): Extension<ClientInfo>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let mut csrf = String::new();
    let mut overwrite = false;
    let mut commit = false;
    let mut file: Option<Vec<u8>> = None;
    loop {
        match multipart.next_field().await {
            Ok(Some(f)) => {
                let name = f.name().unwrap_or("").to_string();
                match name.as_str() {
                    "file" => match f.bytes().await {
                        Ok(b) => file = Some(b.to_vec()),
                        Err(_) => {
                            return (StatusCode::PAYLOAD_TOO_LARGE, "File is too large (max 5 MB).\n").into_response()
                        }
                    },
                    "_csrf" => csrf = f.text().await.unwrap_or_default(),
                    "conflict" => overwrite = f.text().await.unwrap_or_default() == "overwrite",
                    "action" => commit = f.text().await.unwrap_or_default() == "import",
                    _ => {}
                }
            }
            Ok(None) => break,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    "Could not read the upload. The file may be larger than 5 MB.\n",
                )
                    .into_response()
            }
        }
    }
    let admin = match require_post(&app, &headers, &ci, &csrf) {
        Ok(a) => a,
        Err(r) => return r,
    };
    let page = |report: Option<ReportView>, errors: Vec<csvio::RowError>, status: StatusCode| {
        let body = ImportPartial {
            csrf: admin.csrf.clone(),
            columns: column_help(),
            report,
            errors,
            overwrite,
        };
        modal(&app, &admin, &ci, "Import", &body, status)
    };
    let Some(bytes) = file.filter(|b| !b.is_empty()) else {
        return page(
            None,
            vec![csvio::RowError {
                line: 0,
                message: "Choose a CSV file.".into(),
            }],
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    };
    let parsed = match csvio::parse(&bytes) {
        Ok(p) => p,
        Err(errors) => return page(None, errors, StatusCode::UNPROCESSABLE_ENTITY),
    };
    let policy = if overwrite {
        OnConflict::Overwrite
    } else {
        OnConflict::Skip
    };
    match csvio::import(&app.db, &parsed.rows, policy, commit).await {
        Ok(r) => {
            if r.committed {
                app.cache.clear();
            }
            let view = ReportView {
                created: r.created,
                skipped: r.skipped,
                overwritten: r.overwritten,
                duplicate_targets: r.duplicate_targets,
                committed: r.committed,
                skipped_rows: r.skipped_rows,
                ignored_columns: parsed.ignored_columns.join(", "),
            };
            page(Some(view), vec![], StatusCode::OK)
        }
        Err(e) if is_unique_violation(&e) => page(
            None,
            vec![csvio::RowError {
                line: 0,
                message: "Two rows use the same slug.".into(),
            }],
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        Err(e) => server_error(e),
    }
}
