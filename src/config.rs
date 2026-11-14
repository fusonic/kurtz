//! Configuration from environment variables. Every variable read here is
//! documented in the README.

use crate::auth;

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    /// Argon2 PHC string of the admin password.
    pub admin_password_hash: String,
    pub session_secret: Vec<u8>,
    pub slug_length: usize,
    pub root_redirect_url: Option<String>,
    /// `error`, `warn`, `info` (default) or `debug`.
    pub log_level: String,
    pub cookie_secure: bool,
    pub cache_size: usize,
    pub last_used_flush_secs: u64,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Builds the configuration from any key/value source (used by tests).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let var = |key: &str| get(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());

        let database_url = var("DATABASE_URL").ok_or("DATABASE_URL is required")?;
        let admin_password_hash = var("ADMIN_PASSWORD").ok_or("ADMIN_PASSWORD is required")?;
        auth::check_hash_format(&admin_password_hash)
            .map_err(|e| format!("ADMIN_PASSWORD must be an Argon2 hash, not the plain password: {e}"))?;

        let session_secret = var("SESSION_SECRET").ok_or("SESSION_SECRET is required")?;
        if session_secret.len() < 64 {
            return Err("SESSION_SECRET must be at least 64 characters (e.g. `openssl rand -hex 32`)".into());
        }

        let slug_length = parse_num(&var, "SLUG_LENGTH", 5)?;
        if !(1..=64).contains(&slug_length) {
            return Err("SLUG_LENGTH must be between 1 and 64".into());
        }

        let root_redirect_url = match var("ROOT_REDIRECT_URL") {
            Some(u) => Some(crate::validate::target_url(&u).map_err(|e| format!("ROOT_REDIRECT_URL: {e}"))?),
            None => None,
        };

        let cache_size = parse_num(&var, "CACHE_SIZE", 1000)?;
        if cache_size == 0 {
            return Err("CACHE_SIZE must be at least 1".into());
        }
        let last_used_flush_secs = parse_num(&var, "LAST_USED_FLUSH_SECS", 60)? as u64;
        if last_used_flush_secs == 0 {
            return Err("LAST_USED_FLUSH_SECS must be at least 1".into());
        }

        Ok(Self {
            database_url,
            admin_password_hash,
            session_secret: session_secret.into_bytes(),
            slug_length,
            root_redirect_url,
            log_level: log_level(var("LOG_LEVEL").as_deref())?,
            cookie_secure: parse_bool(&var, "COOKIE_SECURE", true)?,
            cache_size,
            last_used_flush_secs,
        })
    }

    /// Scheme used when the UI builds short URLs.
    pub fn public_scheme(&self) -> &'static str {
        if self.cookie_secure {
            "https"
        } else {
            "http"
        }
    }
}

/// Validates `LOG_LEVEL`. `debug` logs every request, `info` only 404s.
pub fn log_level(raw: Option<&str>) -> Result<String, String> {
    let level = raw
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("info")
        .to_ascii_lowercase();
    match level.as_str() {
        "error" | "warn" | "info" | "debug" | "trace" => Ok(level),
        other => Err(format!("LOG_LEVEL must be error, warn, info or debug, got `{other}`")),
    }
}

fn parse_num(var: &impl Fn(&str) -> Option<String>, key: &str, default: usize) -> Result<usize, String> {
    match var(key) {
        Some(v) => v.parse().map_err(|_| format!("{key} must be a positive number")),
        None => Ok(default),
    }
}

fn parse_bool(var: &impl Fn(&str) -> Option<String>, key: &str, default: bool) -> Result<bool, String> {
    match var(key).map(|v| v.to_ascii_lowercase()) {
        None => Ok(default),
        Some(v) if ["1", "true", "yes", "on"].contains(&v.as_str()) => Ok(true),
        Some(v) if ["0", "false", "no", "off"].contains(&v.as_str()) => Ok(false),
        Some(_) => Err(format!("{key} must be true or false")),
    }
}
