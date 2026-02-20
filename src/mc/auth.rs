use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::AppState;

type HmacSha256 = Hmac<Sha256>;

/// Default token lifetime: 24 hours.
const DEFAULT_TOKEN_TTL_SECS: i64 = 86400;

/// Cookie name for the session token.
pub const COOKIE_NAME: &str = "mc_session";

// ---------------------------------------------------------------------------
// Token format: base64url( JSON{"u":"<user>","iat":<epoch>,"exp":<epoch>} ) . hex(hmac)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
pub struct TokenPayload {
    /// username
    pub u: String,
    /// issued-at (unix timestamp)
    pub iat: i64,
    /// expiry (unix timestamp)
    pub exp: i64,
}

/// Create a signed token string for the given user.
pub fn create_token(username: &str, secret: &str) -> String {
    create_token_with_ttl(username, secret, DEFAULT_TOKEN_TTL_SECS)
}

/// Create a signed token with custom TTL (seconds).
pub fn create_token_with_ttl(username: &str, secret: &str, ttl_secs: i64) -> String {
    let now = Utc::now();
    let payload = TokenPayload {
        u: username.to_string(),
        iat: now.timestamp(),
        exp: (now + Duration::seconds(ttl_secs)).timestamp(),
    };
    sign_payload(&payload, secret)
}

/// Encode & sign a payload.
fn sign_payload(payload: &TokenPayload, secret: &str) -> String {
    use base64::Engine;
    let json = serde_json::to_string(payload).expect("serialize token payload");
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.as_bytes());
    let sig = compute_hmac(encoded.as_bytes(), secret);
    format!("{}.{}", encoded, sig)
}

/// Verify a token string. Returns the payload if valid and not expired.
pub fn verify_token(token: &str, secret: &str) -> Result<TokenPayload, TokenError> {
    verify_token_at(token, secret, Utc::now())
}

/// Verify a token string at a specific time (for testing).
pub fn verify_token_at(
    token: &str,
    secret: &str,
    now: DateTime<Utc>,
) -> Result<TokenPayload, TokenError> {
    let (encoded, sig) = token
        .rsplit_once('.')
        .ok_or(TokenError::MalformedToken)?;

    // Verify HMAC
    let expected_sig = compute_hmac(encoded.as_bytes(), secret);
    if !constant_time_eq(sig.as_bytes(), expected_sig.as_bytes()) {
        return Err(TokenError::InvalidSignature);
    }

    // Decode payload
    use base64::Engine;
    let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| TokenError::MalformedToken)?;
    let payload: TokenPayload =
        serde_json::from_slice(&json_bytes).map_err(|_| TokenError::MalformedToken)?;

    // Check expiry
    if now.timestamp() > payload.exp {
        return Err(TokenError::Expired);
    }

    // Sanity: username must not be empty
    if payload.u.is_empty() {
        return Err(TokenError::MalformedToken);
    }

    Ok(payload)
}

#[derive(Debug, PartialEq)]
pub enum TokenError {
    MalformedToken,
    InvalidSignature,
    Expired,
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::MalformedToken => write!(f, "malformed token"),
            TokenError::InvalidSignature => write!(f, "invalid signature"),
            TokenError::Expired => write!(f, "token expired"),
        }
    }
}

fn compute_hmac(data: &[u8], secret: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

/// Constant-time comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

// ---------------------------------------------------------------------------
// Axum extractor
// ---------------------------------------------------------------------------

pub struct AuthedUser(pub String);

#[async_trait]
impl FromRequestParts<AppState> for AuthedUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let cookies = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok());
        let Some(cookies) = cookies else {
            return Err((StatusCode::UNAUTHORIZED, "missing cookie"));
        };
        let token_value = cookies
            .split(';')
            .map(|s| s.trim())
            .find_map(|kv| kv.strip_prefix(&format!("{}=", COOKIE_NAME)).map(|v| v.to_string()));
        let Some(token) = token_value else {
            return Err((StatusCode::UNAUTHORIZED, "missing session cookie"));
        };
        if token.is_empty() {
            return Err((StatusCode::UNAUTHORIZED, "empty session cookie"));
        }

        match verify_token(&token, &state.cfg.session_secret) {
            Ok(payload) => Ok(AuthedUser(payload.u)),
            Err(_) => Err((StatusCode::UNAUTHORIZED, "invalid or expired session")),
        }
    }
}

