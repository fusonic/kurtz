//! End-to-end tests through the HTTP router against PostgreSQL and MySQL.

mod common;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use common::*;

#[tokio::test]
async fn any_host_works_and_unknown_slugs_are_404() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        app.create_ok(&s, "https://x.com", "x").await;
        for host in ["a.test", "other.example", "127.0.0.1", "short.example:8080"] {
            assert_eq!(app.get(host, "/x", None).await.header("location"), "https://x.com");
            assert_eq!(app.get(host, "/nope", None).await.status, StatusCode::NOT_FOUND);
            assert_eq!(app.get(host, "/robots.txt", None).await.status, StatusCode::OK);
            assert!(app.get(host, "/admin", None).await.text().contains("type=\"password\""));
        }
        let health = app.get("localhost", "/_health", None).await;
        assert_eq!((health.status, health.text().as_str()), (StatusCode::OK, "OK\n"));
        assert_eq!(
            app.get("localhost", "/health", None).await.status,
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn short_urls_use_the_scheme_host_and_port_of_the_request() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let id = app.create_ok(&s, "https://x.com", "x").await;
        let detail = |headers: &[(&str, &str)]| {
            let mut b = Request::builder()
                .uri(format!("/admin/links/{id}"))
                .header(header::HOST, "internal:8080")
                .header(header::COOKIE, &s.cookie);
            for (k, v) in headers {
                b = b.header(*k, *v);
            }
            app.send(b.body(Body::empty()).unwrap())
        };
        // The session belongs to a.test, so every variant forwards that host.
        let cases: [(&[(&str, &str)], &str); 5] = [
            (&[("x-forwarded-host", "a.test:8443")], "http://a.test:8443/x"),
            (
                &[("x-forwarded-host", "a.test"), ("x-forwarded-proto", "https")],
                "https://a.test/x",
            ),
            (
                &[("forwarded", "host=a.test:9000;proto=https")],
                "https://a.test:9000/x",
            ),
            (
                &[("x-original-host", "a.test"), ("cloudfront-forwarded-proto", "https")],
                "https://a.test/x",
            ),
            (&[("x-host", "a.test:81")], "http://a.test:81/x"),
        ];
        for (headers, expected) in cases {
            let page = detail(headers).await.text();
            assert!(page.contains(expected), "{headers:?}: {page}");
        }
    }
}

#[tokio::test]
async fn robots_and_headers() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        for host in [HOST_A, HOST_B] {
            let res = app.get(host, "/robots.txt", None).await;
            assert_eq!(res.status, StatusCode::OK);
            assert_eq!(res.text(), "User-agent: *\nDisallow: /\n");
            assert_eq!(res.header("x-robots-tag"), "noindex, nofollow");
        }
        let admin = app.get(HOST_A, "/admin", None).await;
        assert_eq!(admin.header("cache-control"), "no-store");
        // `no-referrer` would make browsers send `Origin: null` on the login form
        assert_eq!(admin.header("referrer-policy"), "same-origin");
        assert_eq!(
            app.get(HOST_A, "/robots.txt", None).await.header("referrer-policy"),
            "no-referrer"
        );
        assert!(admin.header("content-security-policy").contains("default-src 'self'"));
        assert_eq!(admin.header("x-robots-tag"), "noindex, nofollow");
    }
}

#[tokio::test]
async fn login_is_a_password_field_and_sessions_are_per_host() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let page = app.get(HOST_B, "/admin", None).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.text().contains("type=\"password\""));
        assert!(!page.text().contains("username"));

        let wrong = app.post(HOST_A, "/admin/login", None, &[("password", "nope")]).await;
        assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
        assert!(wrong.text().contains("Wrong password"));

        let s = app.login(HOST_A).await;
        let list = app.get(HOST_A, "/admin", Some(&s)).await;
        assert!(list.text().contains("new-target"));
        // the cookie for a.test does not log you in on b.test
        let other = app.get(HOST_B, "/admin", Some(&s)).await;
        assert!(other.text().contains("type=\"password\""));
        // and admin works on every host
        let s_b = app.login(HOST_B).await;
        assert!(app
            .get(HOST_B, "/admin", Some(&s_b))
            .await
            .text()
            .contains("new-target"));
    }
}

