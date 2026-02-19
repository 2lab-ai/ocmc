use axum::{
    extract::{Form, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
};
use serde::Deserialize;

use crate::{mc, AppState};

use super::auth;

pub async fn login_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html>
<head><meta charset="utf-8"/><title>Login</title></head>
<body style="font-family: ui-sans-serif, system-ui; padding: 24px;">
<h1>Mission Control Login</h1>
<form method="post" action="/login">
  <div><label>Username <input name="username"/></label></div>
  <div style="margin-top:8px;"><label>Password <input name="password" type="password"/></label></div>
  <button style="margin-top:12px;">Login</button>
</form>
</body>
</html>"#,
    )
}

#[derive(Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

pub async fn login_post(
    State(state): State<AppState>,
    Form(f): Form<LoginForm>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let ok = mc::db::verify_user(&state.pool, &f.username, &f.password)
        .await
        .map_err(internal)?;
    if !ok {
        return Err((StatusCode::UNAUTHORIZED, "bad credentials".into()));
    }

    mc::db::audit(
        &state.pool,
        &f.username,
        "auth.login",
        "{}",
    )
    .await
    .map_err(internal)?;

    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::SET_COOKIE,
        auth::set_session_cookie(&f.username).parse().unwrap(),
    );
    headers.insert(axum::http::header::LOCATION, "/".parse().unwrap());

    Ok((StatusCode::SEE_OTHER, headers))
}

pub async fn logout_post(
    State(state): State<AppState>,
    auth::AuthedUser(user): auth::AuthedUser,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    mc::db::audit(&state.pool, &user, "auth.logout", "{}")
        .await
        .map_err(internal)?;

    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::SET_COOKIE,
        auth::clear_session_cookie().parse().unwrap(),
    );
    headers.insert(axum::http::header::LOCATION, "/login".parse().unwrap());

    Ok((StatusCode::SEE_OTHER, headers))
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
