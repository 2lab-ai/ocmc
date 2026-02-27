///! Integration tests using mock bd binary (mc-b1j.14)
///!
///! These tests set MC_BD_BIN to tests/mock_bd.sh and exercise:
///! - bd::list_issues → lane mapping
///! - poller::tick → KanbanSnapshot
///! - bd::set_lane → task move flow end-to-end

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use mission_control::mc::{bd, McConfig, CacheState, KanbanSnapshot};

/// Serialize tests that mutate process-wide env vars (MOCK_BD_ARGS_LOG).
static ENV_MUTEX: Mutex<()> = Mutex::new(());
static LOG_COUNTER: AtomicU64 = AtomicU64::new(0);

struct MockArgsLog {
    path: PathBuf,
}

impl MockArgsLog {
    fn install(prefix: &str) -> Self {
        let seq = LOG_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "{}_{}_{}.log",
            prefix,
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_file(&path);
        unsafe { std::env::set_var("MOCK_BD_ARGS_LOG", &path); }
        Self { path }
    }

    fn path(&self) -> &std::path::Path {
        self.path.as_path()
    }
}

impl Drop for MockArgsLog {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        unsafe { std::env::remove_var("MOCK_BD_ARGS_LOG"); }
    }
}

/// Build a McConfig pointing at the mock bd binary.
fn mock_config() -> McConfig {
    let mock_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("mock_bd.sh");
    McConfig {
        sqlite_url: "sqlite::memory:".to_string(),
        bind_host: "127.0.0.1".to_string(),
        bind_port: 0,
        poll_ms: 60_000,
        bd_bin: mock_path.to_string_lossy().to_string(),
        gateway_url: "ws://127.0.0.1:1".to_string(),
        gateway_token: None,
        gateway_password: None,
        admin_user: "test".to_string(),
        admin_pass: "test".to_string(),
        session_secret: "test-secret".to_string(),
        root_agent_id: "main".to_string(),
        override_ttl_s: 600,
    }
}

// ═══════════════════════════════════════════════════════════════════
// bd::list_issues with mock
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn list_issues_returns_all_fixture_tasks() {
    let cfg = mock_config();
    let tasks = bd::list_issues(&cfg).await.expect("list_issues should succeed");
    assert_eq!(tasks.len(), 7);
}

#[tokio::test]
async fn list_issues_lane_mapping_is_correct() {
    let cfg = mock_config();
    let tasks = bd::list_issues(&cfg).await.unwrap();

    let lane_of = |id: &str| -> String {
        tasks.iter().find(|t| t.id == id).unwrap().lane.clone()
    };

    assert_eq!(lane_of("mc-b1j.1"), "Done",         "closed + mc/done → Done");
    assert_eq!(lane_of("mc-b1j.2"), "Doing",         "mc/doing label → Doing");
    assert_eq!(lane_of("mc-b1j.3"), "Ready",         "mc/ready label → Ready");
    assert_eq!(lane_of("mc-b1j.4"), "Blocked",       "mc/blocked label → Blocked");
    assert_eq!(lane_of("mc-b1j.5"), "Backlog",       "mc/backlog label → Backlog");
    assert_eq!(lane_of("mc-b1j.6"), "Waiting Room",  "open, no labels → Waiting Room");
    assert_eq!(lane_of("mc-b1j.7"), "Doing",         "in_progress status, no mc/ labels → Doing");
}

#[tokio::test]
async fn list_issues_preserves_metadata() {
    let cfg = mock_config();
    let tasks = bd::list_issues(&cfg).await.unwrap();

    let t1 = tasks.iter().find(|t| t.id == "mc-b1j.1").unwrap();
    assert_eq!(t1.title, "Remove Leptos UI");
    assert_eq!(t1.assignee.as_deref(), Some("opus46"));
    assert_eq!(t1.priority.as_deref(), Some("high"));
    assert!(t1.updated_at.is_some());

    let t2 = tasks.iter().find(|t| t.id == "mc-b1j.2").unwrap();
    assert!(t2.labels.contains(&"bug".to_string()));
}

// ═══════════════════════════════════════════════════════════════════
// poller::tick → KanbanSnapshot (requires SQLite)
// ═══════════════════════════════════════════════════════════════════

