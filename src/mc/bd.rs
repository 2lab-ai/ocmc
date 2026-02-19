use std::process::Stdio;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::process::Command;

use super::{McConfig, TaskCard};

#[derive(Debug, Deserialize)]
struct BdIssue {
    id: String,
    title: String,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    assignee: Option<String>,
    #[serde(default)]
    updated_at: Option<DateTime<Utc>>,
}

pub async fn list_issues(cfg: &McConfig) -> anyhow::Result<Vec<TaskCard>> {
    let out = Command::new(&cfg.bd_bin)
        .args(["list", "--json", "--limit", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("run bd list")?;

    if !out.status.success() {
        return Err(anyhow::anyhow!(
            "bd list failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let issues: Vec<BdIssue> = serde_json::from_slice(&out.stdout)
        .context("parse bd list --json output")?;

    Ok(issues
        .into_iter()
        .map(|i| {
            let lane = lane_from_issue(&i);
            TaskCard {
                id: i.id,
                title: i.title,
                priority: i.priority,
                status: i.status,
                labels: i.labels,
                assignee: i.assignee,
                lane,
                updated_at: i.updated_at,
            }
        })
        .collect())
}

fn lane_from_issue(i: &BdIssue) -> String {
    // Fixed lanes via labels, fallback from bd status.
    if i.labels.iter().any(|l| l == "mc/backlog") {
        return "Backlog".to_string();
    }
    if i.labels.iter().any(|l| l == "mc/ready") {
        return "Ready".to_string();
    }
    if i.labels.iter().any(|l| l == "mc/doing") {
        return "Doing".to_string();
    }
    if i.labels.iter().any(|l| l == "mc/blocked") {
        return "Blocked".to_string();
    }
    if i.labels.iter().any(|l| l == "mc/done") {
        return "Done".to_string();
    }

    match i.status.as_str() {
        "in_progress" => "Doing".to_string(),
        "blocked" => "Blocked".to_string(),
        "closed" => "Done".to_string(),
        _ => "Ready".to_string(),
    }
}

pub async fn set_lane(cfg: &McConfig, id: &str, lane: &str) -> anyhow::Result<()> {
    let (add, remove) = labels_for_lane(lane);

    let mut cmd = Command::new(&cfg.bd_bin);
    cmd.arg("update").arg(id);

    for r in remove {
        cmd.args(["--remove-label", r]);
    }
    for a in add {
        cmd.args(["--add-label", a]);
    }

    // Keep bd status loosely aligned
    match lane {
        "Doing" => {
            cmd.args(["--status", "in_progress"]);
        }
        "Blocked" => {
            cmd.args(["--status", "blocked"]);
        }
        "Done" => {
            cmd.args(["--status", "closed"]);
        }
        _ => {}
    }

    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(anyhow::anyhow!(
            "bd update failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

pub async fn set_assignee(cfg: &McConfig, id: &str, assignee: Option<&str>) -> anyhow::Result<()> {
    let mut cmd = Command::new(&cfg.bd_bin);
    cmd.arg("update").arg(id);
    if let Some(a) = assignee {
        cmd.args(["--assignee", a]);
    } else {
        cmd.args(["--assignee", ""]);
    }

    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(anyhow::anyhow!(
            "bd update assignee failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn labels_for_lane(lane: &str) -> (Vec<&'static str>, Vec<&'static str>) {
    let all = ["mc/backlog", "mc/ready", "mc/doing", "mc/blocked", "mc/done"];
    let target = match lane {
        "Backlog" => Some("mc/backlog"),
        "Ready" => Some("mc/ready"),
        "Doing" => Some("mc/doing"),
        "Blocked" => Some("mc/blocked"),
        "Done" => Some("mc/done"),
        _ => None,
    };
    let remove = all
        .into_iter()
        .filter(|l| Some(*l) != target)
        .collect::<Vec<_>>();
    let add = target.into_iter().collect::<Vec<_>>();
    (add, remove)
}