#[tokio::test]
async fn login_is_rate_limited() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        for _ in 0..5 {
            app.post(HOST_A, "/admin/login", None, &[("password", "nope")]).await;
        }
        let res = app.post(HOST_A, "/admin/login", None, &[("password", PASSWORD)]).await;
        assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    }
}

#[tokio::test]
async fn state_changes_need_session_csrf_and_origin() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let form = [("target", "https://e.com"), ("enabled", "on")];
        // no session
        let res = app.post(HOST_A, "/admin/links", None, &form).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER);
        assert_eq!(res.header("location"), "/admin");
        // session but no token
        let s = app.login(HOST_A).await;
        assert_eq!(
            app.post(HOST_A, "/admin/links", Some(&s), &form).await.status,
            StatusCode::FORBIDDEN
        );
        // token but foreign origin
        let res = app
            .post_with(
                HOST_A,
                "/admin/links",
                Some(&s),
                &[("_csrf", &s.csrf), ("target", "https://e.com"), ("enabled", "on")],
                &[("origin", "https://evil.example")],
            )
            .await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
        // correct
        let res = app
            .post_with(
                HOST_A,
                "/admin/links",
                Some(&s),
                &[("_csrf", &s.csrf), ("target", "https://e.com"), ("enabled", "on")],
                &[("origin", "http://a.test")],
            )
            .await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.text());
    }
}

#[tokio::test]
async fn redirect_merges_query_and_keeps_fragment() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        app.create_ok(&s, "https://x.com/p?a=1#top", "Docs").await;
        let res = app.get(HOST_A, "/DOCS?b=2&b=3", None).await;
        assert_eq!(res.status, StatusCode::FOUND);
        assert_eq!(res.header("location"), "https://x.com/p?a=1&b=2&b=3#top");
        assert_eq!(
            app.get(HOST_B, "/docs", None).await.header("location"),
            "https://x.com/p?a=1#top"
        );
        assert_eq!(app.get(HOST_A, "/nope", None).await.status, StatusCode::NOT_FOUND);
        assert_eq!(app.get(HOST_A, "/docs/", None).await.status, StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn auto_slug_is_five_chars() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let id = app.create_ok(&s, "https://e.com/auto", "").await;
        let link = app.state.db.get(id).await.unwrap().unwrap();
        assert_eq!(link.slug.len(), 5);
        assert!(link.slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }
}

#[tokio::test]
async fn invalid_and_reserved_slugs_are_rejected() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        for slug in ["admin", "Admin", "static", "has_underscore", "has space"] {
            let res = app.create(&s, "https://e.com", slug).await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{slug}");
        }
        assert!(app
            .state
            .db
            .list(kurtz::db::ListFilter::default())
            .await
            .unwrap()
            .is_empty());
        // /admin always stays the admin
        assert!(app
            .get(HOST_A, "/admin", None)
            .await
            .text()
            .contains("type=\"password\""));
    }
}

#[tokio::test]
async fn slugs_are_unique_and_case_insensitive() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        app.create_ok(&s, "https://one.com", "x").await;
        let dup = app.create(&s, "https://two.com", "X").await;
        assert!(dup.text().contains("already used"), "{}", dup.text());
        assert_eq!(app.get(HOST_A, "/x", None).await.header("location"), "https://one.com");
    }
}

