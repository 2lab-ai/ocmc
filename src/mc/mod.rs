pub mod auth;
pub mod auth_http;
pub mod bd;
pub mod cron;
pub mod db;
pub mod handlers;
pub mod poller;
pub mod ws;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct McConfig {
    pub sqlite_url: String,
    pub bind_host: String,
    pub bind_port: u16,
    pub poll_ms: u64,

    // external tools (mounted from host)
    pub bd_bin: String,

    // openclaw gateway
    pub gateway_url: String,
    pub gateway_token: Option<String>,
    pub gateway_password: Option<String>,

    // auth bootstrap
    pub admin_user: String,
    pub admin_pass: String,
}

impl McConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            sqlite_url: std::env::var("MC_SQLITE_URL")
                .unwrap_or_else(|_| "sqlite:///data/mc.db".to_string()),
            bind_host: std::env::var("MC_BIND_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            bind_port: std::env::var("MC_BIND_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3000),
            poll_ms: std::env::var("MC_POLL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5000),
            bd_bin: std::env::var("MC_BD_BIN").unwrap_or_else(|_| "/hostbin/bd".to_string()),
            gateway_url: std::env::var("MC_GATEWAY_URL").unwrap_or_else(|_| "ws://127.0.0.1:18789".to_string()),
            gateway_token: std::env::var("MC_GATEWAY_TOKEN").ok().filter(|s| !s.trim().is_empty()),
            gateway_password: std::env::var("MC_GATEWAY_PASSWORD").ok().filter(|s| !s.trim().is_empty()),
            admin_user: std::env::var("MC_ADMIN_USER").unwrap_or_else(|_| "admin".to_string()),
            admin_pass: std::env::var("MC_ADMIN_PASS").unwrap_or_else(|_| "change-me".to_string()),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct CacheState {
    pub last_snapshot_at: Option<DateTime<Utc>>,
    pub snapshot: Option<KanbanSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum McEvent {
    Refresh { at: DateTime<Utc>, reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct KanbanSnapshot {
    pub generated_at: DateTime<Utc>,
    pub agents: Vec<Agent>,
    pub tasks: Vec<TaskCard>,
    pub cron: Vec<CronCard>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub display_name: String,
    pub current_card_id: Option<String>, // task:<id> or cron:<id>
    pub state: String,                  // doing|waiting
    pub last_event_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskCard {
    pub id: String, // bd id
    pub title: String,
    pub priority: Option<String>,
    pub status: String,
    pub labels: Vec<String>,
    pub assignee: Option<String>,
    pub lane: String, // Backlog|Ready|Doing|Blocked|Done
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CronCard {
    pub id: String, // cron job id
    pub name: String,
    pub enabled: bool,
    pub schedule: String,
    pub next_run_at_ms: Option<i64>,
    pub lane: String, // Scheduled|Disabled
}
