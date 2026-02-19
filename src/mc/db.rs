use anyhow::Context;
use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::{DateTime, Utc};
use sqlx::{Executor, SqlitePool};

use super::McConfig;

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    pool.execute(
        r#"
CREATE TABLE IF NOT EXISTS users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  username TEXT NOT NULL UNIQUE,
  pass_hash TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agents (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at TEXT NOT NULL,
  username TEXT NOT NULL,
  action TEXT NOT NULL,
  payload_json TEXT NOT NULL
);
"#,
    )
    .await?;
    Ok(())
}

pub async fn ensure_admin(pool: &SqlitePool, cfg: &McConfig) -> anyhow::Result<()> {
    let existing: Option<(i64,)> = sqlx::query_as("SELECT id FROM users WHERE username = ?")
        .bind(&cfg.admin_user)
        .fetch_optional(pool)
        .await?;
    if existing.is_some() {
        return Ok(());
    }

    let salt = SaltString::generate(&mut rand::thread_rng());
    let pass_hash = Argon2::default()
        .hash_password(cfg.admin_pass.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("argon2 hash: {e}"))?
        .to_string();

    sqlx::query("INSERT INTO users (username, pass_hash, created_at) VALUES (?, ?, ?)")
        .bind(&cfg.admin_user)
        .bind(&pass_hash)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn seed_default_agents(pool: &SqlitePool) -> anyhow::Result<()> {
    for (id, name) in [("main", "main"), ("opus46", "opus46")] {
        sqlx::query("INSERT OR IGNORE INTO agents (id, display_name, created_at) VALUES (?, ?, ?)")
            .bind(id)
            .bind(name)
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn list_agents(pool: &SqlitePool) -> anyhow::Result<Vec<(String, String)>> {
    let rows = sqlx::query_as::<_, (String, String)>("SELECT id, display_name FROM agents ORDER BY id")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn audit(
    pool: &SqlitePool,
    username: &str,
    action: &str,
    payload_json: &str,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO audit_events (at, username, action, payload_json) VALUES (?, ?, ?, ?)")
        .bind(Utc::now().to_rfc3339())
        .bind(username)
        .bind(action)
        .bind(payload_json)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn verify_user(pool: &SqlitePool, username: &str, password: &str) -> anyhow::Result<bool> {
    let row: Option<(String,)> = sqlx::query_as("SELECT pass_hash FROM users WHERE username=?")
        .bind(username)
        .fetch_optional(pool)
        .await?;
    let Some((pass_hash,)) = row else { return Ok(false) };
    let parsed = PasswordHash::new(&pass_hash)
        .map_err(|e| anyhow::anyhow!("argon2 parse: {e}"))?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}