#[tokio::test]
async fn duplicate_target_warns_first() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        app.create_ok(&s, "https://same.com", "first").await;
        let form = [
            ("_csrf", s.csrf.as_str()),
            ("target", "https://same.com"),
            ("enabled", "on"),
        ];
        let warn = app.post(HOST_A, "/admin/links", Some(&s), &form).await;
        assert_eq!(warn.status, StatusCode::OK);
        assert!(warn.text().contains("already has a short link"));
        assert!(warn.text().contains("a.test/first"));
        assert_eq!(
            app.state.db.list(kurtz::db::ListFilter::default()).await.unwrap().len(),
            1
        );
        // confirming creates it
        let mut confirmed = form.to_vec();
        confirmed.push(("confirm_duplicate", "1"));
        assert_eq!(
            app.post(HOST_A, "/admin/links", Some(&s), &confirmed).await.status,
            StatusCode::SEE_OTHER
        );
        assert_eq!(
            app.state.db.list(kurtz::db::ListFilter::default()).await.unwrap().len(),
            2
        );
    }
}

#[tokio::test]
async fn toggle_disable_and_enable() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let id = app.create_ok(&s, "https://t.com", "tog").await;
        assert_eq!(app.get(HOST_A, "/tog", None).await.status, StatusCode::FOUND); // now cached
        let path = format!("/admin/links/{id}/toggle");
        app.post(HOST_A, &path, Some(&s), &[("_csrf", &s.csrf)]).await;
        assert_eq!(app.get(HOST_A, "/tog", None).await.status, StatusCode::NOT_FOUND);
        let link = app.state.db.get(id).await.unwrap().unwrap();
        assert!(!link.enabled);
        assert!(link.updated_at >= link.created_at);
        app.post(HOST_A, &path, Some(&s), &[("_csrf", &s.csrf)]).await;
        assert_eq!(app.get(HOST_A, "/tog", None).await.status, StatusCode::FOUND);
    }
}

#[tokio::test]
async fn edits_invalidate_the_cache() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let id = app.create_ok(&s, "https://old.com", "c").await;
        assert_eq!(app.get(HOST_A, "/c", None).await.header("location"), "https://old.com");
        let res = app
            .post(
                HOST_A,
                &format!("/admin/links/{id}/edit"),
                Some(&s),
                &[("_csrf", &s.csrf), ("target", "https://new.com"), ("enabled", "on")],
            )
            .await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.text());
        assert_eq!(app.get(HOST_A, "/c", None).await.header("location"), "https://new.com");
    }
}

#[tokio::test]
async fn delete_removes_link() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let id = app.create_ok(&s, "https://d.com", "del").await;
        assert!(app
            .get(HOST_A, &format!("/admin/links/{id}/delete"), Some(&s))
            .await
            .text()
            .contains("Delete link"));
        app.post(
            HOST_A,
            &format!("/admin/links/{id}/delete"),
            Some(&s),
            &[("_csrf", &s.csrf)],
        )
        .await;
        assert_eq!(app.get(HOST_A, "/del", None).await.status, StatusCode::NOT_FOUND);
        assert!(app.state.db.get(id).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn last_used_is_batched_and_skips_head() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let id = app.create_ok(&s, "https://lu.com", "lu").await;

        let head = Request::builder()
            .method(Method::HEAD)
            .uri("/lu")
            .header(header::HOST, HOST_A)
            .body(Body::empty())
            .unwrap();
        assert_eq!(app.send(head).await.status, StatusCode::FOUND);
        assert!(app.state.last_used.is_empty(), "HEAD must not count");

        app.get(HOST_A, "/lu", None).await;
        // recorded in memory, not yet in the DB
        assert_eq!(app.state.last_used.len(), 1);
        assert!(app.state.db.get(id).await.unwrap().unwrap().last_used_at.is_none());

        assert_eq!(app.state.flush_last_used().await.unwrap(), 1);
        let link = app.state.db.get(id).await.unwrap().unwrap();
        assert!(link.last_used_at.is_some());
        assert_eq!(link.updated_at, link.created_at, "last-used must not touch updated_at");

        // a 404 does not count
        app.get(HOST_A, "/missing", None).await;
        assert!(app.state.last_used.is_empty());
    }
}

