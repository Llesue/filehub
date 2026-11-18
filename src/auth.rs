use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::http::{HeaderMap, HeaderValue, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use parking_lot::Mutex;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::db::{SessionUser, TokenUser, User, unix_now};
use crate::error::AppError;
use crate::state::AppState;

const MAX_LOGIN_FAILURES: u32 = 5;
const LOGIN_WINDOW: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    Session,
    Token,
}

#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub user: User,
    pub csrf_token: Option<String>,
    pub auth_kind: AuthKind,
    pub scope: Option<String>,
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| anyhow::anyhow!("password hash failed: {err}"))
}

pub async fn hash_password_async(password: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|err| anyhow::anyhow!("password hash task failed: {err}"))?
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(encoded) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub async fn verify_password_async(password: String, encoded: String) -> anyhow::Result<bool> {
    tokio::task::spawn_blocking(move || verify_password(&password, &encoded))
        .await
        .map_err(|err| anyhow::anyhow!("password verify task failed: {err}"))
}

pub fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

pub fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

pub fn constant_time_eq(left: &str, right: &str) -> bool {
    left.len() == right.len() && left.as_bytes().ct_eq(right.as_bytes()).into()
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let header = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in header.split(';') {
        let part = part.trim();
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key == name {
            return Some(value.to_owned());
        }
    }
    None
}

pub fn session_token(headers: &HeaderMap) -> Option<String> {
    cookie_value(headers, "filehub_session")
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::to_owned)
}

pub async fn current_user(
    state: &Arc<AppState>,
    headers: &HeaderMap,
) -> Result<CurrentUser, AppError> {
    if let Some(token) = bearer_token(headers) {
        let token_hash = sha256_hex(&token);
        let token_user = state
            .db
            .find_token(&token_hash)
            .await
            .map_err(AppError::internal)?
            .ok_or(AppError::Unauthorized)?;

        validate_token(&token_user)?;
        // API clients often poll; a write per request turns into a global
        // SQLite writer bottleneck. Record usage at most once every 5 minutes.
        if token_user
            .last_used_at
            .is_none_or(|at| crate::db::unix_now().saturating_sub(at) >= 300)
        {
            let db = state.db.clone();
            let token_id = token_user.token_id;
            tokio::spawn(async move {
                if let Err(error) = db.touch_token(token_id).await {
                    tracing::warn!(%error, "failed to update token last_used_at");
                }
            });
        }

        return Ok(CurrentUser {
            user: token_user.user,
            csrf_token: None,
            auth_kind: AuthKind::Token,
            scope: Some(token_user.scope),
        });
    }

    if let Some(token) = session_token(headers) {
        let token_hash = sha256_hex(&token);
        let session = state
            .db
            .find_session(&token_hash)
            .await
            .map_err(AppError::internal)?
            .ok_or(AppError::Unauthorized)?;

        validate_session(&session)?;

        return Ok(CurrentUser {
            user: session.user,
            csrf_token: Some(session.csrf_token),
            auth_kind: AuthKind::Session,
            scope: None,
        });
    }

    Err(AppError::Unauthorized)
}

pub fn require_admin(user: &CurrentUser) -> Result<(), AppError> {
    if user.user.role == "admin" {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

pub fn require_csrf(user: &CurrentUser, headers: &HeaderMap) -> Result<(), AppError> {
    if user.auth_kind == AuthKind::Token {
        return if user.scope.as_deref() == Some("write") {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        };
    }

    let expected = user.csrf_token.as_deref().ok_or(AppError::Forbidden)?;
    let provided = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::Forbidden)?;

    if constant_time_eq(expected, provided) {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

pub async fn create_session(
    state: &Arc<AppState>,
    user: &User,
    headers: &HeaderMap,
) -> Result<(String, String, i64), AppError> {
    let token = random_token(32);
    let csrf_token = random_token(24);
    let token_hash = sha256_hex(&token);
    let ttl = session_ttl_seconds(state);
    let expires_at = unix_now() + ttl;
    let ip = client_ip(headers);
    let user_agent = user_agent(headers);

    state
        .db
        .insert_session(
            &token_hash,
            &csrf_token,
            user.id,
            expires_at,
            ip.as_deref(),
            user_agent.as_deref(),
        )
        .await
        .map_err(AppError::internal)?;

    Ok((token, csrf_token, expires_at))
}

pub fn session_cookie(token: &str, max_age: i64, secure: bool) -> Result<HeaderValue, AppError> {
    let secure_flag = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "filehub_session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure_flag}"
    ))
    .map_err(|err| AppError::internal(anyhow::anyhow!(err)))
}

pub fn clear_session_cookie(secure: bool) -> Result<HeaderValue, AppError> {
    session_cookie("", 0, secure)
}

pub fn session_ttl_seconds(state: &Arc<AppState>) -> i64 {
    let hours = state.settings().settings.security.session_ttl_hours;
    (hours as i64).saturating_mul(3600)
}

/// Header injected by the client-IP middleware. Handlers must only trust this
/// value, never `X-Forwarded-For` from the wire.
pub const CLIENT_IP_HEADER: &str = "x-filehub-client-ip";

pub fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get(CLIENT_IP_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Resolve the audit client IP from the TCP peer and (only when the peer is a
/// trusted proxy) the forwarding headers.
pub fn resolve_client_ip(
    peer: std::net::SocketAddr,
    headers: &HeaderMap,
    trusted_proxies: &[String],
) -> Option<String> {
    let peer_ip = normalize_ip(peer.ip());
    if trusted_proxies
        .iter()
        .any(|rule| ip_matches_rule(peer_ip, rule))
        && let Some(forwarded) = forwarded_ip(headers)
    {
        return Some(forwarded);
    }
    Some(peer_ip.to_string())
}

pub fn valid_ip_rule(rule: &str) -> bool {
    parse_ip_rule(rule).is_some()
}

fn forwarded_ip(headers: &HeaderMap) -> Option<String> {
    for name in ["x-forwarded-for", "x-real-ip"] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            let first = value.split(',').next().unwrap_or(value).trim();
            if let Ok(ip) = first.parse::<std::net::IpAddr>() {
                return Some(normalize_ip(ip).to_string());
            }
        }
    }
    None
}

fn normalize_ip(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V6(value) => value
            .to_ipv4_mapped()
            .map(std::net::IpAddr::V4)
            .unwrap_or(std::net::IpAddr::V6(value)),
        value => value,
    }
}

