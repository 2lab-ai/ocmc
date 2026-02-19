use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{mc, AppState};

use super::auth::AuthedUser;
use super::policy::{
    self, Action, Actor, ActorKind, OverrideCtx, PolicyDecision,
};

// ---------------------------------------------------------------------------
// GET /api/kanban
// ---------------------------------------------------------------------------

pub async fn kanban_get(
    _user: AuthedUser,
    State(state): State<AppState>,
) -> Result<Json<mc::KanbanSnapshot>, (StatusCode, String)> {
    let r = state.cache.read().await;
    if let Some(snapshot) = &r.snapshot {
        Ok(Json(snapshot.clone()))
    } else {
        Err((StatusCode::SERVICE_UNAVAILABLE, "no snapshot".into()))
    }
}

// ---------------------------------------------------------------------------
// POST /api/task/:id/move
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct MoveReq {
    pub lane: String,
    /// Optional: the agent that this task is assigned to (for policy check).
    #[serde(default)]
    pub target_agent: Option<String>,
    /// Override context.
    #[serde(default, rename = "override")]
    pub override_flag: bool,
    #[serde(default)]
    pub override_reason: Option<String>,
}

pub async fn task_move_post(
    user: AuthedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<MoveReq>,
) -> Result<(), (StatusCode, String)> {
    let actor = Actor {
        kind: ActorKind::User,
        id: user.0.clone(),
    };
    let override_ctx = OverrideCtx {
        override_flag: req.override_flag,
        override_reason: req.override_reason.clone(),
    };
    let policy_cfg = state.cfg.policy_config();

    // Resolve target agent: if task has assignee, that's the target.
    let target_agent_id = resolve_task_assignee(&state, &id, req.target_agent.as_deref()).await;

    let target_rec = if let Some(ref tid) = target_agent_id {
        policy::load_agent(&state.pool, tid).await.map_err(internal)?
    } else {
        None
    };

    let has_override = policy::has_active_override(&state.pool, &user.0)
        .await
        .map_err(internal)?;

    let decision = policy::authorize(
        &actor,
        &Action::TaskMove,
        target_agent_id.as_deref(),
        None, // actor is User, no agent record
        target_rec.as_ref(),
        has_override,
        &override_ctx,
        &policy_cfg,
    );

    // Log the audit event (both allow and deny).
    let payload = serde_json::to_string(&serde_json::json!({
        "task_id": id,
        "lane": req.lane,
        "target_agent": target_agent_id,
    }))
    .unwrap();

    mc::db::audit_policy(
        &state.pool,
        &ActorKind::User,
        &user.0,
        &decision,
        "task.move",
        &payload,
    )
    .await
    .map_err(internal)?;

    if !decision.is_allowed() {
        return Err((
            StatusCode::FORBIDDEN,
            serde_json::to_string(&serde_json::json!({
                "error": "policy_denied",
                "reason": decision.deny_reason(),
            }))
            .unwrap(),
        ));
    }

    mc::bd::set_lane(&state.cfg, &id, &req.lane)
        .await
        .map_err(internal)?;

    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("task.move:{id}"),
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// POST /api/task/:id/assign
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct AssignReq {
    pub assignee: Option<String>,
    #[serde(default, rename = "override")]
    pub override_flag: bool,
    #[serde(default)]
    pub override_reason: Option<String>,
}

pub async fn task_assign_post(
    user: AuthedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AssignReq>,
) -> Result<(), (StatusCode, String)> {
    let actor = Actor {
        kind: ActorKind::User,
        id: user.0.clone(),
    };
    let override_ctx = OverrideCtx {
        override_flag: req.override_flag,
        override_reason: req.override_reason.clone(),
    };
    let policy_cfg = state.cfg.policy_config();

    // Target is the agent being assigned to.
    let target_agent_id = req.assignee.as_deref();

    let target_rec = if let Some(tid) = target_agent_id {
        policy::load_agent(&state.pool, tid).await.map_err(internal)?
    } else {
        None
    };

    let has_override = policy::has_active_override(&state.pool, &user.0)
        .await
        .map_err(internal)?;

    let decision = policy::authorize(
        &actor,
        &Action::TaskAssign,
        target_agent_id,
        None,
        target_rec.as_ref(),
        has_override,
        &override_ctx,
        &policy_cfg,
    );

    let payload = serde_json::to_string(&serde_json::json!({
        "task_id": id,
        "assignee": req.assignee,
    }))
    .unwrap();

    mc::db::audit_policy(
        &state.pool,
        &ActorKind::User,
        &user.0,
        &decision,
        "task.assign",
        &payload,
    )
    .await
    .map_err(internal)?;

    if !decision.is_allowed() {
        return Err((
            StatusCode::FORBIDDEN,
            serde_json::to_string(&serde_json::json!({
                "error": "policy_denied",
                "reason": decision.deny_reason(),
            }))
            .unwrap(),
        ));
    }

    mc::bd::set_assignee(&state.cfg, &id, req.assignee.as_deref())
        .await
        .map_err(internal)?;

    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("task.assign:{id}"),
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// POST /api/cron/:id/toggle
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct CronToggleReq {
    pub enabled: bool,
    #[serde(default, rename = "override")]
    pub override_flag: bool,
    #[serde(default)]
    pub override_reason: Option<String>,
}

pub async fn cron_toggle_post(
    user: AuthedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<CronToggleReq>,
) -> Result<(), (StatusCode, String)> {
    // Cron actions are always at root-agent level → allowed for users.
    let decision = PolicyDecision::Allow;
    let payload = serde_json::to_string(&serde_json::json!({
        "cron_id": id,
        "enabled": req.enabled,
    }))
    .unwrap();

    mc::db::audit_policy(
        &state.pool,
        &ActorKind::User,
        &user.0,
        &decision,
        "cron.toggle",
        &payload,
    )
    .await
    .map_err(internal)?;

    mc::cron::toggle(&state.cfg, &id, req.enabled)
        .await
        .map_err(internal)?;

    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("cron.toggle:{id}"),
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// POST /api/cron/:id/run
// ---------------------------------------------------------------------------

pub async fn cron_run_post(
    user: AuthedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(), (StatusCode, String)> {
    let decision = PolicyDecision::Allow;

    mc::db::audit_policy(
        &state.pool,
        &ActorKind::User,
        &user.0,
        &decision,
        "cron.run",
        &serde_json::json!({"cron_id": id}).to_string(),
    )
    .await
    .map_err(internal)?;

    mc::cron::run_now(&state.cfg, &id).await.map_err(internal)?;

    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("cron.run:{id}"),
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// POST /api/override/enable
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct OverrideEnableReq {
    pub reason: String,
    /// TTL in seconds. Defaults to config value.
    pub ttl_s: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct OverrideEnableResp {
    pub session_id: String,
    pub expires_at: String,
}

pub async fn override_enable_post(
    user: AuthedUser,
    State(state): State<AppState>,
    Json(req): Json<OverrideEnableReq>,
) -> Result<Json<OverrideEnableResp>, (StatusCode, String)> {
    if req.reason.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            r#"{"error":"reason_required","reason":"Override reason must not be empty."}"#.into(),
        ));
    }

    let ttl = req.ttl_s.unwrap_or(state.cfg.override_ttl_s);
    let session_id = mc::db::create_override_session(&state.pool, &user.0, &req.reason, ttl)
        .await
        .map_err(internal)?;

    let expires_at =
        (chrono::Utc::now() + chrono::Duration::seconds(ttl as i64)).to_rfc3339();

    mc::db::audit_policy(
        &state.pool,
        &ActorKind::User,
        &user.0,
        &PolicyDecision::Allow,
        "override.enable",
        &serde_json::to_string(&serde_json::json!({
            "session_id": session_id,
            "reason": req.reason,
            "ttl_s": ttl,
        }))
        .unwrap(),
    )
    .await
    .map_err(internal)?;

    Ok(Json(OverrideEnableResp {
        session_id,
        expires_at,
    }))
}

// ---------------------------------------------------------------------------
// POST /api/override/disable
// ---------------------------------------------------------------------------

pub async fn override_disable_post(
    user: AuthedUser,
    State(state): State<AppState>,
) -> Result<(), (StatusCode, String)> {
    let revoked = mc::db::revoke_override_session(&state.pool, &user.0)
        .await
        .map_err(internal)?;

    mc::db::audit_policy(
        &state.pool,
        &ActorKind::User,
        &user.0,
        &PolicyDecision::Allow,
        "override.disable",
        &serde_json::to_string(&serde_json::json!({
            "revoked_count": revoked,
        }))
        .unwrap(),
    )
    .await
    .map_err(internal)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to resolve the target agent for a task operation.
/// If a target_agent is explicitly provided, use that.
/// Otherwise try to look up the task's current assignee from the cache.
async fn resolve_task_assignee(
    state: &AppState,
    task_id: &str,
    explicit_target: Option<&str>,
) -> Option<String> {
    if let Some(t) = explicit_target {
        return Some(t.to_string());
    }
    // Try to resolve from cached snapshot.
    let r = state.cache.read().await;
    if let Some(snap) = &r.snapshot {
        if let Some(task) = snap.tasks.iter().find(|t| t.id == task_id) {
            return task.assignee.clone();
        }
    }
    None
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
