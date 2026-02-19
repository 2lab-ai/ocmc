//! Central policy engine for Mission Control routing guardrails.
//!
//! Every mutating endpoint must call `authorize()` before executing.
//!
//! Rules (PRD §3 / ADR-015):
//! - Default CEO/user direct control: root-only surface (root = MC_ROOT_AGENT_ID, default "main")
//! - Without override: deny main → dev/qa direct control/dispatch
//! - Allow pdpm → its child dev/qa control
//! - CEO override: explicit flag + reason, time-bounded, audit-logged

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Who is performing the action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    User,
    Agent,
}

impl std::fmt::Display for ActorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActorKind::User => write!(f, "user"),
            ActorKind::Agent => write!(f, "agent"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: String,
}

/// Actions that can be guarded by the policy engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    TaskAssign,
    TaskMove,
    AgentCreate,
    AgentDelete,
    AgentReparent,
    CronToggle,
    CronRun,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::TaskAssign => write!(f, "task.assign"),
            Action::TaskMove => write!(f, "task.move"),
            Action::AgentCreate => write!(f, "agent.create"),
            Action::AgentDelete => write!(f, "agent.delete"),
            Action::AgentReparent => write!(f, "agent.reparent"),
            Action::CronToggle => write!(f, "cron.toggle"),
            Action::CronRun => write!(f, "cron.run"),
        }
    }
}

/// Optional override context supplied by the caller.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OverrideCtx {
    #[serde(default)]
    pub override_flag: bool,
    #[serde(default)]
    pub override_reason: Option<String>,
}

/// Result of a policy evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyDecision {
    Allow,
    AllowWithOverride { reason: String },
    Deny { reason: String },
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PolicyDecision::Allow | PolicyDecision::AllowWithOverride { .. })
    }

    pub fn decision_str(&self) -> &'static str {
        match self {
            PolicyDecision::Allow | PolicyDecision::AllowWithOverride { .. } => "allow",
            PolicyDecision::Deny { .. } => "deny",
        }
    }

    pub fn is_override(&self) -> bool {
        matches!(self, PolicyDecision::AllowWithOverride { .. })
    }

    pub fn deny_reason(&self) -> Option<&str> {
        match self {
            PolicyDecision::Deny { reason } => Some(reason),
            _ => None,
        }
    }

    pub fn override_reason(&self) -> Option<&str> {
        match self {
            PolicyDecision::AllowWithOverride { reason } => Some(reason),
            _ => None,
        }
    }
}

/// Agent hierarchy record (from DB).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: String,
    pub role: String,
    pub parent_id: Option<String>,
}

/// Configuration for the policy engine.
#[derive(Debug, Clone)]
pub struct PolicyConfig {
    pub root_agent_id: String,
    pub override_ttl_s: u64,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            root_agent_id: "main".to_string(),
            override_ttl_s: 600,
        }
    }
}

/// Check if an active (non-expired, non-revoked) override session exists for the user.
pub async fn has_active_override(pool: &SqlitePool, username: &str) -> anyhow::Result<bool> {
    let now = Utc::now().to_rfc3339();
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM override_sessions WHERE username = ? AND expires_at > ? AND revoked_at IS NULL ORDER BY enabled_at DESC LIMIT 1",
    )
    .bind(username)
    .bind(&now)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// Load an agent record from the DB.
pub async fn load_agent(pool: &SqlitePool, agent_id: &str) -> anyhow::Result<Option<AgentRecord>> {
    let row: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, role, parent_id FROM agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(id, role, parent_id)| AgentRecord {
        id,
        role,
        parent_id,
    }))
}

/// Check if `potential_child` is a direct child of `potential_parent` in the hierarchy.
pub async fn is_direct_child(
    pool: &SqlitePool,
    potential_parent: &str,
    potential_child: &str,
) -> anyhow::Result<bool> {
    let child = load_agent(pool, potential_child).await?;
    Ok(child.map_or(false, |c| c.parent_id.as_deref() == Some(potential_parent)))
}

