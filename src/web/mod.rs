//! HTTP layer: router, request context and shared state.

mod admin;
mod public;
mod views;

use std::{sync::Arc, time::Duration};

use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};

use crate::{
    assets::Assets,
    auth::LoginLimiter,
    cache::{LastUsed, LinkCache},
    config::Config,
    db::Db,
};

pub struct App {
    pub cfg: Config,
    pub db: Db,
    pub cache: LinkCache,
    pub last_used: LastUsed,
    pub limiter: LoginLimiter,
    pub assets: Assets,
}

pub type AppState = Arc<App>;

impl App {
    pub fn new(cfg: Config, db: Db) -> AppState {
        Arc::new(Self {
            cache: LinkCache::new(cfg.cache_size),
            last_used: LastUsed::default(),
            limiter: LoginLimiter::new(5, Duration::from_secs(60)),
            assets: Assets::default(),
            cfg,
            db,
        })
    }

    /// Writes pending last-used dates. On failure they are kept for the next run.
    pub async fn flush_last_used(&self) -> Result<usize, sqlx::Error> {
        let pending = self.last_used.take();
        let n = pending.len();
        if let Err(e) = self.db.flush_last_used(&pending).await {
            self.last_used.restore(pending);
            return Err(e);
        }
        Ok(n)
    }

    /// Background task flushing last-used dates every `LAST_USED_FLUSH_SECS`.
    pub fn spawn_flusher(state: AppState) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(state.cfg.last_used_flush_secs));
            tick.tick().await;
            loop {
                tick.tick().await;
                if let Err(e) = state.flush_last_used().await {
                    tracing::warn!("could not write last-used dates, will retry: {e}");
                }
            }
        })
    }
}

/// Per-request facts derived from headers and the peer address.
#[derive(Clone, Debug)]
pub struct ClientInfo {
    /// Request host, lowercase, without port.
    pub host: String,
    /// `http` or `https`, as the visitor reached us (forwarded by the proxy).
    pub scheme: &'static str,
    /// The visitor's port including the colon (`:8080`), empty if none.
    pub port: String,
    /// Request was made by htmx.
    pub htmx: bool,
}

impl ClientInfo {
    /// Short URL for `host` in the scheme and port the visitor is using.
    pub fn short_url(&self, host: &str, slug: &str) -> String {
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        format!("{}://{host}{}/{slug}", self.scheme, self.port)
    }
}

pub fn router(state: AppState) -> Router {
    let import_limit = crate::csvio::MAX_BYTES + 256 * 1024;
    Router::new()
        .route("/_health", get(public::health))
        .route("/robots.txt", get(public::robots))
        .route("/static/{file}", get(public::asset))
        .route("/", get(public::root))
        .route("/{slug}", get(public::redirect))
        .route("/admin", get(admin::index))
        .route("/admin/login", post(admin::login))
        .route("/admin/logout", post(admin::logout))
        .route("/admin/links", post(admin::create))
        .route("/admin/links/new", get(admin::new_form))
        .route("/admin/links/{id}", get(admin::detail))
        .route("/admin/links/{id}/edit", get(admin::edit_form).post(admin::update))
        .route("/admin/links/{id}/toggle", post(admin::toggle))
        .route(
            "/admin/links/{id}/delete",
            get(admin::confirm_delete).post(admin::delete),
        )
        .route("/admin/links/{id}/qr", get(admin::qr))
        .route("/admin/export.csv", get(admin::export))
        .route(
            "/admin/import",
            get(admin::import_form)
                .post(admin::import)
                .layer(DefaultBodyLimit::max(import_limit)),
        )
        .fallback(public::not_found)
        .layer(middleware::from_fn_with_state(state.clone(), context))
        .with_state(state)
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Lowercases a host header and strips the port.
pub fn normalize_host(raw: &str) -> String {
    let first = raw.split(',').next().unwrap_or("").trim().to_ascii_lowercase();
    let host = if let Some(rest) = first.strip_prefix('[') {
        rest.split(']').next().unwrap_or("").to_string()
    } else {
        match first.rsplit_once(':') {
            Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h.to_string(),
            _ => first,
        }
    };
    host.trim_end_matches('.').to_string()
}

/// Less common headers that carry the visitor's hostname, tried after
/// `X-Forwarded-Host` and `Forwarded` (RFC 7239).
const FORWARDED_HOST_HEADERS: [&str; 2] = ["x-original-host", "x-host"];

/// The `host=` parameter of the first element of an RFC 7239 `Forwarded` header.
fn forwarded_param_host(headers: &HeaderMap) -> Option<&str> {
    header_str(headers, "forwarded")?
        .split(',')
        .next()?
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.trim().trim_matches('"'))
        .filter(|v| !v.is_empty())
}