#[tokio::test]
async fn qr_is_admin_only() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let id = app.create_ok(&s, "https://qr.com", "qr1").await;
        let path = format!("/admin/links/{id}/qr?format=png");
        assert_eq!(app.get(HOST_A, &path, None).await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(app.get(HOST_A, "/qr1/qr", None).await.status, StatusCode::NOT_FOUND);

        let png = app.get(HOST_A, &path, Some(&s)).await;
        assert_eq!(png.status, StatusCode::OK);
        assert_eq!(png.header("content-type"), "image/png");
        let decoder = png::Decoder::new(std::io::Cursor::new(png.body.as_slice()));
        let reader = decoder.read_info().unwrap();
        assert_eq!(reader.info().width, 512);

        let svg = app
            .get(HOST_A, &format!("/admin/links/{id}/qr?format=svg&download=1"), Some(&s))
            .await;
        assert_eq!(svg.header("content-type"), "image/svg+xml");
        assert_eq!(svg.header("content-disposition"), "attachment; filename=\"qr1.svg\"");
        assert!(svg.text().contains("fill=\"#000\""));
    }
}

#[tokio::test]
async fn list_search_and_detail() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let id = app.create_ok(&s, "https://rust-lang.org", "rust").await;
        app.create_ok(&s, "https://example.com/Docs", "other").await;
        let list = app.get(HOST_A, "/admin?q=RUST", Some(&s)).await.text();
        assert!(list.contains(">/rust</a>"));
        assert!(!list.contains(">/other</a>"));
        let list = app.get(HOST_A, "/admin?q=docs", Some(&s)).await.text();
        assert!(list.contains(">/other</a>"));
        let list = app.get(HOST_A, "/admin?q=%25", Some(&s)).await.text();
        assert!(list.contains("No links match"));
        let detail = app.get(HOST_A, &format!("/admin/links/{id}"), Some(&s)).await.text();
        assert!(detail.contains("http://a.test/rust"));
        assert!(detail.contains("Last used"));
    }
}

#[tokio::test]
async fn htmx_requests_get_fragments_and_hx_redirect() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let form = app
            .send(
                Request::builder()
                    .uri("/admin/links/new")
                    .header(header::HOST, HOST_A)
                    .header(header::COOKIE, &s.cookie)
                    .header("hx-request", "true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(form.text().starts_with("<form"), "modal gets a fragment");
        let res = app
            .post_with(
                HOST_A,
                "/admin/links",
                Some(&s),
                &[("_csrf", &s.csrf), ("target", "https://hx.com"), ("enabled", "on")],
                &[("hx-request", "true")],
            )
            .await;
        assert_eq!(res.status, StatusCode::OK);
        assert_eq!(res.header("hx-redirect"), "/admin");
        let invalid = app
            .post_with(
                HOST_A,
                "/admin/links",
                Some(&s),
                &[("_csrf", &s.csrf), ("target", "nope")],
                &[("hx-request", "true")],
            )
            .await;
        assert_eq!(invalid.status, StatusCode::OK);
        assert!(invalid.text().contains("not a valid URL"));
    }
}

fn multipart(csrf: &str, action: &str, conflict: &str, csv: &str) -> (String, String) {
    let b = "XKURTZBOUNDARY";
    let mut body = String::new();
    for (name, value) in [("_csrf", csrf), ("action", action), ("conflict", conflict)] {
        body.push_str(&format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ));
    }
    body.push_str(&format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"l.csv\"\r\nContent-Type: text/csv\r\n\r\n{csv}\r\n--{b}--\r\n"
    ));
    (format!("multipart/form-data; boundary={b}"), body)
}

