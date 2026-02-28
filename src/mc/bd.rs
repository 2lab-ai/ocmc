use std::process::Stdio;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};
use tokio::process::Command;

use super::{McConfig, TaskCard};

#[derive(Debug, Deserialize)]
struct BdIssue {
    id: String,
    title: String,
    #[serde(default, deserialize_with = "deserialize_priority")]
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

fn deserialize_priority<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Priority {
        String(String),
        Number(i64),
    }

    Ok(
        Option::<Priority>::deserialize(deserializer)?.map(|priority| match priority {
            Priority::String(value) => value,
            Priority::Number(value) => value.to_string(),
        }),
    )
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
    // Labels take priority over status (PRD §4.1).
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

    // No mc/* label — fall back to bd status.
    // Tasks with a clear status map to the corresponding lane;
    // everything else goes to Waiting Room (PRD §4.1).
    match i.status.as_str() {
        "in_progress" => "Doing".to_string(),
        "blocked" => "Blocked".to_string(),
        "closed" => "Done".to_string(),
        _ => "Waiting Room".to_string(),
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
        // Waiting Room: remove all mc/* lane labels (task has no explicit lane)
        "Waiting Room" => None,
        _ => None,
    };
    let remove = all
        .into_iter()
        .filter(|l| Some(*l) != target)
        .collect::<Vec<_>>();
    let add = target.into_iter().collect::<Vec<_>>();
    (add, remove)
}

