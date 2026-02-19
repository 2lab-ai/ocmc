use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::{mc, AppState};

use super::auth::AuthedUser;

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

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct MoveReq {
    pub lane: String,
}

pub async fn task_move_post(
    user: AuthedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<MoveReq>,
) -> Result<(), (StatusCode, String)> {
    mc::bd::set_lane(&state.cfg, &id, &req.lane)
        .await
        .map_err(internal)?;

    mc::db::audit(&state.pool, &user.0, "task.move", &serde_json::to_string(&req).unwrap())
        .await
        .map_err(internal)?;

    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("task.move:{id}"),
    });

    Ok(())
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct AssignReq {
    pub assignee: Option<String>,
}

pub async fn task_assign_post(
    user: AuthedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AssignReq>,
) -> Result<(), (StatusCode, String)> {
    mc::bd::set_assignee(&state.cfg, &id, req.assignee.as_deref())
        .await
        .map_err(internal)?;

    mc::db::audit(&state.pool, &user.0, "task.assign", &serde_json::to_string(&req).unwrap())
        .await
        .map_err(internal)?;

    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("task.assign:{id}"),
    });

    Ok(())
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct CronToggleReq {
    pub enabled: bool,
}

pub async fn cron_toggle_post(
    user: AuthedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<CronToggleReq>,
) -> Result<(), (StatusCode, String)> {
    mc::cron::toggle(&state.cfg, &id, req.enabled)
        .await
        .map_err(internal)?;
    mc::db::audit(&state.pool, &user.0, "cron.toggle", &serde_json::to_string(&req).unwrap())
        .await
        .map_err(internal)?;
    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("cron.toggle:{id}"),
    });
    Ok(())
}

pub async fn cron_run_post(
    user: AuthedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(), (StatusCode, String)> {
    mc::cron::run_now(&state.cfg, &id).await.map_err(internal)?;
    mc::db::audit(&state.pool, &user.0, "cron.run", "{}")
        .await
        .map_err(internal)?;
    let _ = state.events_tx.send(mc::McEvent::Refresh {
        at: chrono::Utc::now(),
        reason: format!("cron.run:{id}"),
    });
    Ok(())
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