async fn upload(app: &TestApp, s: &Session, action: &str, conflict: &str, csv: &str) -> Res {
    let (ct, body) = multipart(&s.csrf, action, conflict, csv);
    app.send(
        Request::builder()
            .method(Method::POST)
            .uri("/admin/import")
            .header(header::HOST, HOST_A)
            .header(header::COOKIE, &s.cookie)
            .header(header::CONTENT_TYPE, ct)
            .body(Body::from(body))
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn csv_export_import_round_trip() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        app.create_ok(&s, "https://one.com/?q=1,2", "x").await;
        app.create_tagged(&s, "https://two.com", "y", "team_a, docs").await;
        let id = app.create_ok(&s, "https://all.com", "all").await;
        app.post(
            HOST_A,
            &format!("/admin/links/{id}/toggle"),
            Some(&s),
            &[("_csrf", &s.csrf)],
        )
        .await;
        app.get(HOST_A, "/x", None).await;
        app.state.flush_last_used().await.unwrap();

        let before = app.state.db.list(kurtz::db::ListFilter::default()).await.unwrap();
        let export = app.get(HOST_A, "/admin/export.csv", Some(&s)).await;
        assert_eq!(
            export.header("content-type"),
            "text/csv; charset=utf-8",
            "{} {}",
            export.status,
            export.text()
        );
        let csv = export.text();
        assert_eq!(csv.lines().count(), 4);

        wipe(&app.state.db).await;
        let preview = upload(&app, &s, "preview", "skip", &csv).await;
        assert!(preview.text().contains("Preview"), "{}", preview.text());
        assert!(
            app.state
                .db
                .list(kurtz::db::ListFilter::default())
                .await
                .unwrap()
                .is_empty(),
            "preview must not save"
        );

        let done = upload(&app, &s, "import", "skip", &csv).await;
        assert!(done.text().contains("Import finished"));
        let after = app.state.db.list(kurtz::db::ListFilter::default()).await.unwrap();
        assert_eq!(after.len(), 3);
        for (a, b) in before.iter().zip(after.iter()) {
            assert_eq!(
                (&a.slug, &a.target_url, a.enabled, &a.tags),
                (&b.slug, &b.target_url, b.enabled, &b.tags)
            );
            assert_eq!(
                (a.created_at, a.updated_at, a.last_used_at),
                (b.created_at, b.updated_at, b.last_used_at)
            );
        }
        assert_eq!(app.get(HOST_B, "/y", None).await.header("location"), "https://two.com");

        // importing again skips everything
        let again = upload(&app, &s, "import", "skip", &csv).await.text();
        assert!(
            again.contains("Skipped (slug already in use)</dt><dd class=\"col-4 mb-0\">3</dd>"),
            "{again}"
        );
        // overwrite replaces
        let over = upload(
            &app,
            &s,
            "import",
            "overwrite",
            "slug,target_url\nall,https://replaced.com\n",
        )
        .await
        .text();
        assert!(
            over.contains("Overwritten</dt><dd class=\"col-4 mb-0\">1</dd>"),
            "{over}"
        );
        assert_eq!(
            app.get(HOST_A, "/all", None).await.header("location"),
            "https://replaced.com"
        );
    }
}

#[tokio::test]
async fn csv_import_is_all_or_nothing() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let res = upload(
            &app,
            &s,
            "import",
            "skip",
            "slug,target_url\ngood,https://g.com\nbad slug,https://b.com\n",
        )
        .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(res.text().contains("Line 3"));
        assert!(app
            .state
            .db
            .list(kurtz::db::ListFilter::default())
            .await
            .unwrap()
            .is_empty());
        // same slug twice inside one file
        let res = upload(
            &app,
            &s,
            "import",
            "overwrite",
            "slug,target_url\nsame,https://1.com\nsame,https://2.com\n",
        )
        .await;
        assert_eq!(res.status, StatusCode::OK);
        assert_eq!(
            app.state.db.list(kurtz::db::ListFilter::default()).await.unwrap().len(),
            1
        );
    }
}

