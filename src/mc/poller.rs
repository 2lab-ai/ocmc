use chrono::Utc;
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use super::{bd, cron, CacheState, KanbanSnapshot, McConfig};

pub async fn tick(
    pool: &SqlitePool,
    cache: &RwLock<CacheState>,
    cfg: &McConfig,
) -> anyhow::Result<()> {
    let tasks = bd::list_issues(cfg).await?;
    let cron_cards = cron::list_jobs(cfg).await.unwrap_or_default();

    let agent_rows = list_agents_full(pool).await?;

    // simple assignment: agent.assignee == agent id; if none => waiting
    let mut agents = Vec::new();
    for (id, display_name, role, parent_id) in agent_rows {
        let current_task = tasks
            .iter()
            .find(|t| t.assignee.as_deref() == Some(id.as_str()) && t.lane == "Doing")
            .map(|t| format!("task:{}", t.id));

        agents.push(super::Agent {
            id,
            display_name,
            role,
            parent_id,
            state: if current_task.is_some() {
                "doing".into()
            } else {
                "waiting".into()
            },
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

async fn list_agents_full(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<(String, String, String, Option<String>)>> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, display_name, role, parent_id FROM agents ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
