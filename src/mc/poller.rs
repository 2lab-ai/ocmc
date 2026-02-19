use chrono::Utc;
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use super::{bd, cron, db, CacheState, KanbanSnapshot, McConfig};

pub async fn tick(pool: &SqlitePool, cache: &RwLock<CacheState>, cfg: &McConfig) -> anyhow::Result<()> {
    let tasks = bd::list_issues(cfg).await?;
    let cron_cards = cron::list_jobs(cfg).await.unwrap_or_default();

    let agent_rows = db::list_agents(pool).await?;

    // simple assignment: agent.assignee == agent id; if none => waiting
    let mut agents = Vec::new();
    for (id, display_name) in agent_rows {
        let current_task = tasks
            .iter()
            .find(|t| t.assignee.as_deref() == Some(id.as_str()) && t.lane == "Doing")
            .map(|t| format!("task:{}", t.id));

        agents.push(super::Agent {
            id,
            display_name,
            state: if current_task.is_some() { "doing".into() } else { "waiting".into() },
            current_card_id: current_task,
            last_event_at: None,
        });
    }

    let snapshot = KanbanSnapshot {
        generated_at: Utc::now(),
        agents,
        tasks,
        cron: cron_cards,
    };

    let mut w = cache.write().await;
    w.last_snapshot_at = Some(Utc::now());
    w.snapshot = Some(snapshot);
    Ok(())
}