#[tokio::test]
async fn root_redirect_and_static_assets() {
    for url in backends() {
        let app = setup(&url, &[("ROOT_REDIRECT_URL", "https://home.example")]).await;
        assert_eq!(
            app.get(HOST_A, "/", None).await.header("location"),
            "https://home.example"
        );
        let htmx = app.state.assets.htmx.path.clone();
        let res = app.get(HOST_A, &htmx, None).await;
        assert_eq!(res.status, StatusCode::OK);
        assert!(res.header("cache-control").contains("immutable"));
        assert_eq!(
            app.get(HOST_A, "/static/app.0000000000.js", None).await.status,
            StatusCode::NOT_FOUND
        );

        // Tabler: pinned CDN copy with an SRI hash in development, the embedded
        // (already hash-verified) file in release images.
        let (href, integrity) = app.state.assets.stylesheet();
        let page = app.get(HOST_A, "/admin", None).await.text();
        assert!(page.contains(&href));
        match integrity {
            Some(hash) => {
                assert!(hash.starts_with("sha384-"));
                assert!(href.contains("/@tabler/core@") && !href.contains("@latest"));
                assert!(page.contains(&format!("integrity=\"{hash}\"")));
            }
            None => assert_eq!(app.get(HOST_A, &href, None).await.status, StatusCode::OK),
        }

        // The script follows the same rule as the stylesheet.
        let (src, integrity) = app.state.assets.tabler_script();
        assert!(page.contains(&src));
        match integrity {
            Some(hash) => {
                assert!(hash.starts_with("sha384-"));
                assert!(src.contains("/@tabler/core@") && !src.contains("@latest"));
                assert!(page.contains(&format!("integrity=\"{hash}\"")));
            }
            None => assert_eq!(app.get(HOST_A, &src, None).await.status, StatusCode::OK),
        }
    }
}

#[tokio::test]
async fn tags_create_filter_search_and_edit() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let id = app.create_tagged(&s, "https://docs.rs", "docs", "Docs, Rust").await;
        assert_eq!(id.status, StatusCode::SEE_OTHER, "{}", id.text());
        let id = app.id_of("docs").await;
        app.create_tagged(&s, "https://blog.example", "blog", "marketing").await;
        app.create_tagged(&s, "https://plain.example", "plain", "").await;

        let link = app.state.db.get(id).await.unwrap().unwrap();
        assert_eq!(link.tags, vec!["docs", "rust"]);
        assert_eq!(
            app.state.db.tags().await.unwrap(),
            vec![("docs".into(), 1), ("marketing".into(), 1), ("rust".into(), 1)]
        );

        // filter by tag
        let list = app.get(HOST_A, "/admin?tag=rust", Some(&s)).await.text();
        assert!(list.contains(">/docs</a>") && !list.contains(">/blog</a>") && !list.contains(">/plain</a>"));
        assert!(list.contains("<option value=\"rust\" selected>"));
        // search matches tags too
        let list = app.get(HOST_A, "/admin?q=market", Some(&s)).await.text();
        assert!(list.contains(">/blog</a>") && !list.contains(">/docs</a>"));
        // search and tag combine
        assert!(app
            .get(HOST_A, "/admin?q=blog&tag=rust", Some(&s))
            .await
            .text()
            .contains("No links match"));

        // the form offers existing tags for autosuggest
        let form = app.get(HOST_A, "/admin/links/new", Some(&s)).await.text();
        assert!(form.contains("data-suggestions=\"docs,marketing,rust\""));

        // edit replaces tags; invalid tags are rejected
        let path = format!("/admin/links/{id}/edit");
        let bad = app
            .post(
                HOST_A,
                &path,
                Some(&s),
                &[
                    ("_csrf", &s.csrf),
                    ("target", "https://docs.rs"),
                    ("tags", "ok, not#ok"),
                    ("enabled", "on"),
                ],
            )
            .await;
        assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
        app.post(
            HOST_A,
            &path,
            Some(&s),
            &[
                ("_csrf", &s.csrf),
                ("target", "https://docs.rs"),
                ("tags", "reference"),
                ("enabled", "on"),
            ],
        )
        .await;
        assert_eq!(app.state.db.get(id).await.unwrap().unwrap().tags, vec!["reference"]);

        // deleting a link removes its tags
        app.post(
            HOST_A,
            &format!("/admin/links/{id}/delete"),
            Some(&s),
            &[("_csrf", &s.csrf)],
        )
        .await;
        assert_eq!(app.state.db.tags().await.unwrap(), vec![("marketing".into(), 1)]);
    }
}

