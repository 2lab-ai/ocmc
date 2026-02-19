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
// GET /api/override/status
// ---------------------------------------------------------------------------

pub async fn override_status_get(
    user: AuthedUser,
    State(state): State<AppState>,
) -> Result<Json<mc::db::OverrideStatus>, (StatusCode, String)> {
    let status = mc::db::get_override_status(&state.pool, &user.0)
        .await
        .map_err(internal)?;
    Ok(Json(status))
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
// POST /api/policy/check — pre-flight gating
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PolicyCheckReq {
    pub action: String,
    pub target_agent: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PolicyCheckResp {
    pub allowed: bool,
    pub reason: Option<String>,
    pub needs_override: bool,
}

pub async fn policy_check_post(
    user: AuthedUser,
    State(state): State<AppState>,
    Json(req): Json<PolicyCheckReq>,
) -> Result<Json<PolicyCheckResp>, (StatusCode, String)> {
    let policy_cfg = state.cfg.policy_config();
    let actor = Actor {
        kind: ActorKind::User,
        id: user.0.clone(),
    };

    let action = match req.action.as_str() {
        "task.assign" => Action::TaskAssign,
        "task.move" => Action::TaskMove,
        "agent.create" => Action::AgentCreate,
        "agent.delete" => Action::AgentDelete,
        "agent.reparent" => Action::AgentReparent,
        "cron.toggle" => Action::CronToggle,
        "cron.run" => Action::CronRun,
        _ => {
            return Ok(Json(PolicyCheckResp {
                allowed: true,
                reason: None,
                needs_override: false,
            }));
        }
    };

    let target_rec = if let Some(ref tid) = req.target_agent {
        policy::load_agent(&state.pool, tid).await.map_err(internal)?
    } else {
        None
    };

    // Check without override first
    let decision_without = policy::authorize(
        &actor,
        &action,
        req.target_agent.as_deref(),
        None,
        target_rec.as_ref(),
        false,
        &OverrideCtx::default(),
        &policy_cfg,
    );

    if decision_without.is_allowed() {
        return Ok(Json(PolicyCheckResp {
            allowed: true,
            reason: None,
            needs_override: false,
        }));
    }

    // Denied without override — check if user has active override
    let has_override = policy::has_active_override(&state.pool, &user.0)
        .await
        .map_err(internal)?;

    if has_override {
        return Ok(Json(PolicyCheckResp {
            allowed: true,
            reason: None,
            needs_override: false,
        }));
    }

    Ok(Json(PolicyCheckResp {
        allowed: false,
        reason: decision_without.deny_reason().map(|s| s.to_string()),
        needs_override: true,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/agents
// ---------------------------------------------------------------------------

pub async fn agents_list(
    _user: AuthedUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<mc::db::AgentRow>>, (StatusCode, String)> {
    let agents = mc::db::list_agents_full(&state.pool)
        .await
        .map_err(internal)?;
    Ok(Json(agents))
}

// ---------------------------------------------------------------------------
// POST /api/agents
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateAgentReq {
    pub id: String,
    pub display_name: String,
    pub role: String,
    pub parent_id: Option<String>,
    #[serde(default, rename = "override")]
    pub override_flag: bool,
    #[serde(default)]
    pub override_reason: Option<String>,
}

pub async fn agents_create(
    user: AuthedUser,
    State(state): State<AppState>,
    Json(req): Json<CreateAgentReq>,
) -> Result<(StatusCode, Json<mc::db::AgentRow>), (StatusCode, String)> {
    // Validate id not empty
    if req.id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid_id","reason":"Agent id must not be empty."}"#.into(),
        ));
    }

    // Validate role
    if !mc::db::VALID_ROLES.contains(&req.role.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            serde_json::to_string(&serde_json::json!({
                "error": "invalid_role",
                "reason": format!("Role must be one of: {:?}", mc::db::VALID_ROLES),
            }))
            .unwrap(),
        ));
    }

    // Validate parent exists if provided
    if let Some(ref pid) = req.parent_id {
        let parent = mc::db::get_agent(&state.pool, pid)
            .await
            .map_err(internal)?;
        if parent.is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                serde_json::to_string(&serde_json::json!({
                    "error": "parent_not_found",
                    "reason": format!("Parent agent '{}' does not exist.", pid),
                }))
                .unwrap(),
            ));
        }
    }

    // Policy check: creating an agent is a hierarchy mutation
    let actor = Actor {
        kind: ActorKind::User,
        id: user.0.clone(),
    };
    let override_ctx = OverrideCtx {
        override_flag: req.override_flag,
        override_reason: req.override_reason.clone(),
    };
    let policy_cfg = state.cfg.policy_config();

    let has_override = policy::has_active_override(&state.pool, &user.0)
        .await
        .map_err(internal)?;

    // Target for agent creation: the parent agent (if any), or root.
    let target_agent_id = req.parent_id.as_deref();
    let target_rec = if let Some(tid) = target_agent_id {
        policy::load_agent(&state.pool, tid).await.map_err(internal)?
    } else {
        None
    };

    let decision = policy::authorize(
        &actor,
        &Action::AgentCreate,
        target_agent_id,
        None,
        target_rec.as_ref(),
        has_override,
        &override_ctx,
        &policy_cfg,
    );

    let payload = serde_json::to_string(&serde_json::json!({
        "agent_id": req.id,
        "display_name": req.display_name,
        "role": req.role,
        "parent_id": req.parent_id,
    }))
    .unwrap();

    mc::db::audit_policy(
        &state.pool,
        &ActorKind::User,
        &user.0,
        &decision,
        "agent.create",
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

    // Create the agent
    let agent = mc::db::create_agent(
        &state.pool,
        &req.id,
        &req.display_name,
        &req.role,
        req.parent_id.as_deref(),
    )
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            (
                StatusCode::CONFLICT,
                serde_json::to_string(&serde_json::json!({
                    "error": "agent_exists",
                    "reason": format!("Agent '{}' already exists.", req.id),
                }))
                .unwrap(),
            )
        } else {
            internal(e)
        }
    })?;

    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("agent.create:{}", req.id),
    });

    Ok((StatusCode::CREATED, Json(agent)))
}

