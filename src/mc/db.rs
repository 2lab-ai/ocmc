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
    // mc-6yy.5: per-agent API key hash for agent-to-MC auth
    if !cols.contains(&"api_key_hash".to_string()) {
        pool.execute("ALTER TABLE agents ADD COLUMN api_key_hash TEXT NULL")
            .await?;
    }
    if !cols.contains(&"last_heartbeat_at".to_string()) {
        pool.execute("ALTER TABLE agents ADD COLUMN last_heartbeat_at TEXT NULL")
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

/// Full agent record for hierarchy API.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentRow {
    pub id: String,
    pub display_name: String,
    pub role: String,
    pub parent_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// List all agents with full hierarchy data.
pub async fn list_agents_full(pool: &SqlitePool) -> anyhow::Result<Vec<AgentRow>> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>, String, String)>(
        "SELECT id, display_name, role, parent_id, created_at, updated_at FROM agents ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, display_name, role, parent_id, created_at, updated_at)| AgentRow {
            id,
            display_name,
            role,
            parent_id,
            created_at,
            updated_at,
        })
        .collect())
}

/// Get a single agent by ID.
pub async fn get_agent(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<AgentRow>> {
    let row = sqlx::query_as::<_, (String, String, String, Option<String>, String, String)>(
        "SELECT id, display_name, role, parent_id, created_at, updated_at FROM agents WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id, display_name, role, parent_id, created_at, updated_at)| AgentRow {
        id,
        display_name,
        role,
        parent_id,
        created_at,
        updated_at,
    }))
}

/// Allowed agent roles.
pub const VALID_ROLES: &[&str] = &["root", "pdpm", "dev", "qa", "observer"];