fn parse_ip_rule(rule: &str) -> Option<(std::net::IpAddr, u8)> {
    let rule = rule.trim();
    if rule.is_empty() {
        return None;
    }
    let (address, prefix) = match rule.split_once('/') {
        Some((address, prefix)) => (address.trim(), Some(prefix.trim())),
        None => (rule, None),
    };
    let address = address.parse::<std::net::IpAddr>().ok()?;
    let max = match address {
        std::net::IpAddr::V4(_) => 32,
        std::net::IpAddr::V6(_) => 128,
    };
    let prefix = match prefix {
        Some(value) => value.parse::<u8>().ok().filter(|value| *value <= max)?,
        None => max,
    };
    Some((normalize_ip(address), prefix))
}

fn ip_matches_rule(peer: std::net::IpAddr, rule: &str) -> bool {
    let Some((network, prefix)) = parse_ip_rule(rule) else {
        return false;
    };
    match (network, peer) {
        (std::net::IpAddr::V4(network), std::net::IpAddr::V4(peer)) => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            u32::from(network) & mask == u32::from(peer) & mask
        }
        (std::net::IpAddr::V6(network), std::net::IpAddr::V6(peer)) => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            u128::from(network) & mask == u128::from(peer) & mask
        }
        _ => false,
    }
}

pub fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(512).collect())
}

fn validate_session(session: &SessionUser) -> Result<(), AppError> {
    if session.user.disabled {
        return Err(AppError::Forbidden);
    }
    if session.expires_at <= unix_now() {
        return Err(AppError::Unauthorized);
    }
    Ok(())
}

fn validate_token(token: &TokenUser) -> Result<(), AppError> {
    if token.user.disabled {
        return Err(AppError::Forbidden);
    }
    if token
        .expires_at
        .is_some_and(|expires_at| expires_at <= unix_now())
    {
        return Err(AppError::Unauthorized);
    }
    Ok(())
}

fn login_key(username: &str) -> String {
    username.to_lowercase()
}

#[derive(Debug)]
struct LoginAttempt {
    failures: u32,
    first_failure: Instant,
}

pub struct RateLimiter {
    attempts: Mutex<HashMap<String, LoginAttempt>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_blocked(&self, username: &str) -> bool {
        let key = login_key(username);
        let mut attempts = self.attempts.lock();
        let Some(attempt) = attempts.get(&key) else {
            return false;
        };

        if attempt.first_failure.elapsed() > LOGIN_WINDOW {
            attempts.remove(&key);
            return false;
        }

        attempt.failures >= MAX_LOGIN_FAILURES
    }

    pub fn record_failure(&self, username: &str) {
        let key = login_key(username);
        let mut attempts = self.attempts.lock();
        let attempt = attempts.entry(key).or_insert(LoginAttempt {
            failures: 0,
            first_failure: Instant::now(),
        });

        if attempt.first_failure.elapsed() > LOGIN_WINDOW {
            attempt.failures = 0;
            attempt.first_failure = Instant::now();
        }
        attempt.failures = attempt.failures.saturating_add(1);
    }

    pub fn clear(&self, username: &str) {
        self.attempts.lock().remove(&login_key(username));
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