// ---------------------------------------------------------------------------
// DELETE /api/agents/:id
// ---------------------------------------------------------------------------

pub async fn agents_delete(
    user: AuthedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let policy_cfg = state.cfg.policy_config();

    // Forbid deleting the root agent.
    if id == policy_cfg.root_agent_id {
        let payload = serde_json::to_string(&serde_json::json!({"agent_id": id})).unwrap();
        mc::db::audit_policy(
            &state.pool,
            &ActorKind::User,
            &user.0,
            &PolicyDecision::Deny {
                reason: "Cannot delete root agent.".into(),
            },
            "agent.delete",
            &payload,
        )
        .await
        .map_err(internal)?;

        return Err((
            StatusCode::FORBIDDEN,
            r#"{"error":"cannot_delete_root","reason":"Cannot delete the root agent."}"#.into(),
        ));
    }

    // Check agent exists
    let agent = mc::db::get_agent(&state.pool, &id)
        .await
        .map_err(internal)?;
    if agent.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            serde_json::to_string(&serde_json::json!({
                "error": "not_found",
                "reason": format!("Agent '{}' does not exist.", id),
            }))
            .unwrap(),
        ));
    }

    // Forbid deleting agents that have children (must reparent first).
    let has_children = mc::db::has_children(&state.pool, &id)
        .await
        .map_err(internal)?;
    if has_children {
        return Err((
            StatusCode::CONFLICT,
            serde_json::to_string(&serde_json::json!({
                "error": "has_children",
                "reason": format!("Agent '{}' has children. Reparent them first.", id),
            }))
            .unwrap(),
        ));
    }

    // Policy check
    let actor = Actor {
        kind: ActorKind::User,
        id: user.0.clone(),
    };
    let override_ctx = OverrideCtx::default();
    let has_override = policy::has_active_override(&state.pool, &user.0)
        .await
        .map_err(internal)?;

    // Target is the agent being deleted (and its parent for scope check).
    let agent_rec = agent.as_ref().and_then(|a| {
        Some(policy::AgentRecord {
            id: a.id.clone(),
            role: a.role.clone(),
            parent_id: a.parent_id.clone(),
        })
    });

    let decision = policy::authorize(
        &actor,
        &Action::AgentDelete,
        Some(&id),
        None,
        agent_rec.as_ref(),
        has_override,
        &override_ctx,
        &policy_cfg,
    );

    let payload = serde_json::to_string(&serde_json::json!({"agent_id": id})).unwrap();
    mc::db::audit_policy(
        &state.pool,
        &ActorKind::User,
        &user.0,
        &decision,
        "agent.delete",
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

    let deleted = mc::db::delete_agent(&state.pool, &id)
        .await
        .map_err(internal)?;

    if !deleted {
        return Err((
            StatusCode::NOT_FOUND,
            serde_json::to_string(&serde_json::json!({
                "error": "not_found",
                "reason": format!("Agent '{}' does not exist.", id),
            }))
            .unwrap(),
        ));
    }

    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("agent.delete:{id}"),
    });

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// POST /api/agents/:id/reparent
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct ReparentReq {
    pub parent_id: Option<String>,
    #[serde(default, rename = "override")]
    pub override_flag: bool,
    #[serde(default)]
    pub override_reason: Option<String>,
}

