// NOTE: OpenClaw cron integration is via Gateway RPC (not CLI),
// because the CLI subcommand wiring may be unavailable in some deployments.
// This module will be replaced with ws-RPC client implementation.

use super::{CronCard, McConfig};

pub async fn list_jobs(_cfg: &McConfig) -> anyhow::Result<Vec<CronCard>> {
    Ok(vec![])
}

pub async fn toggle(_cfg: &McConfig, _id: &str, _enabled: bool) -> anyhow::Result<()> {
    Ok(())
}

pub async fn run_now(_cfg: &McConfig, _id: &str) -> anyhow::Result<()> {
    Ok(())
}
