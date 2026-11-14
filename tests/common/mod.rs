//! Shared helpers for the integration tests.
//!
//! Database tests run against every backend configured through
//! `KURTZ_TEST_POSTGRES_URL` and `KURTZ_TEST_MYSQL_URL`. Without either, they
//! are skipped with a message.
#![allow(dead_code)]

use std::{collections::HashMap, sync::OnceLock};

use axum::{
    body::Body,
    http::{header, Method, Request, Response, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use kurtz::{config::Config, db::Db, router, App, AppState};
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

pub const PASSWORD: &str = "correct horse battery staple";
pub const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub const HOST_A: &str = "a.test";
pub const HOST_B: &str = "b.test";

static DB_LOCK: Mutex<()> = Mutex::const_new(());
static HASH: OnceLock<String> = OnceLock::new();

pub fn backends() -> Vec<String> {
    let urls: Vec<String> = ["KURTZ_TEST_POSTGRES_URL", "KURTZ_TEST_MYSQL_URL"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .filter(|v| !v.is_empty())
        .collect();
    if urls.is_empty() {
        eprintln!("skipping database tests: set KURTZ_TEST_POSTGRES_URL and/or KURTZ_TEST_MYSQL_URL");
    }
    urls
}

pub fn config(url: &str, extra: &[(&str, &str)]) -> Config {
    let hash = HASH
        .get_or_init(|| kurtz::auth::hash_password(PASSWORD).unwrap())
        .clone();
    let mut vars: HashMap<String, String> = HashMap::from([
        ("DATABASE_URL".into(), url.to_string()),
        ("ADMIN_PASSWORD".into(), hash),
        ("SESSION_SECRET".into(), SECRET.into()),
        ("COOKIE_SECURE".into(), "false".into()),
    ]);
    for (k, v) in extra {
        vars.insert(k.to_string(), v.to_string());
    }
    Config::from_lookup(|k| vars.get(k).cloned()).expect("test config")
}

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
    _guard: MutexGuard<'static, ()>,
}

/// Fresh, migrated, empty database and an app on top of it.
pub async fn setup(url: &str, extra: &[(&str, &str)]) -> TestApp {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("error")
        .try_init();
    let guard = DB_LOCK.lock().await;
    let cfg = config(url, extra);
    let db = Db::connect(url).await.expect("connect");
    db.migrate().await.expect("migrate");
    wipe(&db).await;
    let state = App::new(cfg, db);
    let router = router(state.clone());
    TestApp {
        router,
        state,
        _guard: guard,
    }
}

pub async fn wipe(db: &Db) {
    match db {
        Db::Pg(p) => {
            sqlx::query("DELETE FROM link_tags").execute(p).await.unwrap();
            sqlx::query("DELETE FROM links").execute(p).await.unwrap();
        }
        Db::My(p) => {
            sqlx::query("DELETE FROM link_tags").execute(p).await.unwrap();
            sqlx::query("DELETE FROM links").execute(p).await.unwrap();
        }
    }
}

pub struct Res {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Vec<u8>,
}

impl Res {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn header(&self, name: &str) -> String {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    }
}

async fn collect(res: Response<Body>) -> Res {
    let status = res.status();
    let headers = res.headers().clone();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    Res { status, headers, body }
}

pub struct Session {
    pub cookie: String,
    pub csrf: String,
}

impl TestApp {
    pub async fn send(&self, req: Request<Body>) -> Res {
        collect(self.router.clone().oneshot(req).await.unwrap()).await
    }

    pub async fn get(&self, host: &str, path: &str, session: Option<&Session>) -> Res {
        let mut b = Request::builder()
            .method(Method::GET)
            .uri(path)
            .header(header::HOST, host);
        if let Some(s) = session {
            b = b.header(header::COOKIE, &s.cookie);
        }
        self.send(b.body(Body::empty()).unwrap()).await
    }

    pub async fn post(&self, host: &str, path: &str, session: Option<&Session>, form: &[(&str, &str)]) -> Res {
        self.post_with(host, path, session, form, &[]).await
    }

    pub async fn post_with(
        &self,
        host: &str,
        path: &str,
        session: Option<&Session>,
        form: &[(&str, &str)],
        extra_headers: &[(&str, &str)],
    ) -> Res {
        let body = form
            .iter()
            .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
            .collect::<Vec<_>>()
            .join("&");
        let mut b = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(s) = session {
            b = b.header(header::COOKIE, &s.cookie);
        }
        for (k, v) in extra_headers {
            b = b.header(*k, *v);
        }
        self.send(b.body(Body::from(body)).unwrap()).await
    }

    pub async fn login(&self, host: &str) -> Session {
        let res = self.post(host, "/admin/login", None, &[("password", PASSWORD)]).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "login failed: {}", res.text());
        let set_cookie = res.header("set-cookie");
        let cookie = set_cookie.split(';').next().unwrap().to_string();
        let value = cookie.split_once('=').unwrap().1.to_string();
        Session {
            csrf: kurtz::auth::csrf_token(SECRET.as_bytes(), &value),
            cookie,
        }
    }

    /// Creates a link through the admin form.
    pub async fn create(&self, s: &Session, target: &str, slug: &str) -> Res {
        self.create_tagged(s, target, slug, "").await
    }

    pub async fn create_tagged(&self, s: &Session, target: &str, slug: &str, tags: &str) -> Res {
        let form: Vec<(&str, &str)> = vec![
            ("_csrf", &s.csrf),
            ("target", target),
            ("slug", slug),
            ("tags", tags),
            ("enabled", "on"),
            ("confirm_duplicate", "1"),
        ];
        self.post(HOST_A, "/admin/links", Some(s), &form).await
    }

    pub async fn create_ok(&self, s: &Session, target: &str, slug: &str) -> i64 {
        let res = self.create(s, target, slug).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "create failed: {}", res.text());
        self.id_of(slug).await
    }

    /// The id of the link with this slug, or of the newest link if empty.
    pub async fn id_of(&self, slug: &str) -> i64 {
        let filter = kurtz::db::ListFilter {
            search: (!slug.is_empty()).then_some(slug),
            tag: None,
        };
        let links = self.state.db.list(filter).await.unwrap();
        let matching = links
            .iter()
            .filter(|l| slug.is_empty() || l.slug.eq_ignore_ascii_case(slug));
        matching.map(|l| l.id).max().expect("link exists")
    }
}

pub fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
