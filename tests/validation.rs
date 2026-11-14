use kurtz::{redirect::merge_query, validate};

#[test]
fn slug_is_lowercased_and_checked() {
    assert_eq!(validate::slug(" My-Link ").unwrap(), "my-link");
    assert_eq!(validate::slug("abc123").unwrap(), "abc123");
    assert!(validate::slug("").is_err());
    assert!(validate::slug("with_underscore").is_err());
    assert!(validate::slug("with space").is_err());
    assert!(validate::slug("dot.ted").is_err());
    assert!(validate::slug("ümlaut").is_err());
    assert!(validate::slug(&"a".repeat(65)).is_err());
    assert!(validate::slug(&"a".repeat(64)).is_ok());
}

#[test]
fn reserved_slugs_are_rejected() {
    assert!(validate::import_slug("_health").is_err());
    for r in ["admin", "ADMIN", "static"] {
        assert!(validate::slug(r).is_err(), "{r} should be reserved");
    }
}

#[test]
fn random_slugs_use_the_alphabet_and_length() {
    for _ in 0..200 {
        let s = validate::random_slug(5);
        assert_eq!(s.len(), 5);
        assert!(s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }
}

#[test]
fn target_urls_are_validated_and_kept_verbatim() {
    let t = "https://example.com/p?a=1&a=2#frag";
    assert_eq!(validate::target_url(&format!("  {t} ")).unwrap(), t);
    assert!(validate::target_url("ftp://example.com").is_err());
    assert!(validate::target_url("javascript:alert(1)").is_err());
    assert!(validate::target_url("example.com").is_err());
    assert!(validate::target_url("https://exa mple.com").is_err());
    assert!(validate::target_url(&format!("https://example.com/{}", "a".repeat(2048))).is_err());
}

#[test]
fn query_is_passed_through() {
    assert_eq!(merge_query("https://x.com/p", None), "https://x.com/p");
    assert_eq!(merge_query("https://x.com/p", Some("")), "https://x.com/p");
    assert_eq!(merge_query("https://x.com/p", Some("b=2")), "https://x.com/p?b=2");
    assert_eq!(
        merge_query("https://x.com/p?a=1#top", Some("b=2")),
        "https://x.com/p?a=1&b=2#top"
    );
    assert_eq!(merge_query("https://x.com/p?", Some("b=2")), "https://x.com/p?b=2");
    assert_eq!(
        merge_query("https://x.com/p?a=1&", Some("b=2")),
        "https://x.com/p?a=1&b=2"
    );
    // order, duplicates and encoding untouched
    assert_eq!(
        merge_query("https://x.com/", Some("z=1&a=%20x&a=2&utm=%C3%BC")),
        "https://x.com/?z=1&a=%20x&a=2&utm=%C3%BC"
    );
}

#[test]
fn hosts_are_normalized() {
    use kurtz::web::normalize_host;
    assert_eq!(normalize_host("A.Test:8080"), "a.test");
    assert_eq!(normalize_host("a.test."), "a.test");
    assert_eq!(normalize_host("[::1]:8080"), "::1");
    assert_eq!(normalize_host("a.test, proxy.internal"), "a.test");
}

#[test]
fn tags_are_normalized() {
    assert_eq!(validate::tag("  Team   A ").unwrap(), "team a");
    assert_eq!(validate::tag("Österreich").unwrap(), "österreich");
    assert_eq!(validate::tag("v1.2_beta-x").unwrap(), "v1.2_beta-x");
    assert!(validate::tag("").is_err());
    assert!(validate::tag("no#hash").is_err());
    assert!(validate::tag(&"a".repeat(33)).is_err());
    assert_eq!(
        validate::tags("b, a; A ,, b").unwrap(),
        vec!["a".to_string(), "b".to_string()]
    );
    assert!(validate::tags("").unwrap().is_empty());
    let eleven: Vec<String> = (0..11).map(|i| format!("t{i}")).collect();
    assert!(validate::tags(&eleven.join(",")).is_err());
}

#[test]
fn imported_slugs_are_more_lenient() {
    assert_eq!(validate::import_slug("dp_stammtisch").unwrap(), "dp_stammtisch");
    assert_eq!(validate::import_slug("Jurybegr%C3%BCndung").unwrap(), "jurybegründung");
    assert_eq!(validate::import_slug("MudBlazor").unwrap(), "mudblazor");
    assert!(validate::import_slug("admin").is_err());
    assert!(validate::import_slug("with space").is_err());
    assert!(validate::import_slug("a/b").is_err());
    assert!(validate::import_slug("%FF").is_err());
    // new slugs stay strict
    assert!(validate::slug("dp_stammtisch").is_err());
    assert!(validate::looks_like_slug("jurybegründung") && validate::looks_like_slug("dp_x"));
}

#[test]
fn spaces_in_targets_are_encoded_and_tags_split_on_pipes() {
    assert_eq!(
        validate::target_url("https://e.com/My File.pdf").unwrap(),
        "https://e.com/My%20File.pdf"
    );
    assert_eq!(validate::tags("b|a").unwrap(), vec!["a".to_string(), "b".to_string()]);
}
