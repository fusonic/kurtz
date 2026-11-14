//! Public routes: redirects, robots.txt, health and static assets.

use axum::{
    extract::{Path, State},
    http::{header, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};

use super::{not_found_text, AppState};
use crate::{db, redirect, validate};

pub async fn not_found() -> Response {
    not_found_text()
}

pub async fn robots() -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "User-agent: *\nDisallow: /\n",
    )
        .into_response()
}

/// `GET /_health`: 200 when the database answers and every migration shipped
/// with this binary has been applied, 503 otherwise. Answers on any host.
pub async fn health(State(app): State<AppState>) -> Response {
    let text = [
        (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
        (header::CACHE_CONTROL, "no-store"),
    ];
    match app.db.check_migrated().await {
        Ok(()) => (StatusCode::OK, text, "OK\n").into_response(),
        Err(reason) => {
            tracing::warn!("health check failed: {reason}");
            (StatusCode::SERVICE_UNAVAILABLE, text, "NOT_OK\n").into_response()
        }
    }
}

pub async fn asset(State(app): State<AppState>, uri: Uri) -> Response {
    match app.assets.get(uri.path()) {
        Some(a) => (
            [
                (header::CONTENT_TYPE, a.content_type),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            a.bytes,
        )
            .into_response(),
        None => not_found_text(),
    }
}

fn found(location: &str) -> Response {
    let value = HeaderValue::from_str(location).or_else(|_| {
        // Non-ASCII targets: percent-encode via the URL parser.
        url::Url::parse(location)
            .map_err(|_| ())
            .and_then(|u| HeaderValue::from_str(u.as_str()).map_err(|_| ()))
    });
    match value {
        Ok(v) => (
            StatusCode::FOUND,
            [
                (header::LOCATION, v),
                (header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=0")),
            ],
        )
            .into_response(),
        Err(()) => not_found_text(),
    }
}

pub async fn root(State(app): State<AppState>) -> Response {
    match &app.cfg.root_redirect_url {
        Some(u) => found(u),
        None => not_found_text(),
    }
}

pub async fn redirect(State(app): State<AppState>, method: Method, Path(slug): Path<String>, uri: Uri) -> Response {
    let slug = slug.to_lowercase();
    if !validate::looks_like_slug(&slug) {
        return not_found_text();
    }
    let resolved = match app.cache.get(&slug) {
        Some(hit) => hit,
        None => match app.db.resolve(&slug).await {
            Ok(found) => {
                app.cache.put(&slug, found.clone());
                found.map(std::sync::Arc::new)
            }
            Err(e) => {
                tracing::error!("lookup failed: {e}");
                return (StatusCode::SERVICE_UNAVAILABLE, "Service unavailable\n").into_response();
            }
        },
    };
    match resolved {
        Some(r) if r.enabled => {
            if method == Method::GET {
                app.last_used.record(r.link_id, db::timestamp());
            }
            found(&redirect::merge_query(&r.target, uri.query()))
        }
        _ => not_found_text(),
    }
}
