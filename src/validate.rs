//! Input validation and normalisation for slugs, target URLs and tags.

use sha2::{Digest, Sha256};

pub const MAX_SLUG_LEN: usize = 64;
pub const MAX_TARGET_LEN: usize = 2048;
/// Slugs that would collide with Kurtz's own routes.
pub const RESERVED_SLUGS: &[&str] = &["admin", "static", "_health"];
const SLUG_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// Lowercases and checks a slug typed by the user.
pub fn slug(input: &str) -> Result<String, String> {
    let s = input.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Err("Slug is empty.".into());
    }
    if s.len() > MAX_SLUG_LEN {
        return Err(format!("Slug is longer than {MAX_SLUG_LEN} characters."));
    }
    if !s.chars().all(is_slug_char) {
        return Err("Slug may only contain a-z, 0-9 and hyphens.".into());
    }
    if RESERVED_SLUGS.contains(&s.as_str()) {
        return Err(format!("“{s}” is reserved and cannot be used as a slug."));
    }
    Ok(s)
}

/// Fast check used on the redirect path: could this (lowercased) path
/// segment be a slug? Includes imported legacy slugs (see [`import_slug`]).
pub fn looks_like_slug(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= MAX_SLUG_LEN && s.chars().all(is_legacy_slug_char)
}

fn is_legacy_slug_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

/// Slugs from imports. Links created elsewhere may already be printed or
/// shared, so their slugs are kept even when they are outside the stricter
/// rule for new slugs: percent-encoding is decoded, case is folded, and
/// letters/digits of any script, `-` and `_` are accepted.
pub fn import_slug(input: &str) -> Result<String, String> {
    let raw = input.trim();
    let decoded = percent_decode(raw).ok_or_else(|| format!("Slug “{raw}” is not valid percent-encoding."))?;
    let s = decoded.to_lowercase();
    if s.is_empty() {
        return Err("Slug is empty.".into());
    }
    if s.chars().count() > MAX_SLUG_LEN {
        return Err(format!("Slug “{s}” is longer than {MAX_SLUG_LEN} characters."));
    }
    if !s.chars().all(is_legacy_slug_char) {
        return Err(format!("Slug “{s}” may only contain letters, digits, - and _."));
    }
    if RESERVED_SLUGS.contains(&s.as_str()) {
        return Err(format!("“{s}” is reserved and cannot be used as a slug."));
    }
    Ok(s)
}

fn percent_decode(s: &str) -> Option<String> {
    if !s.contains('%') {
        return Some(s.to_string());
    }
    let bytes: Vec<u8> = url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned().into_bytes())?;
    String::from_utf8(bytes).ok()
}

fn is_slug_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

/// Random slug from `a-z0-9`.
pub fn random_slug(len: usize) -> String {
    // Rejection sampling keeps every character equally likely.
    let limit = 256 - 256 % SLUG_ALPHABET.len();
    let mut slug = String::with_capacity(len);
    let mut bytes = [0u8; 32];
    while slug.len() < len {
        getrandom::fill(&mut bytes).expect("the operating system provides random numbers");
        for &b in bytes.iter().filter(|&&b| (b as usize) < limit) {
            if slug.len() < len {
                slug.push(SLUG_ALPHABET[b as usize % SLUG_ALPHABET.len()] as char);
            }
        }
    }
    slug
}

/// Checks a target URL. The URL is stored verbatim (trimmed), including its
/// query string and fragment.
pub fn target_url(input: &str) -> Result<String, String> {
    let t = input.trim();
    if t.is_empty() {
        return Err("Target URL is empty.".into());
    }
    if t.len() > MAX_TARGET_LEN {
        return Err(format!("Target URL is longer than {MAX_TARGET_LEN} characters."));
    }
    // Spaces are common in pasted file URLs; browsers send them as %20 anyway.
    let t = &t.replace(' ', "%20");
    if t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("Target URL must not contain spaces or control characters.".into());
    }
    let parsed = url::Url::parse(t).map_err(|_| "Target URL is not a valid URL.".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Target URL must start with http:// or https://.".into());
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err("Target URL has no host.".into());
    }
    Ok(t.to_string())
}

/// SHA-256 of the target, used for the duplicate check.
pub fn target_hash(target: &str) -> String {
    to_hex(&Sha256::digest(target.as_bytes()))
}

pub const MAX_TAG_LEN: usize = 32;
pub const MAX_TAGS: usize = 10;

/// Normalises one tag: trimmed, inner whitespace collapsed, lowercase.
/// Allowed: letters, digits, spaces, `-`, `_`, `.`.
pub fn tag(input: &str) -> Result<String, String> {
    let t = input.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    if t.is_empty() {
        return Err("Tag is empty.".into());
    }
    if t.chars().count() > MAX_TAG_LEN {
        return Err(format!("Tag “{t}” is longer than {MAX_TAG_LEN} characters."));
    }
    if !t
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
    {
        return Err(format!("Tag “{t}” may only contain letters, digits, spaces, - _ and ."));
    }
    Ok(t)
}

/// Parses a separated list of tags (`,`, `;` or `|`), removes duplicates and sorts.
pub fn tags(input: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for part in input.split([',', ';', '|']) {
        if part.trim().is_empty() {
            continue;
        }
        let t = tag(part)?;
        if !out.contains(&t) {
            out.push(t);
        }
    }
    if out.len() > MAX_TAGS {
        return Err(format!("A link can have at most {MAX_TAGS} tags."));
    }
    out.sort();
    Ok(out)
}

/// Lowercase hex encoding.
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}