pub async fn agents_reparent(
    user: AuthedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ReparentReq>,
) -> Result<Json<mc::db::AgentRow>, (StatusCode, String)> {
    let policy_cfg = state.cfg.policy_config();

    // Agent must exist
    let agent = mc::db::get_agent(&state.pool, &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                serde_json::to_string(&serde_json::json!({
                    "error": "not_found",
                    "reason": format!("Agent '{}' does not exist.", id),
                }))
                .unwrap(),
            )
        })?;

    // Validate new parent exists (if provided)
    if let Some(ref new_pid) = req.parent_id {
        let parent = mc::db::get_agent(&state.pool, new_pid)
            .await
            .map_err(internal)?;
        if parent.is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                serde_json::to_string(&serde_json::json!({
                    "error": "parent_not_found",
                    "reason": format!("Parent agent '{}' does not exist.", new_pid),
                }))
                .unwrap(),
            ));
        }

        // Check for cycles
        let would_cycle = mc::db::would_create_cycle(&state.pool, &id, new_pid)
            .await
            .map_err(internal)?;
        if would_cycle {
            return Err((
                StatusCode::CONFLICT,
                serde_json::to_string(&serde_json::json!({
                    "error": "cycle_detected",
                    "reason": format!("Reparenting '{}' under '{}' would create a cycle.", id, new_pid),
                }))
                .unwrap(),
            ));
        }
    }

    // Policy check
    let actor = Actor {
        kind: ActorKind::User,
        id: user.0.clone(),
    };
    let override_ctx = OverrideCtx {
        override_flag: req.override_flag,
        override_reason: req.override_reason.clone(),
    };
    let has_override = policy::has_active_override(&state.pool, &user.0)
        .await
        .map_err(internal)?;

    let agent_rec = policy::AgentRecord {
        id: agent.id.clone(),
        role: agent.role.clone(),
        parent_id: agent.parent_id.clone(),
    };

    let decision = policy::authorize(
        &actor,
        &Action::AgentReparent,
        Some(&id),
        None,
        Some(&agent_rec),
        has_override,
        &override_ctx,
        &policy_cfg,
    );

    let payload = serde_json::to_string(&serde_json::json!({
        "agent_id": id,
        "old_parent_id": agent.parent_id,
        "new_parent_id": req.parent_id,
    }))
    .unwrap();

    mc::db::audit_policy(
        &state.pool,
        &ActorKind::User,
        &user.0,
        &decision,
        "agent.reparent",
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

    // Perform the reparent
    mc::db::reparent_agent(&state.pool, &id, req.parent_id.as_deref())
        .await
        .map_err(internal)?;

    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("agent.reparent:{id}"),
    });

    // Return updated agent
    let updated = mc::db::get_agent(&state.pool, &id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal("agent disappeared after reparent"))?;

    Ok(Json(updated))
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
