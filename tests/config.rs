use std::collections::HashMap;

use kurtz::config::Config;

fn cfg(vars: &[(&str, &str)]) -> Result<Config, String> {
    let mut m: HashMap<String, String> = HashMap::from([
        ("DATABASE_URL".into(), "postgres://x".into()),
        ("ADMIN_PASSWORD".into(), kurtz::auth::hash_password("pw").unwrap()),
        ("SESSION_SECRET".into(), "s".repeat(64)),
    ]);
    for (k, v) in vars {
        m.insert(k.to_string(), v.to_string());
    }
    Config::from_lookup(|k| m.get(k).cloned())
}

#[test]
fn defaults() {
    let c = cfg(&[]).unwrap();
    assert_eq!(c.slug_length, 5);
    assert_eq!(c.cache_size, 1000);
    assert_eq!(c.last_used_flush_secs, 60);
    assert_eq!(c.log_level, "info");
    assert!(c.cookie_secure);
    assert!(kurtz::auth::verify_password(&c.admin_password_hash, "pw"));
}

#[test]
fn required_and_invalid_values() {
    assert!(cfg(&[("SESSION_SECRET", "short")]).is_err());
    assert!(cfg(&[("ADMIN_PASSWORD", "")]).is_err());
    assert!(cfg(&[("ADMIN_PASSWORD", "not-a-hash")]).is_err());
    assert!(cfg(&[("ADMIN_PASSWORD", "plain-password")]).is_err());
    assert!(cfg(&[("SLUG_LENGTH", "0")]).is_err());
    assert!(cfg(&[("ROOT_REDIRECT_URL", "ftp://x")]).is_err());
    assert!(cfg(&[("LOG_LEVEL", "verbose")]).is_err());
    assert_eq!(cfg(&[("LOG_LEVEL", "DEBUG")]).unwrap().log_level, "debug");
}
