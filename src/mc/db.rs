use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::Utc;
use sqlx::{Executor, SqlitePool};

use super::McConfig;
use super::policy::{ActorKind, PolicyDecision};

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
  role TEXT NOT NULL DEFAULT 'dev',
  parent_id TEXT NULL REFERENCES agents(id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_agents_parent_id ON agents(parent_id);

CREATE TABLE IF NOT EXISTS audit_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at TEXT NOT NULL,
  actor_kind TEXT NOT NULL DEFAULT 'user',
  actor_id TEXT NOT NULL DEFAULT '',
  decision TEXT NOT NULL DEFAULT 'allow',
  action TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  override_flag INTEGER NOT NULL DEFAULT 0,
  override_reason TEXT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_at ON audit_events(at);
CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_events(actor_kind, actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_events(action);

CREATE TABLE IF NOT EXISTS override_sessions (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL,
  reason TEXT NOT NULL,
  enabled_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  revoked_at TEXT NULL
);

CREATE INDEX IF NOT EXISTS idx_override_exp ON override_sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_override_user ON override_sessions(username);
"#,
    )
    .await?;

    // Migration: add columns if not present (safe for existing DBs).
    // SQLite doesn't support ADD COLUMN IF NOT EXISTS, so we check pragmas.
    migrate_agents_columns(pool).await?;
    migrate_audit_columns(pool).await?;

    Ok(())
}

async fn migrate_agents_columns(pool: &SqlitePool) -> anyhow::Result<()> {
    let cols = get_column_names(pool, "agents").await?;

    if !cols.contains(&"role".to_string()) {
        pool.execute("ALTER TABLE agents ADD COLUMN role TEXT NOT NULL DEFAULT 'dev'")
            .await?;
    }
    if !cols.contains(&"parent_id".to_string()) {
        pool.execute("ALTER TABLE agents ADD COLUMN parent_id TEXT NULL REFERENCES agents(id)")
            .await?;
    }
    if !cols.contains(&"updated_at".to_string()) {
        pool.execute("ALTER TABLE agents ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''")
            .await?;
    }

    Ok(())
}

async fn migrate_audit_columns(pool: &SqlitePool) -> anyhow::Result<()> {
    let cols = get_column_names(pool, "audit_events").await?;

    if !cols.contains(&"actor_kind".to_string()) {
        pool.execute("ALTER TABLE audit_events ADD COLUMN actor_kind TEXT NOT NULL DEFAULT 'user'")
            .await?;
    }
    if !cols.contains(&"actor_id".to_string()) {
        // Migrate: copy old 'username' into actor_id for existing rows
        pool.execute("ALTER TABLE audit_events ADD COLUMN actor_id TEXT NOT NULL DEFAULT ''")
            .await?;
        // Backfill actor_id from the old username column if it exists
        if cols.contains(&"username".to_string()) {
            pool.execute("UPDATE audit_events SET actor_id = username WHERE actor_id = ''")
                .await
                .ok(); // non-fatal
        }
    }
    if !cols.contains(&"decision".to_string()) {
        pool.execute("ALTER TABLE audit_events ADD COLUMN decision TEXT NOT NULL DEFAULT 'allow'")
            .await?;
    }
    if !cols.contains(&"override_flag".to_string()) {
        pool.execute("ALTER TABLE audit_events ADD COLUMN override_flag INTEGER NOT NULL DEFAULT 0")
            .await?;
    }
    if !cols.contains(&"override_reason".to_string()) {
        pool.execute("ALTER TABLE audit_events ADD COLUMN override_reason TEXT NULL")
            .await?;
    }

    Ok(())
}

async fn get_column_names(pool: &SqlitePool, table: &str) -> anyhow::Result<Vec<String>> {
    // Use PRAGMA table_info to get column names.
    let rows: Vec<(i32, String, String, i32, Option<String>, i32)> = sqlx::query_as(&format!(
        "PRAGMA table_info({})",
        table
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(_, name, ..)| name).collect())
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
    let now = Utc::now().to_rfc3339();
    // Root agent: main
    sqlx::query(
        "INSERT OR IGNORE INTO agents (id, display_name, role, parent_id, created_at, updated_at) VALUES (?, ?, ?, NULL, ?, ?)",
    )
    .bind("main")
    .bind("main")
    .bind("root")
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    // Default dev agent: opus46 (child of main for now — will be reparented under pdpm later)
    sqlx::query(
        "INSERT OR IGNORE INTO agents (id, display_name, role, parent_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("opus46")
    .bind("opus46")
    .bind("dev")
    .bind("main")
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    // Ensure existing agents have role set (migration backfill)
    sqlx::query("UPDATE agents SET role = 'root', updated_at = ? WHERE id = 'main' AND role = 'dev'")
        .bind(&now)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn list_agents(pool: &SqlitePool) -> anyhow::Result<Vec<(String, String)>> {
    let rows =
        sqlx::query_as::<_, (String, String)>("SELECT id, display_name FROM agents ORDER BY id")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// Write a policy-aware audit event.
pub async fn audit_policy(
    pool: &SqlitePool,
    actor_kind: &ActorKind,
    actor_id: &str,
    decision: &PolicyDecision,
    action: &str,
    payload_json: &str,
) -> anyhow::Result<()> {
    let (override_flag, override_reason) = match decision {
        PolicyDecision::AllowWithOverride { reason } => (1i32, Some(reason.as_str())),
        _ => (0, None),
    };

    sqlx::query(
        "INSERT INTO audit_events (at, actor_kind, actor_id, decision, action, payload_json, override_flag, override_reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(actor_kind.to_string())
    .bind(actor_id)
    .bind(decision.decision_str())
    .bind(action)
    .bind(payload_json)
    .bind(override_flag)
    .bind(override_reason)
    .execute(pool)
    .await?;
    Ok(())
}

/// Legacy audit helper (backwards-compatible wrapper).
pub async fn audit(
    pool: &SqlitePool,
    username: &str,
    action: &str,
    payload_json: &str,
) -> anyhow::Result<()> {
    audit_policy(
        pool,
        &ActorKind::User,
        username,
        &PolicyDecision::Allow,
        action,
        payload_json,
    )
    .await
}

pub async fn verify_user(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> anyhow::Result<bool> {
    let row: Option<(String,)> = sqlx::query_as("SELECT pass_hash FROM users WHERE username=?")
        .bind(username)
        .fetch_optional(pool)
        .await?;
    let Some((pass_hash,)) = row else {
        return Ok(false);
    };
    let parsed =
        PasswordHash::new(&pass_hash).map_err(|e| anyhow::anyhow!("argon2 parse: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

// --- Override session management ---

pub async fn create_override_session(
    pool: &SqlitePool,
    username: &str,
    reason: &str,
    ttl_s: u64,
) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires_at = now + chrono::Duration::seconds(ttl_s as i64);

    sqlx::query(
        "INSERT INTO override_sessions (id, username, reason, enabled_at, expires_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(username)
    .bind(reason)
    .bind(now.to_rfc3339())
    .bind(expires_at.to_rfc3339())
    .execute(pool)
    .await?;

    Ok(id)
}

pub async fn revoke_override_session(pool: &SqlitePool, username: &str) -> anyhow::Result<u64> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE override_sessions SET revoked_at = ? WHERE username = ? AND revoked_at IS NULL AND expires_at > ?",
    )
    .bind(&now)
    .bind(username)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