/// The hostname the visitor used: a host header set by the reverse proxy or
/// CDN if present, otherwise `Host`.
fn request_host<'a>(headers: &'a HeaderMap, uri_host: Option<&'a str>) -> &'a str {
    header_str(headers, "x-forwarded-host")
        .or_else(|| forwarded_param_host(headers))
        .or_else(|| FORWARDED_HOST_HEADERS.iter().find_map(|name| header_str(headers, name)))
        .or_else(|| header_str(headers, "host"))
        .or(uri_host)
        .unwrap_or("")
}

/// The port of a `Host`-style value (`example.com:8080`, `[::1]:8080`), with colon.
fn host_port(raw: &str) -> String {
    let first = raw.split(',').next().unwrap_or("").trim();
    match first.rsplit_once(':') {
        Some((_, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => format!(":{port}"),
        _ => String::new(),
    }
}

/// The scheme the visitor used, from the proxy's headers.
fn forwarded_scheme(headers: &HeaderMap) -> Option<&'static str> {
    let from_forwarded = || {
        header_str(headers, "forwarded")?
            .split(',')
            .next()?
            .split(';')
            .filter_map(|pair| pair.split_once('='))
            .find(|(k, _)| k.trim().eq_ignore_ascii_case("proto"))
            .map(|(_, v)| v.trim().trim_matches('"'))
    };
    header_str(headers, "x-forwarded-proto")
        .or_else(from_forwarded)
        .or_else(|| header_str(headers, "cloudfront-forwarded-proto"))
        .or_else(|| header_str(headers, "x-forwarded-scheme"))
        .and_then(|v| match v.split(',').next()?.trim().to_ascii_lowercase().as_str() {
            "https" => Some("https"),
            "http" => Some("http"),
            _ => None,
        })
}

/// Resolves host, scheme and port (taken from the proxy's forwarding headers)
/// and adds the global response headers.
async fn context(State(app): State<AppState>, mut req: Request, next: Next) -> Response {
    let headers = req.headers();

    let raw_host = request_host(headers, req.uri().host());
    let host = normalize_host(raw_host);
    let port = host_port(raw_host);
    let scheme = forwarded_scheme(headers).unwrap_or_else(|| app.cfg.public_scheme());
    let htmx = header_str(headers, "hx-request") == Some("true");
    let path = req.uri().path().to_string();

    let method = req.method().clone();
    let started = std::time::Instant::now();
    req.extensions_mut().insert(ClientInfo {
        host: host.clone(),
        scheme,
        port,
        htmx,
    });
    let mut res = next.run(req).await;
    log_request(&method, &host, &path, res.status(), started.elapsed());

    let h = res.headers_mut();
    let set = |h: &mut HeaderMap, k: header::HeaderName, v: &'static str| {
        h.entry(k).or_insert(HeaderValue::from_static(v));
    };
    set(h, header::HeaderName::from_static("x-robots-tag"), "noindex, nofollow");
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    let is_admin = path == "/admin" || path.starts_with("/admin/");
    // Redirects leak nothing to the target. Admin pages need `same-origin`:
    // with `no-referrer` browsers send `Origin: null` on form posts, which
    // the CSRF origin check would reject.
    set(
        h,
        header::REFERRER_POLICY,
        if is_admin { "same-origin" } else { "no-referrer" },
    );
    set(h, header::X_FRAME_OPTIONS, "DENY");
    if app.cfg.cookie_secure {
        set(h, header::STRICT_TRANSPORT_SECURITY, "max-age=31536000");
    }
    if is_admin {
        set(h, header::CACHE_CONTROL, "no-store");
        // Without the embedded Tabler copy (development) the stylesheet comes from the CDN.
        let styles = if app.assets.css.is_some() {
            "'self'"
        } else {
            "'self' https://cdn.jsdelivr.net"
        };
        let csp = format!(
            "default-src 'self'; img-src 'self' data:; style-src {styles}; script-src 'self'; \
             frame-ancestors 'none'; form-action 'self'; base-uri 'none'"
        );
        if let Ok(v) = HeaderValue::from_str(&csp) {
            h.insert(header::CONTENT_SECURITY_POLICY, v);
        }
    }
    res
}

/// `info`: only 404s, so broken or mistyped links stand out.
/// `debug`: every request. Query strings and client IPs are never logged.
fn log_request(method: &axum::http::Method, host: &str, path: &str, status: StatusCode, took: Duration) {
    let ms = took.as_secs_f64() * 1000.0;
    if status == StatusCode::NOT_FOUND {
        tracing::info!(target: "kurtz::request", "{method} {host}{path} {} {ms:.1}ms", status.as_u16());
    } else {
        tracing::debug!(target: "kurtz::request", "{method} {host}{path} {} {ms:.1}ms", status.as_u16());
    }
}

pub(crate) fn not_found_text() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "Not found\n",
    )
        .into_response()
}