#[tokio::test]
async fn shlink_export_imports_with_legacy_slugs_and_reports_skips() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        let s = app.login(HOST_A).await;
        let csv = "\"createdAt\",\"domain\",\"shortCode\",\"shortUrl\",\"longUrl\",\"title\",\"tags\",\"visits\"\n\
\"2025-06-02T10:00:00+02:00\",\"a.test\",\"okr\",\"https://a.test/okr\",\"https://e.com/new\",\"\",\"service\",49\n\
\"2023-08-23T10:00:00+02:00\",\"a.test\",\"OKR\",\"https://a.test/OKR\",\"https://e.com/old\",\"\",\"blog|okr\",140\n\
\"2023-03-22T08:02:59+01:00\",\"a.test\",\"dp_stammtisch\",\"https://a.test/dp_stammtisch\",\"https://e.com/stammtisch\",\"\",\"\",3\n\
\"2023-03-22T08:02:59+01:00\",\"b.test\",\"Jurybegr%C3%BCndung\",\"https://b.test/x\",\"https://e.com/jury\",\"\",\"\",0\n";
        let preview = upload(&app, &s, "preview", "skip", csv).await.text();
        assert!(
            preview.contains("Ignored columns: domain, shortUrl, title, visits."),
            "{preview}"
        );
        assert!(
            preview.contains("Line 3: /okr is already used by line 2 of this file."),
            "{preview}"
        );
        let done = upload(&app, &s, "import", "skip", csv).await.text();
        assert!(done.contains("New</dt><dd class=\"col-4 mb-0\">3</dd>"), "{done}");

        assert_eq!(
            app.get(HOST_A, "/OKR", None).await.header("location"),
            "https://e.com/new"
        );
        assert_eq!(
            app.get(HOST_A, "/dp_stammtisch", None).await.header("location"),
            "https://e.com/stammtisch"
        );
        assert_eq!(
            app.get(HOST_B, "/Jurybegr%C3%BCndung", None).await.header("location"),
            "https://e.com/jury"
        );
        assert_eq!(
            app.get(HOST_B, "/JURYBEGR%C3%9CNDUNG", None).await.header("location"),
            "https://e.com/jury"
        );
    }
}

#[tokio::test]
async fn health_reports_pending_migrations() {
    for url in backends() {
        let app = setup(&url, &[]).await;
        assert_eq!(app.get(HOST_A, "/_health", None).await.status, StatusCode::OK);
        // pretend the newest migration was never applied
        match &app.state.db {
            kurtz::db::Db::Pg(p) => {
                sqlx::query("UPDATE _sqlx_migrations SET success = false WHERE version = (SELECT MAX(version) FROM _sqlx_migrations)")
                    .execute(p)
                    .await
                    .unwrap();
            }
            kurtz::db::Db::My(p) => {
                sqlx::query("UPDATE _sqlx_migrations m JOIN (SELECT MAX(version) v FROM _sqlx_migrations) x ON m.version = x.v SET m.success = false")
                    .execute(p)
                    .await
                    .unwrap();
            }
        }
        let res = app.get(HOST_A, "/_health", None).await;
        let restore = "UPDATE _sqlx_migrations SET success = true";
        match &app.state.db {
            kurtz::db::Db::Pg(p) => sqlx::query(restore).execute(p).await.map(|_| ()).unwrap(),
            kurtz::db::Db::My(p) => sqlx::query(restore).execute(p).await.map(|_| ()).unwrap(),
        }
        assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(res.text(), "NOT_OK\n");
    }
}
