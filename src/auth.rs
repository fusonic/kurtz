//! Admin password, signed session cookies, CSRF tokens and login rate limiting.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

pub const SESSION_COOKIE: &str = "kurtz_session";
pub const SESSION_TTL_SECS: i64 = 7 * 24 * 3600;

type HmacSha256 = Hmac<Sha256>;

pub fn hash_password(plain: &str) -> Result<String, String> {
    Argon2::default()
        .hash_password(plain.as_bytes())
        .map(|h: PasswordHash| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn check_hash_format(hash: &str) -> Result<(), String> {
    PasswordHash::new(hash).map(|_| ()).map_err(|e| e.to_string())
}

/// Constant-time verification (Argon2 compares in constant time).
pub fn verify_password(hash: &str, candidate: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default().verify_password(candidate.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

fn sign(secret: &[u8], message: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    crate::validate::to_hex(&mac.finalize().into_bytes())
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

/// Creates a session cookie value bound to `host`: `<expiry>.<signature>`.
/// Sessions are stateless; logout clears the cookie.
pub fn new_session(secret: &[u8], host: &str, now: i64) -> String {
    let exp = now + SESSION_TTL_SECS;
    format!("{exp}.{}", sign(secret, &format!("session:{host}:{exp}")))
}

pub fn verify_session(secret: &[u8], host: &str, value: &str, now: i64) -> bool {
    let Some((exp, sig)) = value.split_once('.') else {
        return false;
    };
    let Ok(exp_num) = exp.parse::<i64>() else { return false };
    exp_num > now && ct_eq(sig, &sign(secret, &format!("session:{host}:{exp}")))
}

/// CSRF token derived from the session, so it needs no server-side storage.
pub fn csrf_token(secret: &[u8], session: &str) -> String {
    sign(secret, &format!("csrf:{session}"))
}

pub fn verify_csrf(secret: &[u8], session: &str, token: &str) -> bool {
    ct_eq(token, &csrf_token(secret, session))
}

/// Reads a cookie from a `Cookie` header value.
pub fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|part| {
        let (k, v) = part.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

pub fn session_cookie(value: &str, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={value}; Path=/admin; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECS}{}",
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear_session_cookie(secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}=; Path=/admin; HttpOnly; SameSite=Lax; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

/// Counts failed logins in a fixed window, for the whole instance: after too
/// many failures nobody can log in until the window is over.
pub struct LoginLimiter {
    max_failures: u32,
    window: Duration,
    state: Mutex<(Instant, u32)>,
}

impl LoginLimiter {
    pub fn new(max_failures: u32, window: Duration) -> Self {
        Self {
            max_failures,
            window,
            state: Mutex::new((Instant::now(), 0)),
        }
    }

    pub fn is_blocked(&self) -> bool {
        let (start, failures) = *self.state.lock().expect("limiter lock");
        start.elapsed() < self.window && failures >= self.max_failures
    }

    pub fn record_failure(&self) {
        let mut state = self.state.lock().expect("limiter lock");
        if state.0.elapsed() >= self.window {
            *state = (Instant::now(), 0);
        }
        state.1 += 1;
    }

    pub fn reset(&self) {
        *self.state.lock().expect("limiter lock") = (Instant::now(), 0);
    }
}
