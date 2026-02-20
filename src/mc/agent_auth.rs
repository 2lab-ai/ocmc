///! mc-6yy.5: Agent-to-MC authentication via per-agent API keys.
///!
///! Agents authenticate with `Authorization: Bearer <api-key>`.
///! The key is validated against sha256(key) stored in agents.api_key_hash.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{db, AppState};

/// Hash a raw API key to hex SHA-256.
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

/// Validate the Authorization header for an agent.
pub async fn validate_agent_bearer(
    pool: &sqlx::SqlitePool,
    agent_id: &str,
    auth_header: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    let header = auth_header.ok_or((
        StatusCode::UNAUTHORIZED,
        r#"{"error":"missing_authorization"}"#.to_string(),
    ))?;

    let token = header.strip_prefix("Bearer ").ok_or((
        StatusCode::UNAUTHORIZED,
        r#"{"error":"invalid_auth_scheme","reason":"Expected Bearer token"}"#.to_string(),
    ))?;

    let stored_hash = db::get_agent_api_key_hash(pool, agent_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::FORBIDDEN,
            r#"{"error":"agent_not_found_or_no_key"}"#.to_string(),
        ))?;

    let provided_hash = hash_api_key(token);

    // Constant-time comparison
    if !constant_time_eq(provided_hash.as_bytes(), stored_hash.as_bytes()) {
        return Err((
            StatusCode::UNAUTHORIZED,
            r#"{"error":"invalid_api_key"}"#.to_string(),
        ));
    }

    Ok(())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------------------
// Heartbeat handler
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HeartbeatReq {
    pub status: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatResp {
    pub ok: bool,
    pub agent_id: String,
    pub received_at: String,
}

pub async fn heartbeat_post(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<HeartbeatReq>,
) -> Result<Json<HeartbeatResp>, (StatusCode, String)> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    validate_agent_bearer(&state.pool, &id, auth_header).await?;

    db::record_heartbeat(&state.pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(agent_id = %id, status = ?req.status, "agent heartbeat");

    Ok(Json(HeartbeatResp {
        ok: true,
        agent_id: id,
        received_at: chrono::Utc::now().to_rfc3339(),
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        pool
    }

    async fn insert_agent_with_key(pool: &SqlitePool, agent_id: &str, raw_key: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO agents (id, display_name, role, created_at, updated_at, api_key_hash) VALUES (?, ?, 'dev', ?, ?, ?)"
        )
        .bind(agent_id)
        .bind(agent_id)
        .bind(&now)
        .bind(&now)
        .bind(hash_api_key(raw_key))
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn valid_bearer_accepted() {
        let pool = test_pool().await;
        insert_agent_with_key(&pool, "agent1", "secret-key-123").await;
        let result = validate_agent_bearer(&pool, "agent1", Some("Bearer secret-key-123")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wrong_key_rejected_401() {
        let pool = test_pool().await;
        insert_agent_with_key(&pool, "agent1", "secret-key-123").await;
        let result = validate_agent_bearer(&pool, "agent1", Some("Bearer wrong-key")).await;
        assert_eq!(result.unwrap_err().0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_header_rejected_401() {
        let pool = test_pool().await;
        insert_agent_with_key(&pool, "agent1", "secret-key-123").await;
        let result = validate_agent_bearer(&pool, "agent1", None).await;
        assert_eq!(result.unwrap_err().0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unknown_agent_rejected_403() {
        let pool = test_pool().await;
        let result = validate_agent_bearer(&pool, "nonexistent", Some("Bearer any-key")).await;
        assert_eq!(result.unwrap_err().0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn agent_without_key_rejected_403() {
        let pool = test_pool().await;
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO agents (id, display_name, role, created_at, updated_at) VALUES (?, ?, 'dev', ?, ?)"
        )
        .bind("agent-nokey")
        .bind("agent-nokey")
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        let result = validate_agent_bearer(&pool, "agent-nokey", Some("Bearer any-key")).await;
        assert_eq!(result.unwrap_err().0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn basic_auth_scheme_rejected_401() {
        let pool = test_pool().await;
        insert_agent_with_key(&pool, "agent1", "secret-key-123").await;
        let result = validate_agent_bearer(&pool, "agent1", Some("Basic dXNlcjpwYXNz")).await;
        assert_eq!(result.unwrap_err().0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn hash_is_deterministic() {
        let h1 = hash_api_key("test");
        let h2 = hash_api_key("test");
        assert_eq!(h1, h2);
        assert_ne!(h1, hash_api_key("other"));
    }

    #[tokio::test]
    async fn heartbeat_records_timestamp() {
        let pool = test_pool().await;
        insert_agent_with_key(&pool, "agent1", "key").await;
        db::record_heartbeat(&pool, "agent1").await.unwrap();
        let row: (Option<String>,) =
            sqlx::query_as("SELECT last_heartbeat_at FROM agents WHERE id = ?")
                .bind("agent1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(row.0.is_some());
    }
}