/// Core authorization function.
///
/// Evaluates whether `actor` can perform `action` targeting `target_agent_id`.
/// If `target_agent_id` is None, this is an action without a specific agent target
/// (e.g., task.move within the user's own control scope) — generally allowed for users.
pub fn authorize(
    actor: &Actor,
    _action: &Action,
    target_agent_id: Option<&str>,
    actor_agent_record: Option<&AgentRecord>,
    target_agent_record: Option<&AgentRecord>,
    has_override: bool,
    override_ctx: &OverrideCtx,
    policy_cfg: &PolicyConfig,
) -> PolicyDecision {
    // ----- Rule 1: Users (CEO) -----
    if actor.kind == ActorKind::User {
        // Users can always operate on the root agent or on un-targeted actions.
        let Some(target_id) = target_agent_id else {
            return PolicyDecision::Allow;
        };

        // If target IS the root agent → always allowed.
        if target_id == policy_cfg.root_agent_id {
            return PolicyDecision::Allow;
        }

        // Target is NOT root → requires CEO override.
        if override_ctx.override_flag && has_override {
            let reason = override_ctx
                .override_reason
                .clone()
                .unwrap_or_else(|| "CEO override (session active)".to_string());
            return PolicyDecision::AllowWithOverride { reason };
        }

        return PolicyDecision::Deny {
            reason: format!(
                "User direct control is limited to root agent '{}'. Use CEO override to control '{}' directly.",
                policy_cfg.root_agent_id, target_id
            ),
        };
    }

    // ----- Rule 2: Agents -----
    // Agent acting on its own tasks (no specific target agent) → allow.
    let Some(target_id) = target_agent_id else {
        return PolicyDecision::Allow;
    };

    // Agent needs a record to evaluate hierarchy.
    let Some(actor_rec) = actor_agent_record else {
        return PolicyDecision::Deny {
            reason: format!("Agent '{}' has no hierarchy record.", actor.id),
        };
    };

    // Root agent (main) → can control its direct children (pdpm), but NOT grandchildren (dev/qa).
    if actor_rec.id == policy_cfg.root_agent_id {
        // Check if target is a direct child of root.
        if let Some(target_rec) = target_agent_record {
            if target_rec.parent_id.as_deref() == Some(&actor_rec.id) {
                return PolicyDecision::Allow;
            }
        }

        // main → dev/qa (non-direct-child) is DENIED.
        return PolicyDecision::Deny {
            reason: format!(
                "Root agent '{}' cannot directly control '{}'. Route through the target's parent (pdpm).",
                actor_rec.id, target_id
            ),
        };
    }

    // Non-root agent (pdpm, dev, qa, etc.) → can control direct children only.
    if let Some(target_rec) = target_agent_record {
        if target_rec.parent_id.as_deref() == Some(&actor_rec.id) {
            return PolicyDecision::Allow;
        }
    }

    // An agent targeting itself → allowed (e.g., self-reporting).
    if actor_rec.id == target_id {
        return PolicyDecision::Allow;
    }

    PolicyDecision::Deny {
        reason: format!(
            "Agent '{}' (role={}) cannot control '{}': not a direct child.",
            actor_rec.id, actor_rec.role, target_id
        ),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_cfg() -> PolicyConfig {
        PolicyConfig {
            root_agent_id: "main".to_string(),
            override_ttl_s: 600,
        }
    }

    fn no_override() -> OverrideCtx {
        OverrideCtx {
            override_flag: false,
            override_reason: None,
        }
    }

    fn with_override(reason: &str) -> OverrideCtx {
        OverrideCtx {
            override_flag: true,
            override_reason: Some(reason.to_string()),
        }
    }

    fn agent_rec(id: &str, role: &str, parent: Option<&str>) -> AgentRecord {
        AgentRecord {
            id: id.to_string(),
            role: role.to_string(),
            parent_id: parent.map(|s| s.to_string()),
        }
    }

    fn user_actor(name: &str) -> Actor {
        Actor {
            kind: ActorKind::User,
            id: name.to_string(),
        }
    }

    fn agent_actor(id: &str) -> Actor {
        Actor {
            kind: ActorKind::Agent,
            id: id.to_string(),
        }
    }

    // ---- Test 1: User → root agent (main) is ALLOWED ----
    #[test]
    fn test_user_to_root_allowed() {
        let d = authorize(
            &user_actor("admin"),
            &Action::TaskAssign,
            Some("main"),
            None,
            None,
            false,
            &no_override(),
            &default_cfg(),
        );
        assert_eq!(d, PolicyDecision::Allow);
    }

    // ---- Test 2: User → non-root (dev) is DENIED without override ----
    #[test]
    fn test_user_to_dev_denied_no_override() {
        let d = authorize(
            &user_actor("admin"),
            &Action::TaskAssign,
            Some("dev-a"),
            None,
            None,
            false,
            &no_override(),
            &default_cfg(),
        );
        assert!(matches!(d, PolicyDecision::Deny { .. }));
        assert!(d.deny_reason().unwrap().contains("root agent"));
    }

    // ---- Test 3: User → non-root with CEO override is ALLOWED + logged ----
    #[test]
    fn test_user_to_dev_allowed_with_override() {
        let d = authorize(
            &user_actor("admin"),
            &Action::TaskAssign,
            Some("dev-a"),
            None,
            None,
            true, // active override session
            &with_override("emergency hotfix"),
            &default_cfg(),
        );
        assert_eq!(
            d,
            PolicyDecision::AllowWithOverride {
                reason: "emergency hotfix".to_string()
            }
        );
    }

    // ---- Test 4: User with override flag but no active session → DENIED ----
    #[test]
    fn test_user_override_flag_but_no_session_denied() {
        let d = authorize(
            &user_actor("admin"),
            &Action::TaskAssign,
            Some("dev-a"),
            None,
            None,
            false, // no active override session
            &with_override("reason"),
            &default_cfg(),
        );
        assert!(matches!(d, PolicyDecision::Deny { .. }));
    }

    // ---- Test 5: main → dev (not direct child) is DENIED ----
    #[test]
    fn test_main_to_dev_denied() {
        let main_rec = agent_rec("main", "root", None);
        let dev_rec = agent_rec("dev-a", "dev", Some("pdpm-mc"));

        let d = authorize(
            &agent_actor("main"),
            &Action::TaskAssign,
            Some("dev-a"),
            Some(&main_rec),
            Some(&dev_rec),
            false,
            &no_override(),
            &default_cfg(),
        );
        assert!(matches!(d, PolicyDecision::Deny { .. }));
        assert!(d.deny_reason().unwrap().contains("Route through"));
    }

    // ---- Test 6: main → pdpm (direct child) is ALLOWED ----
    #[test]
    fn test_main_to_pdpm_allowed() {
        let main_rec = agent_rec("main", "root", None);
        let pdpm_rec = agent_rec("pdpm-mc", "pdpm", Some("main"));

        let d = authorize(
            &agent_actor("main"),
            &Action::TaskAssign,
            Some("pdpm-mc"),
            Some(&main_rec),
            Some(&pdpm_rec),
            false,
            &no_override(),
            &default_cfg(),
        );
        assert_eq!(d, PolicyDecision::Allow);
    }

    // ---- Test 7: pdpm → its child dev is ALLOWED ----
    #[test]
    fn test_pdpm_to_own_dev_allowed() {
        let pdpm_rec = agent_rec("pdpm-mc", "pdpm", Some("main"));
        let dev_rec = agent_rec("dev-a", "dev", Some("pdpm-mc"));

        let d = authorize(
            &agent_actor("pdpm-mc"),
            &Action::TaskAssign,
            Some("dev-a"),
            Some(&pdpm_rec),
            Some(&dev_rec),
            false,
            &no_override(),
            &default_cfg(),
        );
        assert_eq!(d, PolicyDecision::Allow);
    }

    // ---- Test 8: pdpm → NOT its child dev is DENIED ----
    #[test]
    fn test_pdpm_to_other_dev_denied() {
        let pdpm_rec = agent_rec("pdpm-mc", "pdpm", Some("main"));
        let dev_rec = agent_rec("dev-b", "dev", Some("pdpm-other"));

        let d = authorize(
            &agent_actor("pdpm-mc"),
            &Action::TaskAssign,
            Some("dev-b"),
            Some(&pdpm_rec),
            Some(&dev_rec),
            false,
            &no_override(),
            &default_cfg(),
        );
        assert!(matches!(d, PolicyDecision::Deny { .. }));
    }

    // ---- Test 9: Agent with no hierarchy record is DENIED ----
    #[test]
    fn test_unknown_agent_denied() {
        let d = authorize(
            &agent_actor("rogue"),
            &Action::TaskAssign,
            Some("dev-a"),
            None, // no record
            None,
            false,
            &no_override(),
            &default_cfg(),
        );
        assert!(matches!(d, PolicyDecision::Deny { .. }));
        assert!(d.deny_reason().unwrap().contains("no hierarchy record"));
    }

    // ---- Test 10: Agent targeting itself is ALLOWED ----
    #[test]
    fn test_agent_self_target_allowed() {
        let dev_rec = agent_rec("dev-a", "dev", Some("pdpm-mc"));

        let d = authorize(
            &agent_actor("dev-a"),
            &Action::TaskMove,
            Some("dev-a"),
            Some(&dev_rec),
            Some(&dev_rec),
            false,
            &no_override(),
            &default_cfg(),
        );
        assert_eq!(d, PolicyDecision::Allow);
    }

    // ---- Test 11: User → no target (e.g., general task move) is ALLOWED ----
    #[test]
    fn test_user_no_target_allowed() {
        let d = authorize(
            &user_actor("admin"),
            &Action::TaskMove,
            None,
            None,
            None,
            false,
            &no_override(),
            &default_cfg(),
        );
        assert_eq!(d, PolicyDecision::Allow);
    }

    // ---- Test 12: Agent → no target is ALLOWED ----
    #[test]
    fn test_agent_no_target_allowed() {
        let d = authorize(
            &agent_actor("dev-a"),
            &Action::TaskMove,
            None,
            None,
            None,
            false,
            &no_override(),
            &default_cfg(),
        );
        assert_eq!(d, PolicyDecision::Allow);
    }

    // ---- Test 13: main → qa (grandchild via pdpm) is DENIED ----
    #[test]
    fn test_main_to_qa_denied() {
        let main_rec = agent_rec("main", "root", None);
        let qa_rec = agent_rec("qa-a", "qa", Some("pdpm-mc"));

        let d = authorize(
            &agent_actor("main"),
            &Action::TaskAssign,
            Some("qa-a"),
            Some(&main_rec),
            Some(&qa_rec),
            false,
            &no_override(),
            &default_cfg(),
        );
        assert!(matches!(d, PolicyDecision::Deny { .. }));
    }

    // ---- Test 14: PolicyDecision helpers work correctly ----
    #[test]
    fn test_decision_helpers() {
        let allow = PolicyDecision::Allow;
        assert!(allow.is_allowed());
        assert!(!allow.is_override());
        assert!(allow.deny_reason().is_none());

        let deny = PolicyDecision::Deny {
            reason: "nope".to_string(),
        };
        assert!(!deny.is_allowed());
        assert_eq!(deny.deny_reason(), Some("nope"));

        let ovr = PolicyDecision::AllowWithOverride {
            reason: "hotfix".to_string(),
        };
        assert!(ovr.is_allowed());
        assert!(ovr.is_override());
        assert_eq!(ovr.override_reason(), Some("hotfix"));
    }

    // ---- Test 15: User → root for every action type is ALLOWED ----
    #[test]
    fn test_user_root_all_actions() {
        let cfg = default_cfg();
        for action in [
            Action::TaskAssign,
            Action::TaskMove,
            Action::AgentCreate,
            Action::AgentDelete,
            Action::AgentReparent,
            Action::CronToggle,
            Action::CronRun,
        ] {
            let d = authorize(
                &user_actor("admin"),
                &action,
                Some("main"),
                None,
                None,
                false,
                &no_override(),
                &cfg,
            );
            assert_eq!(d, PolicyDecision::Allow, "failed for action: {action}");
        }
    }
}
