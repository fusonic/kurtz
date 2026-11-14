//! Every environment variable the code reads must be documented in the README.

fn quoted_upper_names(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_ascii_uppercase() || bytes[j] == '_') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == '"' && j - start >= 4 && bytes[start..j].contains(&'_') {
                out.push(bytes[start..j].iter().collect());
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

#[test]
fn every_env_var_is_documented() {
    let readme = include_str!("../README.md");
    let mut names = quoted_upper_names(include_str!("../src/config.rs"));
    names.extend(quoted_upper_names(include_str!("../src/main.rs")));
    names.retain(|n| !n.starts_with("CARGO_")); // compile-time, not runtime config
    assert!(
        names.contains(&"DATABASE_URL".to_string()),
        "scanner found nothing: {names:?}"
    );
    for name in names {
        assert!(
            readme.contains(&format!("`{name}`")),
            "{name} is read by the code but not documented in README.md"
        );
    }
}