/// Create a new agent.
pub async fn create_agent(
    pool: &SqlitePool,
    id: &str,
    display_name: &str,
    role: &str,
    parent_id: Option<&str>,
) -> anyhow::Result<AgentRow> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO agents (id, display_name, role, parent_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(display_name)
    .bind(role)
    .bind(parent_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(AgentRow {
        id: id.to_string(),
        display_name: display_name.to_string(),
        role: role.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Delete an agent by ID. Returns true if a row was deleted.
pub async fn delete_agent(pool: &SqlitePool, id: &str) -> anyhow::Result<bool> {
    let result = sqlx::query("DELETE FROM agents WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Update an agent's parent_id (reparent).
pub async fn reparent_agent(
    pool: &SqlitePool,
    id: &str,
    new_parent_id: Option<&str>,
) -> anyhow::Result<bool> {
    let now = Utc::now().to_rfc3339();
    let result =
        sqlx::query("UPDATE agents SET parent_id = ?, updated_at = ? WHERE id = ?")
            .bind(new_parent_id)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// Check if setting `agent_id.parent_id = new_parent_id` would create a cycle.
/// Walks from new_parent_id up through the parent chain; if we encounter agent_id, it's a cycle.
pub async fn would_create_cycle(
    pool: &SqlitePool,
    agent_id: &str,
    new_parent_id: &str,
) -> anyhow::Result<bool> {
    // Self-parenting is always a cycle.
    if agent_id == new_parent_id {
        return Ok(true);
    }

    let mut current = new_parent_id.to_string();
    // Walk up the tree from new_parent. If we reach agent_id, it's a cycle.
    // Safety: limit iterations to prevent infinite loop on corrupt data.
    for _ in 0..100 {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT parent_id FROM agents WHERE id = ?")
                .bind(&current)
                .fetch_optional(pool)
                .await?;
        match row {
            Some((Some(pid),)) => {
                if pid == agent_id {
                    return Ok(true);
                }
                current = pid;
            }
            _ => break, // reached root or unknown agent
        }
    }
    Ok(false)
}

/// Check if the agent has any children (used before delete).
pub async fn has_children(pool: &SqlitePool, agent_id: &str) -> anyhow::Result<bool> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT COUNT(*) FROM agents WHERE parent_id = ?")
            .bind(agent_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map_or(false, |(c,)| c > 0))
}

// ---------------------------------------------------------------------------
// Audit query API
// ---------------------------------------------------------------------------

/// A single audit event row returned by the query API.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditEventRow {
    pub id: i64,
    pub at: String,
    pub actor_kind: String,
    pub actor_id: String,
    pub decision: String,
    pub action: String,
    pub payload: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_reason: Option<String>,
}

/// Query parameters for audit event listing.
#[derive(Debug, Default)]
pub struct AuditQuery {
    pub since: Option<String>,
    pub until: Option<String>,
    pub actor_kind: Option<String>,
    pub actor_id: Option<String>,
    pub username: Option<String>, // backward compat: maps to actor_kind=user + actor_id
    pub action_prefix: Option<String>,
    pub decision: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

/// Query audit events with filters. Returns (events, total_count).
///
/// Uses `sqlx::query` with `SqliteArguments` for fully dynamic binding.
pub async fn query_audit_events(
    pool: &SqlitePool,
    q: &AuditQuery,
) -> anyhow::Result<(Vec<AuditEventRow>, i64)> {
    use sqlx::Arguments;

    let mut conditions: Vec<String> = Vec::new();
    let mut args = sqlx::sqlite::SqliteArguments::default();

    if let Some(ref v) = q.since {
        conditions.push("at >= ?".into());
        args.add(v.as_str()).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    if let Some(ref v) = q.until {
        conditions.push("at <= ?".into());
        args.add(v.as_str()).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    if let Some(ref v) = q.username {
        conditions.push("actor_kind = 'user'".into());
        conditions.push("actor_id = ?".into());
        args.add(v.as_str()).map_err(|e| anyhow::anyhow!("{e}"))?;
    } else {
        if let Some(ref v) = q.actor_kind {
            conditions.push("actor_kind = ?".into());
            args.add(v.as_str()).map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        if let Some(ref v) = q.actor_id {
            conditions.push("actor_id = ?".into());
            args.add(v.as_str()).map_err(|e| anyhow::anyhow!("{e}"))?;
        }
    }
    if let Some(ref v) = q.action_prefix {
        conditions.push("action LIKE ?".into());
        let like = format!("{v}%");
        args.add(like).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    if let Some(ref v) = q.decision {
        conditions.push("decision = ?".into());
        args.add(v.as_str()).map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    // --- count ---
    let count_sql = format!("SELECT COUNT(*) FROM audit_events {where_clause}");
    let total: (i64,) = sqlx::query_as_with(&count_sql, args.clone())
        .fetch_one(pool)
        .await?;

    // --- data ---
    let data_sql = format!(
        "SELECT id, at, actor_kind, actor_id, decision, action, payload_json, override_flag, override_reason \
         FROM audit_events {where_clause} ORDER BY id DESC LIMIT ? OFFSET ?"
    );
    args.add(q.limit).map_err(|e| anyhow::anyhow!("{e}"))?;
    args.add(q.offset).map_err(|e| anyhow::anyhow!("{e}"))?;

    let rows: Vec<(i64, String, String, String, String, String, String, i32, Option<String>)> =
        sqlx::query_as_with(&data_sql, args)
            .fetch_all(pool)
            .await?;

    let events = rows
        .into_iter()
        .map(|(id, at, actor_kind, actor_id, decision, action, payload_json, override_flag, override_reason)| {
            let payload = serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::String(payload_json));
            AuditEventRow {
                id,
                at,
                actor_kind,
                actor_id,
                decision,
                action,
                payload,
                override_reason: if override_flag != 0 { override_reason } else { None },
            }
        })
        .collect();

    Ok((events, total.0))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        // Create tables manually (simplified migration for tests).
        pool.execute(
            r#"
CREATE TABLE IF NOT EXISTS agents (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'dev',
  parent_id TEXT NULL REFERENCES agents(id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_agents_parent_id ON agents(parent_id);
"#,
        )
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_create_and_list_agents() {
        let pool = test_pool().await;

        // Create root agent
        let root = create_agent(&pool, "main", "main", "root", None)
            .await
            .unwrap();
        assert_eq!(root.id, "main");
        assert_eq!(root.role, "root");
        assert!(root.parent_id.is_none());

        // Create child
        let pdpm = create_agent(&pool, "pdpm-mc", "pdpm-mc", "pdpm", Some("main"))
            .await
            .unwrap();
        assert_eq!(pdpm.parent_id.as_deref(), Some("main"));

        // List
        let agents = list_agents_full(&pool).await.unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].id, "main");
        assert_eq!(agents[1].id, "pdpm-mc");
    }

    #[tokio::test]
    async fn test_get_agent() {
        let pool = test_pool().await;
        create_agent(&pool, "main", "main", "root", None)
            .await
            .unwrap();

        let found = get_agent(&pool, "main").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().role, "root");

        let missing = get_agent(&pool, "nonexistent").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_create_duplicate_agent() {
        let pool = test_pool().await;
        create_agent(&pool, "main", "main", "root", None)
            .await
            .unwrap();
        let result = create_agent(&pool, "main", "main2", "root", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("UNIQUE"));
    }

    #[tokio::test]
    async fn test_delete_agent() {
        let pool = test_pool().await;
        create_agent(&pool, "dev-a", "dev-a", "dev", None)
            .await
            .unwrap();

        let deleted = delete_agent(&pool, "dev-a").await.unwrap();
        assert!(deleted);

        let deleted_again = delete_agent(&pool, "dev-a").await.unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn test_reparent_agent() {
        let pool = test_pool().await;
        create_agent(&pool, "main", "main", "root", None)
            .await
            .unwrap();
        create_agent(&pool, "pdpm-mc", "pdpm-mc", "pdpm", Some("main"))
            .await
            .unwrap();
        create_agent(&pool, "dev-a", "dev-a", "dev", Some("main"))
            .await
            .unwrap();

        // Reparent dev-a under pdpm-mc
        let ok = reparent_agent(&pool, "dev-a", Some("pdpm-mc"))
            .await
            .unwrap();
        assert!(ok);

        let agent = get_agent(&pool, "dev-a").await.unwrap().unwrap();
        assert_eq!(agent.parent_id.as_deref(), Some("pdpm-mc"));
    }

    #[tokio::test]
    async fn test_cycle_detection_self() {
        let pool = test_pool().await;
        create_agent(&pool, "main", "main", "root", None)
            .await
            .unwrap();

        let is_cycle = would_create_cycle(&pool, "main", "main")
            .await
            .unwrap();
        assert!(is_cycle);
    }

    #[tokio::test]
    async fn test_cycle_detection_indirect() {
        let pool = test_pool().await;
        create_agent(&pool, "a", "a", "root", None).await.unwrap();
        create_agent(&pool, "b", "b", "pdpm", Some("a"))
            .await
            .unwrap();
        create_agent(&pool, "c", "c", "dev", Some("b"))
            .await
            .unwrap();

        // Reparenting a under c would create a→b→c→a cycle
        let is_cycle = would_create_cycle(&pool, "a", "c").await.unwrap();
        assert!(is_cycle);

        // Reparenting c under a is not a cycle (c→a, already a→b→c→a? no, a→b→c currently)
        // Actually: if c.parent_id = a, then tree is a→b (b.parent=a), a→c (c.parent=a). No cycle.
        let no_cycle = would_create_cycle(&pool, "c", "a").await.unwrap();
        assert!(!no_cycle);
    }

    #[tokio::test]
    async fn test_has_children() {
        let pool = test_pool().await;
        create_agent(&pool, "main", "main", "root", None)
            .await
            .unwrap();
        create_agent(&pool, "dev-a", "dev-a", "dev", Some("main"))
            .await
            .unwrap();

        assert!(has_children(&pool, "main").await.unwrap());
        assert!(!has_children(&pool, "dev-a").await.unwrap());
    }

    #[tokio::test]
    async fn test_valid_roles() {
        assert!(VALID_ROLES.contains(&"root"));
        assert!(VALID_ROLES.contains(&"pdpm"));
        assert!(VALID_ROLES.contains(&"dev"));
        assert!(VALID_ROLES.contains(&"qa"));
        assert!(VALID_ROLES.contains(&"observer"));
        assert!(!VALID_ROLES.contains(&"admin"));
    }

    // --- Audit query tests ---

    async fn audit_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        pool.execute(
            r#"
CREATE TABLE audit_events (
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
CREATE INDEX idx_audit_at ON audit_events(at);
CREATE INDEX idx_audit_actor ON audit_events(actor_kind, actor_id);
CREATE INDEX idx_audit_action ON audit_events(action);
"#,
        )
        .await
        .unwrap();
        pool
    }

    async fn insert_audit(pool: &SqlitePool, at: &str, actor_kind: &str, actor_id: &str, decision: &str, action: &str, payload: &str, override_flag: i32, override_reason: Option<&str>) {
        sqlx::query(
            "INSERT INTO audit_events (at, actor_kind, actor_id, decision, action, payload_json, override_flag, override_reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(at).bind(actor_kind).bind(actor_id).bind(decision).bind(action).bind(payload).bind(override_flag).bind(override_reason)
        .execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn test_audit_query_no_filters() {
        let pool = audit_pool().await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "allow", "auth.login", "{}", 0, None).await;
        insert_audit(&pool, "2026-01-02T00:00:00Z", "agent", "opus46", "deny", "task.move", "{}", 0, None).await;

        let q = AuditQuery { limit: 50, ..Default::default() };
        let (events, total) = query_audit_events(&pool, &q).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(events.len(), 2);
        // Ordered by id DESC
        assert_eq!(events[0].action, "task.move");
        assert_eq!(events[1].action, "auth.login");
    }

    #[tokio::test]
    async fn test_audit_query_since_until() {
        let pool = audit_pool().await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "allow", "a", "{}", 0, None).await;
        insert_audit(&pool, "2026-01-15T00:00:00Z", "user", "admin", "allow", "b", "{}", 0, None).await;
        insert_audit(&pool, "2026-02-01T00:00:00Z", "user", "admin", "allow", "c", "{}", 0, None).await;

        let q = AuditQuery {
            since: Some("2026-01-10T00:00:00Z".into()),
            until: Some("2026-01-20T00:00:00Z".into()),
            limit: 50,
            ..Default::default()
        };
        let (events, total) = query_audit_events(&pool, &q).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0].action, "b");
    }

    #[tokio::test]
    async fn test_audit_query_username_compat() {
        let pool = audit_pool().await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "allow", "x", "{}", 0, None).await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "agent", "opus46", "allow", "y", "{}", 0, None).await;

        let q = AuditQuery { username: Some("admin".into()), limit: 50, ..Default::default() };
        let (events, total) = query_audit_events(&pool, &q).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0].actor_id, "admin");
    }

    #[tokio::test]
    async fn test_audit_query_actor_kind_and_id() {
        let pool = audit_pool().await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "allow", "x", "{}", 0, None).await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "agent", "opus46", "allow", "y", "{}", 0, None).await;

        let q = AuditQuery { actor_kind: Some("agent".into()), actor_id: Some("opus46".into()), limit: 50, ..Default::default() };
        let (events, total) = query_audit_events(&pool, &q).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0].action, "y");
    }

    #[tokio::test]
    async fn test_audit_query_action_prefix() {
        let pool = audit_pool().await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "allow", "task.move", "{}", 0, None).await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "allow", "task.assign", "{}", 0, None).await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "allow", "auth.login", "{}", 0, None).await;

        let q = AuditQuery { action_prefix: Some("task".into()), limit: 50, ..Default::default() };
        let (events, total) = query_audit_events(&pool, &q).await.unwrap();
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn test_audit_query_decision_filter() {
        let pool = audit_pool().await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "allow", "a", "{}", 0, None).await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "deny", "b", "{}", 0, None).await;

        let q = AuditQuery { decision: Some("deny".into()), limit: 50, ..Default::default() };
        let (events, total) = query_audit_events(&pool, &q).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0].action, "b");
    }

    #[tokio::test]
    async fn test_audit_query_override_metadata() {
        let pool = audit_pool().await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "allow", "task.move", "{}", 1, Some("emergency fix")).await;
        insert_audit(&pool, "2026-01-01T00:00:00Z", "user", "admin", "allow", "auth.login", "{}", 0, None).await;

        let q = AuditQuery { limit: 50, ..Default::default() };
        let (events, _) = query_audit_events(&pool, &q).await.unwrap();
        // First event (by id DESC) is auth.login - no override
        assert!(events[0].override_reason.is_none());
        // Second is task.move with override
        assert_eq!(events[1].override_reason.as_deref(), Some("emergency fix"));
    }

    #[tokio::test]
    async fn test_audit_query_pagination() {
        let pool = audit_pool().await;
        for i in 0..10 {
            insert_audit(&pool, &format!("2026-01-{:02}T00:00:00Z", i + 1), "user", "admin", "allow", &format!("act{i}"), "{}", 0, None).await;
        }

        let q = AuditQuery { limit: 3, offset: 0, ..Default::default() };
        let (events, total) = query_audit_events(&pool, &q).await.unwrap();
        assert_eq!(total, 10);
        assert_eq!(events.len(), 3);

        let q2 = AuditQuery { limit: 3, offset: 3, ..Default::default() };
        let (events2, _) = query_audit_events(&pool, &q2).await.unwrap();
        assert_eq!(events2.len(), 3);
        // No overlap
        assert_ne!(events[0].id, events2[0].id);
    }

    // --- Override session tests ---

    async fn override_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        pool.execute(
            r#"
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
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_override_status_no_session() {
        let pool = override_pool().await;
        let status = get_override_status(&pool, "admin").await.unwrap();
        assert!(!status.active);
        assert!(status.session_id.is_none());
        assert!(status.reason.is_none());
        assert!(status.expires_at.is_none());
    }

    #[tokio::test]
    async fn test_override_status_active_session() {
        let pool = override_pool().await;
        let session_id = create_override_session(&pool, "admin", "hotfix deploy", 600)
            .await
            .unwrap();

        let status = get_override_status(&pool, "admin").await.unwrap();
        assert!(status.active);
        assert_eq!(status.session_id.unwrap(), session_id);
        assert_eq!(status.reason.unwrap(), "hotfix deploy");
        assert!(status.expires_at.is_some());
        assert!(status.enabled_at.is_some());
    }

    #[tokio::test]
    async fn test_override_status_after_revoke() {
        let pool = override_pool().await;
        create_override_session(&pool, "admin", "test", 600)
            .await
            .unwrap();

        // Status is active
        let status = get_override_status(&pool, "admin").await.unwrap();
        assert!(status.active);

        // Revoke
        let revoked = revoke_override_session(&pool, "admin").await.unwrap();
        assert_eq!(revoked, 1);

        // Status is now inactive
        let status = get_override_status(&pool, "admin").await.unwrap();
        assert!(!status.active);
    }

    #[tokio::test]
    async fn test_override_status_expired_session() {
        let pool = override_pool().await;
        // Create a session with 0 TTL (immediately expired)
        // We need to insert manually with a past expires_at
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let past = now - chrono::Duration::seconds(100);

        sqlx::query(
            "INSERT INTO override_sessions (id, username, reason, enabled_at, expires_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind("admin")
        .bind("test")
        .bind(now.to_rfc3339())
        .bind(past.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let status = get_override_status(&pool, "admin").await.unwrap();
        assert!(!status.active, "Expired session should not be active");
    }

    #[tokio::test]
    async fn test_override_create_and_revoke_roundtrip() {
        let pool = override_pool().await;

        // Create
        let sid = create_override_session(&pool, "ceo", "incident-001", 300)
            .await
            .unwrap();
        assert!(!sid.is_empty());

        // Verify active
        let status = get_override_status(&pool, "ceo").await.unwrap();
        assert!(status.active);

        // Revoke
        let count = revoke_override_session(&pool, "ceo").await.unwrap();
        assert_eq!(count, 1);

        // Revoke again (should affect 0)
        let count = revoke_override_session(&pool, "ceo").await.unwrap();
        assert_eq!(count, 0);

        // Different user not affected
        create_override_session(&pool, "other", "test", 600)
            .await
            .unwrap();
        let status = get_override_status(&pool, "ceo").await.unwrap();
        assert!(!status.active);
        let status = get_override_status(&pool, "other").await.unwrap();
        assert!(status.active);
    }
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

/// Override session status for UI display.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OverrideStatus {
    pub active: bool,
    pub session_id: Option<String>,
    pub reason: Option<String>,
    pub expires_at: Option<String>,
    pub enabled_at: Option<String>,
}

/// Get the current active override session for a user (if any).
pub async fn get_override_status(pool: &SqlitePool, username: &str) -> anyhow::Result<OverrideStatus> {
    let now = Utc::now().to_rfc3339();
    let row: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, reason, enabled_at, expires_at FROM override_sessions WHERE username = ? AND expires_at > ? AND revoked_at IS NULL ORDER BY enabled_at DESC LIMIT 1",
    )
    .bind(username)
    .bind(&now)
    .fetch_optional(pool)
    .await?;

    Ok(match row {
        Some((id, reason, enabled_at, expires_at)) => OverrideStatus {
            active: true,
            session_id: Some(id),
            reason: Some(reason),
            expires_at: Some(expires_at),
            enabled_at: Some(enabled_at),
        },
        None => OverrideStatus {
            active: false,
            session_id: None,
            reason: None,
            expires_at: None,
            enabled_at: None,
        },
    })
}

// ---------------------------------------------------------------------------
// mc-6yy.5: Agent API key + heartbeat
// ---------------------------------------------------------------------------

/// Get the stored API key hash for an agent.
pub async fn get_agent_api_key_hash(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<Option<String>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT api_key_hash FROM agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(h,)| h))
}

/// Set the API key hash for an agent.
pub async fn set_agent_api_key_hash(
    pool: &SqlitePool,
    agent_id: &str,
    key_hash: &str,
) -> anyhow::Result<bool> {
    let result = sqlx::query("UPDATE agents SET api_key_hash = ? WHERE id = ?")
        .bind(key_hash)
        .bind(agent_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Record a heartbeat timestamp for an agent.
pub async fn record_heartbeat(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE agents SET last_heartbeat_at = ? WHERE id = ?")
        .bind(&now)
        .bind(agent_id)
        .execute(pool)
        .await?;
    Ok(())
}
