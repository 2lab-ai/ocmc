use axum::{
    async_trait,
    extract::{FromRequestParts, State},
    http::{request::Parts, HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::AppState;

// Minimal cookie-based auth.
// Cookie: mc_session=<username>

pub struct AuthedUser(pub String);

#[async_trait]
impl FromRequestParts<AppState> for AuthedUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let cookies = parts.headers.get(axum::http::header::COOKIE).and_then(|v| v.to_str().ok());
        let Some(cookies) = cookies else {
            return Err((StatusCode::UNAUTHORIZED, "missing cookie"));
        };
        let username = cookies
            .split(';')
            .map(|s| s.trim())
            .find_map(|kv| kv.strip_prefix("mc_session=").map(|v| v.to_string()));
        match username {
            Some(u) if !u.is_empty() => Ok(AuthedUser(u)),
            _ => Err((StatusCode::UNAUTHORIZED, "invalid session")),
        }
    }
}

pub fn set_session_cookie(username: &str) -> String {
    format!("mc_session={}; Path=/; HttpOnly; SameSite=Lax", username)
}

pub fn clear_session_cookie() -> String {
    "mc_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax".to_string()
}
