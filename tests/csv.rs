use chrono::{TimeZone, Utc};
use kurtz::{
    csvio::{export, parse, COLUMNS},
    db::Link,
};

#[test]
fn export_then_parse_round_trips() {
    let t = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
    let links = vec![
        Link {
            id: 1,
            slug: "x".into(),
            target_url: "https://e.com/?a=1,2#f".into(),
            enabled: true,
            created_at: t,
            updated_at: t,
            last_used_at: Some(t),
            tags: vec!["docs".into(), "team_a".into()],
        },
        Link {
            id: 2,
            slug: "y".into(),
            target_url: "https://other.com/\"quoted\"".into(),
            enabled: false,
            created_at: t,
            updated_at: t,
            last_used_at: None,
            tags: vec![],
        },
        Link {
            id: 3,
            slug: "all".into(),
            target_url: "https://all.com".into(),
            enabled: true,
            created_at: t,
            updated_at: t,
            last_used_at: None,
            tags: vec!["unicode_1".into()],
        },
    ];
    let bytes = export(&links).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.starts_with("slug,target_url,tags,enabled,created_at,updated_at,last_used_at\n"));
    let rows = parse(&bytes).unwrap().rows;
    assert_eq!(rows.len(), 3);
    for (row, l) in rows.iter().zip(&links) {
        assert_eq!(row.link.slug, l.slug);
        assert_eq!(row.link.target_url, l.target_url);
        assert_eq!(row.link.enabled, l.enabled);
        assert_eq!(row.link.tags, l.tags);
        assert_eq!(row.link.created_at, l.created_at);
        assert_eq!(row.link.last_used_at, l.last_used_at);
    }
}

#[test]
fn minimal_columns_bom_and_crlf() {
    let bytes = b"\xEF\xBB\xBFslug,target_url\r\nAbc,https://e.com\r\n";
    let rows = parse(bytes).unwrap().rows;
    assert_eq!(rows[0].link.slug, "abc");
    assert!(rows[0].link.enabled);
}

#[test]
fn errors_name_the_line() {
    let bytes = b"slug,target_url\nok,https://e.com\nbad slug,https://e.com\nfine,ftp://x\nz,https://e.com\n";
    let errs = parse(bytes).unwrap_err();
    let lines: Vec<u64> = errs.iter().map(|e| e.line).collect();
    assert_eq!(lines, vec![3, 4]);
}

#[test]
fn header_must_have_required_columns() {
    assert!(parse(b"foo,bar\n1,2\n").is_err());
}

#[test]
fn tags_are_runs_of_letters_digits_and_underscores() {
    let tags = |cell: &str| {
        let csv = format!("slug,target_url,tags\nx,https://e.com,\"{cell}\"\n");
        parse(csv.as_bytes()).map(|p| p.rows[0].link.tags.clone())
    };
    assert_eq!(tags("Docs; Team  A ;docs").unwrap(), vec!["a", "docs", "team"]);
    assert_eq!(
        tags("bad#tag|Foo_1,x-y").unwrap(),
        vec!["bad", "foo_1", "tag", "x", "y"]
    );
    assert!(tags("").unwrap().is_empty());
    let eleven: Vec<String> = (0..11).map(|i| format!("t{i}")).collect();
    assert_eq!(tags(&eleven.join(",")).unwrap_err()[0].line, 2);
    assert!(tags(&"a".repeat(33)).is_err());
}

#[test]
fn headers_match_in_any_casing_but_only_once() {
    let ok = parse(b"ShortCode,longUrl,Last_Used_At\nabc,https://e.com,2026-01-02T03:04:05Z\n").unwrap();
    assert_eq!(ok.rows[0].link.slug, "abc");
    assert!(ok.rows[0].link.last_used_at.is_some());
    let err = parse(b"slug,code,target_url\nabc,abc,https://e.com\n").unwrap_err();
    assert_eq!(err[0].line, 1);
    assert!(
        err[0].message.contains("Several columns match slug: slug, code"),
        "{err:?}"
    );
    assert!(parse(b"slug,target_url,URL\nabc,https://e.com,https://e.com\n").is_err());
}

#[test]
fn header_help_lists_every_casing() {
    let slug = COLUMNS.iter().find(|c| c.key == "slug").unwrap();
    assert!(slug.required);
    let headers = slug.headers();
    for name in ["slug", "Slug", "code", "short_code", "shortCode", "ShortCode"] {
        assert!(headers.split(", ").any(|h| h == name), "{name} missing in {headers}");
    }
    assert!(!COLUMNS.iter().find(|c| c.key == "tags").unwrap().required);
}

#[test]
fn shlink_export_is_understood() {
    let csv = "\"createdAt\",\"domain\",\"shortCode\",\"shortUrl\",\"longUrl\",\"title\",\"tags\",\"visits\"\n\
\"2026-10-14T13:41:46+02:00\",\"a.test\",\"Case-Study\",\"https://a.test/Case-Study\",\"https://e.com/p?x=1\",\"Some title\",\"casestudy|award\",21\n\
\"2023-03-22T08:02:59+01:00\",\"DEFAULT\",\"dp_stammtisch\",\"https://a.test/dp_stammtisch\",\"https://e.com/files/My File 2023.pdf\",\"\",\"\",3\n\
\"2023-03-22T08:02:59+01:00\",\"b.test\",\"Jurybegr%C3%BCndung\",\"https://b.test/x\",\"https://e.com/jury\",\"\",\"\",0\n";
    let parsed = parse(csv.as_bytes()).unwrap();
    assert_eq!(parsed.ignored_columns, vec!["domain", "shortUrl", "title", "visits"]);
    let r = &parsed.rows;
    assert_eq!(r[0].link.slug, "case-study");
    assert_eq!(r[0].link.tags, vec!["award", "casestudy"]);
    assert_eq!(
        r[0].link.created_at,
        Utc.with_ymd_and_hms(2026, 10, 14, 11, 41, 46).unwrap()
    );
    assert_eq!(r[1].link.slug, "dp_stammtisch");
    assert_eq!(r[1].link.target_url, "https://e.com/files/My%20File%202023.pdf");
    assert_eq!(r[2].link.slug, "jurybegründung");
    assert_eq!(r[2].line, 4);
}