// ---------------------------------------------------------------------------
// Cookie helpers
// ---------------------------------------------------------------------------

pub fn set_session_cookie(username: &str, secret: &str) -> String {
    let token = create_token(username, secret);
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        COOKIE_NAME, token, DEFAULT_TOKEN_TTL_SECS
    )
}

pub fn clear_session_cookie() -> String {
    format!(
        "{}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax",
        COOKIE_NAME
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    const TEST_SECRET: &str = "test-secret-key-for-hmac-signing";

    #[test]
    fn test_create_and_verify_token() {
        let token = create_token("alice", TEST_SECRET);
        let payload = verify_token(&token, TEST_SECRET).expect("valid token");
        assert_eq!(payload.u, "alice");
        assert!(payload.exp > payload.iat);
        assert_eq!(payload.exp - payload.iat, DEFAULT_TOKEN_TTL_SECS);
    }

    #[test]
    fn test_wrong_secret_rejected() {
        let token = create_token("alice", TEST_SECRET);
        let result = verify_token(&token, "wrong-secret");
        assert_eq!(result.unwrap_err(), TokenError::InvalidSignature);
    }

    #[test]
    fn test_tampered_payload_rejected() {
        let token = create_token("alice", TEST_SECRET);
        // Swap out the payload portion with different data
        let (_, sig) = token.rsplit_once('.').unwrap();
        use base64::Engine;
        let fake_payload = TokenPayload {
            u: "evil".to_string(),
            iat: Utc::now().timestamp(),
            exp: (Utc::now() + Duration::hours(1)).timestamp(),
        };
        let fake_encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_string(&fake_payload).unwrap().as_bytes());
        let tampered = format!("{}.{}", fake_encoded, sig);
        let result = verify_token(&tampered, TEST_SECRET);
        assert_eq!(result.unwrap_err(), TokenError::InvalidSignature);
    }

    #[test]
    fn test_expired_token_rejected() {
        let token = create_token_with_ttl("alice", TEST_SECRET, 1);
        // Verify at a time 10 seconds in the future
        let future = Utc::now() + Duration::seconds(10);
        let result = verify_token_at(&token, TEST_SECRET, future);
        assert_eq!(result.unwrap_err(), TokenError::Expired);
    }

    #[test]
    fn test_malformed_token_rejected() {
        assert_eq!(
            verify_token("not-a-valid-token", TEST_SECRET).unwrap_err(),
            TokenError::MalformedToken
        );
        assert_eq!(
            verify_token("", TEST_SECRET).unwrap_err(),
            TokenError::MalformedToken
        );
        // Valid base64 but invalid JSON
        assert_eq!(
            verify_token("bm90LWpzb24.abcdef", TEST_SECRET).unwrap_err(),
            TokenError::InvalidSignature
        );
    }

    #[test]
    fn test_empty_username_rejected() {
        let payload = TokenPayload {
            u: "".to_string(),
            iat: Utc::now().timestamp(),
            exp: (Utc::now() + Duration::hours(1)).timestamp(),
        };
        let token = sign_payload(&payload, TEST_SECRET);
        let result = verify_token(&token, TEST_SECRET);
        assert_eq!(result.unwrap_err(), TokenError::MalformedToken);
    }

    #[test]
    fn test_cookie_format() {
        let cookie = set_session_cookie("admin", TEST_SECRET);
        assert!(cookie.starts_with("mc_session="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Max-Age="));
        // The token part should contain a dot separator
        let token_part = cookie
            .split('=')
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        assert!(token_part.contains('.'));
        // And it should verify
        let payload = verify_token(token_part, TEST_SECRET).expect("cookie token valid");
        assert_eq!(payload.u, "admin");
    }

    #[test]
    fn test_clear_cookie() {
        let cookie = clear_session_cookie();
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.contains(COOKIE_NAME));
    }
}