async fn setup_db_and_tick(cfg: &McConfig) -> KanbanSnapshot {
    use sqlx::SqlitePool;
    use tokio::sync::RwLock;

    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite");

    // Create the agents table (poller queries it)
    sqlx::query(
        "CREATE TABLE agents (
            id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'worker',
            parent_id TEXT
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Insert a test agent
    sqlx::query("INSERT INTO agents (id, display_name, role) VALUES ('opus46', 'Opus 46', 'worker')")
        .execute(&pool)
        .await
        .unwrap();

    let cache = RwLock::new(CacheState::default());

    mission_control::mc::poller::tick(&pool, &cache, cfg)
        .await
        .expect("tick should succeed");

    let guard = cache.read().await;
    guard.snapshot.clone().expect("snapshot should be set after tick")
}

#[tokio::test]
async fn poller_tick_produces_snapshot_with_all_tasks() {
    let cfg = mock_config();
    let snap = setup_db_and_tick(&cfg).await;
    assert_eq!(snap.tasks.len(), 7);
}

#[tokio::test]
async fn poller_tick_snapshot_has_agents() {
    let cfg = mock_config();
    let snap = setup_db_and_tick(&cfg).await;

    assert_eq!(snap.agents.len(), 1);
    assert_eq!(snap.agents[0].id, "opus46");
    // opus46 is assignee on mc-b1j.2 which is "Doing"
    assert_eq!(snap.agents[0].state, "doing");
    assert!(snap.agents[0].current_card_id.is_some());
}

#[tokio::test]
async fn poller_tick_snapshot_generated_at_is_set() {
    let cfg = mock_config();
    let snap = setup_db_and_tick(&cfg).await;
    let now = chrono::Utc::now();
    let diff = (now - snap.generated_at).num_seconds();
    assert!(diff < 5, "generated_at should be recent, diff={}s", diff);
}

// ═══════════════════════════════════════════════════════════════════
// Task move flow end-to-end via mock
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn set_lane_calls_mock_with_correct_args() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = mock_config();
    let args_log = MockArgsLog::install("mock_bd_args");

    bd::set_lane(&cfg, "mc-b1j.6", "Doing").await.expect("set_lane should succeed");

    let log = std::fs::read_to_string(args_log.path()).expect("args log should exist");
    assert!(log.contains("update mc-b1j.6"), "should call update with issue id, got: {}", log);
    assert!(log.contains("--add-label mc/doing"), "should add mc/doing label, got: {}", log);
    assert!(log.contains("--status in_progress"), "should set status, got: {}", log);
}

#[tokio::test]
async fn move_task_to_each_lane_via_mock() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = mock_config();
    let args_log = MockArgsLog::install("mock_bd_lanes");

    for lane in &["Backlog", "Ready", "Doing", "Blocked", "Done", "Waiting Room"] {
        bd::set_lane(&cfg, "test-1", lane)
            .await
            .unwrap_or_else(|e| panic!("set_lane to {} failed: {}", lane, e));
    }

    let log = std::fs::read_to_string(args_log.path()).unwrap();
    let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("update ")).collect();
    assert_eq!(lines.len(), 6, "expected 6 update calls, got: {}", log);

    assert!(lines[0].contains("--add-label mc/backlog"));
    assert!(lines[1].contains("--add-label mc/ready"));
    assert!(lines[2].contains("--add-label mc/doing"));
    assert!(lines[3].contains("--add-label mc/blocked"));
    assert!(lines[4].contains("--add-label mc/done"));
    // Waiting Room adds no labels, removes all
    assert!(!lines[5].contains("--add-label"), "Waiting Room should add no labels");
    assert!(lines[5].contains("--remove-label"));

}

/// End-to-end: poll → pick a task → move it → poll again
#[tokio::test]
async fn end_to_end_poll_move_poll() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = mock_config();

    // 1. List issues
    let tasks = bd::list_issues(&cfg).await.unwrap();
    assert_eq!(tasks.len(), 7);

    // 2. Pick the "Waiting Room" task and move it to Doing
    let waiting = tasks.iter().find(|t| t.lane == "Waiting Room").unwrap();
    assert_eq!(waiting.id, "mc-b1j.6");

    bd::set_lane(&cfg, &waiting.id, "Doing")
        .await
        .expect("move to Doing should succeed");

    // 3. Poll again (stateless mock returns same data, but verifies no errors)
    let tasks2 = bd::list_issues(&cfg).await.unwrap();
    assert_eq!(tasks2.len(), 7);
}