// ---------------------------------------------------------------------------
// Unit tests [mc-b1j.13]
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ────────────────────────────────────────────────────────
    fn issue(status: &str, labels: &[&str]) -> BdIssue {
        BdIssue {
            id: "test-1".into(),
            title: "test issue".into(),
            priority: None,
            status: status.into(),
            labels: labels.iter().map(|s| (*s).to_string()).collect(),
            assignee: None,
            updated_at: None,
        }
    }

    // ── JSON parsing fixtures ─────────────────────────────────────────
    const FIXTURE_JSON: &str = r#"[
        {
            "id": "mc-abc.1",
            "title": "First task",
            "priority": "high",
            "status": "open",
            "labels": ["mc/backlog"],
            "assignee": "agent-1",
            "updated_at": "2026-01-15T10:00:00Z"
        },
        {
            "id": "mc-abc.2",
            "title": "Second task (in progress)",
            "status": "in_progress",
            "labels": [],
            "assignee": null
        },
        {
            "id": "mc-abc.3",
            "title": "Third task (closed, labeled done)",
            "status": "closed",
            "labels": ["mc/done", "important"],
            "assignee": "agent-2",
            "updated_at": "2026-02-01T08:30:00Z"
        },
        {
            "id": "mc-abc.4",
            "title": "Minimal task"
        }
    ]"#;

    #[test]
    fn parse_fixture_json() {
        let issues: Vec<BdIssue> =
            serde_json::from_str(FIXTURE_JSON).expect("fixture JSON should parse");
        assert_eq!(issues.len(), 4);

        assert_eq!(issues[0].id, "mc-abc.1");
        assert_eq!(issues[0].title, "First task");
        assert_eq!(issues[0].priority.as_deref(), Some("high"));
        assert_eq!(issues[0].status, "open");
        assert_eq!(issues[0].labels, vec!["mc/backlog"]);
        assert_eq!(issues[0].assignee.as_deref(), Some("agent-1"));
        assert!(issues[0].updated_at.is_some());

        // in_progress, empty labels, null assignee
        assert_eq!(issues[1].status, "in_progress");
        assert!(issues[1].labels.is_empty());
        assert!(issues[1].assignee.is_none());

        // closed with mc/done + extra label
        assert_eq!(issues[2].status, "closed");
        assert!(issues[2].labels.contains(&"mc/done".to_string()));
        assert!(issues[2].labels.contains(&"important".to_string()));

        // Minimal: missing optional fields default correctly
        assert_eq!(issues[3].id, "mc-abc.4");
        assert_eq!(issues[3].status, "");           // serde default
        assert!(issues[3].labels.is_empty());
        assert!(issues[3].priority.is_none());
        assert!(issues[3].assignee.is_none());
        assert!(issues[3].updated_at.is_none());
    }

    #[test]
    fn parse_priority_string_and_numeric_variants() {
        let fixture = r#"[
            {"id":"mc-prio.1","title":"Numeric priority","priority":0},
            {"id":"mc-prio.2","title":"String priority","priority":"P1"}
        ]"#;

        let issues: Vec<BdIssue> = serde_json::from_str(fixture).expect("fixture JSON should parse");

        assert_eq!(issues[0].priority.as_deref(), Some("0"));
        assert_eq!(issues[1].priority.as_deref(), Some("P1"));
    }

    // ── lane_from_issue: label-based mapping ──────────────────────────
    #[test]
    fn lane_label_backlog() {
        assert_eq!(lane_from_issue(&issue("open", &["mc/backlog"])), "Backlog");
    }

    #[test]
    fn lane_label_ready() {
        assert_eq!(lane_from_issue(&issue("open", &["mc/ready"])), "Ready");
    }

    #[test]
    fn lane_label_doing() {
        assert_eq!(lane_from_issue(&issue("open", &["mc/doing"])), "Doing");
    }

    #[test]
    fn lane_label_blocked() {
        assert_eq!(lane_from_issue(&issue("open", &["mc/blocked"])), "Blocked");
    }

    #[test]
    fn lane_label_done() {
        assert_eq!(lane_from_issue(&issue("open", &["mc/done"])), "Done");
    }

    // ── lane_from_issue: label priority over status ───────────────────
    #[test]
    fn lane_label_overrides_status() {
        // Label says Ready, status says in_progress → label wins
        assert_eq!(
            lane_from_issue(&issue("in_progress", &["mc/ready"])),
            "Ready"
        );
        // Label says Blocked, status says closed → label wins
        assert_eq!(
            lane_from_issue(&issue("closed", &["mc/blocked"])),
            "Blocked"
        );
    }

    // ── lane_from_issue: status fallback (no mc/* labels) ─────────────
    #[test]
    fn lane_status_in_progress() {
        assert_eq!(lane_from_issue(&issue("in_progress", &[])), "Doing");
    }

    #[test]
    fn lane_status_blocked() {
        assert_eq!(lane_from_issue(&issue("blocked", &[])), "Blocked");
    }

    #[test]
    fn lane_status_closed() {
        assert_eq!(lane_from_issue(&issue("closed", &[])), "Done");
    }

    // ── lane_from_issue: Waiting Room fallback ────────────────────────
    #[test]
    fn lane_open_no_labels_goes_to_waiting_room() {
        assert_eq!(lane_from_issue(&issue("open", &[])), "Waiting Room");
    }

    #[test]
    fn lane_unknown_status_no_labels_goes_to_waiting_room() {
        assert_eq!(lane_from_issue(&issue("whatever", &[])), "Waiting Room");
    }

    #[test]
    fn lane_empty_status_no_labels_goes_to_waiting_room() {
        assert_eq!(lane_from_issue(&issue("", &[])), "Waiting Room");
    }

    // ── lane_from_issue: edge cases ───────────────────────────────────
    #[test]
    fn lane_non_mc_labels_ignored() {
        // Non-mc/* labels should not affect lane mapping
        assert_eq!(
            lane_from_issue(&issue("open", &["important", "bug"])),
            "Waiting Room"
        );
    }

    #[test]
    fn lane_first_mc_label_wins() {
        // Multiple mc/ labels — first match wins (Backlog before Ready)
        assert_eq!(
            lane_from_issue(&issue("open", &["mc/backlog", "mc/ready"])),
            "Backlog"
        );
    }

    // ── labels_for_lane ───────────────────────────────────────────────
    #[test]
    fn labels_for_lane_backlog() {
        let (add, remove) = labels_for_lane("Backlog");
        assert_eq!(add, vec!["mc/backlog"]);
        assert!(remove.contains(&"mc/ready"));
        assert!(remove.contains(&"mc/doing"));
        assert!(remove.contains(&"mc/blocked"));
        assert!(remove.contains(&"mc/done"));
        assert!(!remove.contains(&"mc/backlog"));
    }

    #[test]
    fn labels_for_lane_ready() {
        let (add, remove) = labels_for_lane("Ready");
        assert_eq!(add, vec!["mc/ready"]);
        assert!(!remove.contains(&"mc/ready"));
        assert_eq!(remove.len(), 4);
    }

    #[test]
    fn labels_for_lane_doing() {
        let (add, remove) = labels_for_lane("Doing");
        assert_eq!(add, vec!["mc/doing"]);
        assert!(!remove.contains(&"mc/doing"));
    }

    #[test]
    fn labels_for_lane_blocked() {
        let (add, remove) = labels_for_lane("Blocked");
        assert_eq!(add, vec!["mc/blocked"]);
        assert!(!remove.contains(&"mc/blocked"));
    }

    #[test]
    fn labels_for_lane_done() {
        let (add, remove) = labels_for_lane("Done");
        assert_eq!(add, vec!["mc/done"]);
        assert!(!remove.contains(&"mc/done"));
    }

    #[test]
    fn labels_for_lane_waiting_room() {
        let (add, remove) = labels_for_lane("Waiting Room");
        assert!(add.is_empty(), "Waiting Room should add no labels");
        assert_eq!(remove.len(), 5, "should remove all mc/* lane labels");
    }

    #[test]
    fn labels_for_lane_unknown() {
        let (add, remove) = labels_for_lane("Nonexistent");
        assert!(add.is_empty());
        assert_eq!(remove.len(), 5);
    }
}
