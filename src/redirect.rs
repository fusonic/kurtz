//! Building the redirect location.

/// Appends the incoming query string to the target's own query and keeps the
/// target's fragment. Parameters are passed through exactly as received:
/// order, duplicates and encoding are untouched.
pub fn merge_query(target: &str, incoming: Option<&str>) -> String {
    let incoming = incoming.unwrap_or("");
    if incoming.is_empty() {
        return target.to_string();
    }
    let (base, fragment) = match target.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (target, None),
    };
    let mut out = String::with_capacity(target.len() + incoming.len() + 2);
    out.push_str(base);
    if !base.contains('?') {
        out.push('?');
    } else if !base.ends_with('?') && !base.ends_with('&') {
        out.push('&');
    }
    out.push_str(incoming);
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}
