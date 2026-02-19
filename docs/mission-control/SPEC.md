# Mission Control — Technical Specification

> **bd id:** `clawd-ml1`
> **Status:** Draft v1.0
> **Date:** 2026-02-19
> **Related:** [PRD.md](./PRD.md) · [DECISIONS.md](./DECISIONS.md)

---

## 1. System Overview

Mission Control is a **Rust (axum) web application** that serves a real-time operations dashboard. It polls external data sources (primarily `bd` CLI), caches snapshots in memory, persists user/agent/audit data in SQLite, and pushes updates to connected browser clients via WebSocket.

```
┌──────────────┐     HTTP/WS      ┌──────────────┐
│   Browser     │ ◄──────────────► │  MC Server    │
│  (static JS)  │                  │  (axum)       │
└──────────────┘                  └──────┬───────┘
                                         │
                          ┌──────────────┼──────────────┐
                          │              │              │
                    ┌─────▼─────┐  ┌─────▼─────┐  ┌────▼────┐
                    │  bd CLI    │  │  SQLite    │  │ Gateway │
                    │ (issues)   │  │ (users,    │  │ RPC     │
                    │            │  │  agents,   │  │ (cron)  │
                    │            │  │  audit)    │  │ [stub]  │
                    └───────────┘  └───────────┘  └─────────┘
```

### Decision References
- **ADR-001**: Rust + axum chosen over Node/Python — see [DECISIONS.md](./DECISIONS.md#adr-001).
- **ADR-002**: bd CLI integration via subprocess — see [DECISIONS.md](./DECISIONS.md#adr-002).
- **ADR-003**: Cookie auth over JWT — see [DECISIONS.md](./DECISIONS.md#adr-003).

---

## 2. Technology Stack

| Layer | Choice | Notes |
|---|---|---|
| Language | Rust (edition 2024) | Performance, safety, single binary |
| Web framework | axum 0.7 | WebSocket + REST, tower middleware |
| Database | SQLite via sqlx 0.8 | Embedded, zero-ops, sufficient for single-operator |
| Auth | argon2 password hashing | Cookie-based sessions (`mc_session`) |
| Real-time | WebSocket (native axum) | Push-only from server to client |
| Frontend | Vanilla JS + static HTML/CSS | No build step, served via `tower-http::ServeDir` |
| Task tracker | bd CLI (subprocess) | JSON output parsed into `TaskCard` structs |

---

## 3. Data Model

### 3.1 SQLite Schema (persisted)

```sql
CREATE TABLE users (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  username      TEXT NOT NULL UNIQUE,
  pass_hash     TEXT NOT NULL,          -- argon2 hash
  created_at    TEXT NOT NULL            -- ISO 8601
);

CREATE TABLE agents (
  id            TEXT PRIMARY KEY,        -- e.g. "opus46", "sonnet-a"
  display_name  TEXT NOT NULL,
  created_at    TEXT NOT NULL
);

CREATE TABLE audit_events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  at            TEXT NOT NULL,           -- ISO 8601
  username      TEXT NOT NULL,           -- who performed the action
  action        TEXT NOT NULL,           -- e.g. "task.move", "cron.toggle"
  payload_json  TEXT NOT NULL            -- action-specific details
);
```

### 3.2 In-Memory Cache (`CacheState`)

```rust
pub struct CacheState {
    pub last_snapshot_at: Option<DateTime<Utc>>,
    pub snapshot: Option<KanbanSnapshot>,
}
```

The `KanbanSnapshot` is rebuilt every poll tick and broadcast to WebSocket clients:

```rust
pub struct KanbanSnapshot {
    pub generated_at: DateTime<Utc>,
    pub agents: Vec<Agent>,
    pub tasks: Vec<TaskCard>,
    pub cron: Vec<CronCard>,
}
```

### 3.3 Domain Types

| Type | Source | Key Fields |
|---|---|---|
| `TaskCard` | `bd list --json` | id, title, priority, status, labels, assignee, lane, updated_at |
| `Agent` | SQLite `agents` table + derived state | id, display_name, state (doing/waiting), current_card_id |
| `CronCard` | Gateway RPC (stub) | id, name, enabled, schedule, next_run_at_ms, lane |

**Lane mapping** for tasks: `Backlog`, `Ready`, `Doing`, `Blocked`, `Done`, `Waiting Room`. The lane is derived from bd issue status field with a mapping function in `bd.rs`.

---

## 4. API Surface

### 4.1 REST Endpoints

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/healthz` | No | Health check, returns `"ok"` |
| GET | `/login` | No | Login page (HTML) |
| POST | `/login` | No | Authenticate, set `mc_session` cookie |
| POST | `/logout` | Yes | Clear session cookie |
| GET | `/api/kanban` | Yes | Return current `KanbanSnapshot` JSON |
| POST | `/api/task/:id/move` | Yes | Move task to new lane (`{lane: "Doing"}`) |
| POST | `/api/task/:id/assign` | Yes | Assign task to agent (`{agent: "opus46"}`) |
| POST | `/api/cron/:id/toggle` | Yes | Toggle cron job enabled/disabled |
| POST | `/api/cron/:id/run` | Yes | Trigger immediate cron job execution |

### 4.2 WebSocket

| Path | Direction | Payload |
|---|---|---|
| `/ws` | Server → Client | `{"type":"refresh","at":"...","reason":"poll"}` |

The WebSocket is **push-only**. The server broadcasts a refresh event after each successful poller tick. The client, upon receiving this event, calls `GET /api/kanban` to fetch the latest snapshot.

---

## 5. Module Architecture

```
src/
├── main.rs              # Server bootstrap, router, poller spawn
└── mc/
    ├── mod.rs           # Config, domain types, enums
    ├── auth.rs          # AuthedUser extractor (cookie-based)
    ├── auth_http.rs     # Login/logout HTTP handlers
    ├── bd.rs            # bd CLI subprocess integration
    ├── cron.rs          # Cron job integration (stub for Gateway RPC)
    ├── db.rs            # SQLite migrations, queries, admin bootstrap
    ├── handlers.rs      # REST API handlers (kanban, task, cron)
    ├── poller.rs        # Background poll loop (bd + cron → cache)
    └── ws.rs            # WebSocket handler (broadcast events)
```

### 5.1 Key Flows

#### Poll Tick (every MC_POLL_MS)
```
poller::tick()
  → bd::list_issues(cfg)       # spawns `bd list --json`, parses output
  → cron::list_jobs(cfg)       # stub, returns empty vec
  → db::list_agents(pool)      # SELECT from agents table
  → derives Agent states from current task assignments
  → builds KanbanSnapshot
  → writes to CacheState (RwLock)
  → events_tx.send(Refresh)    # broadcast channel → all WS clients
```

#### Task Move (user drags card)
```
Browser: POST /api/task/:id/move {lane: "Doing"}
  → handlers::task_move_post()
  → auth: extract AuthedUser from cookie
  → bd::set_lane(cfg, id, lane)   # spawns `bd update <id> --status <lane>`
  → db::audit(pool, user, "task.move", payload)
  → next poll tick picks up the change
```

---

## 6. Configuration

All configuration is via environment variables (see **ADR-001** for rationale):

| Env Var | Default | Description |
|---|---|---|
| `MC_SQLITE_URL` | `sqlite:///data/mc.db` | SQLite connection string |
| `MC_BIND_HOST` | `0.0.0.0` | Listen address |
| `MC_BIND_PORT` | `3000` | Listen port |
| `MC_POLL_MS` | `5000` | Poller interval in milliseconds |
| `MC_BD_BIN` | `/hostbin/bd` | Path to bd CLI binary |
| `MC_GATEWAY_URL` | `ws://127.0.0.1:18789` | OpenClaw Gateway WebSocket URL |
| `MC_GATEWAY_TOKEN` | (none) | Gateway auth token |
| `MC_GATEWAY_PASSWORD` | (none) | Gateway auth password |
| `MC_ADMIN_USER` | `admin` | Bootstrap admin username |
| `MC_ADMIN_PASS` | `change-me` | Bootstrap admin password |

---

## 7. bd Integration Detail

### 7.1 Read Path

```rust
// bd.rs — simplified
pub async fn list_issues(cfg: &McConfig) -> Result<Vec<TaskCard>> {
    let out = Command::new(&cfg.bd_bin)
        .args(["list", "--json", "--limit", "0"])
        .output().await?;
    let issues: Vec<BdIssue> = serde_json::from_slice(&out.stdout)?;
    Ok(issues.into_iter().map(to_task_card).collect())
}
```

The `BdIssue` struct maps the bd JSON output:
```rust
struct BdIssue {
    id: String,
    title: String,
    priority: Option<String>,
    status: String,
    labels: Vec<String>,
    assignee: Option<String>,
    updated_at: Option<DateTime<Utc>>,
}
```

### 7.2 Write Path

Task mutations call bd CLI subcommands:
- **Move**: `bd update <id> --status <lane>`
- **Assign**: `bd assign <id> <agent>`
- **Label**: `bd label add <id> <label>`

All writes are fire-and-forget from the API handler perspective — the next poll tick will pick up the change and push it to clients.

### 7.3 Contract

MC depends on `bd list --json` returning a JSON array of objects with the fields defined in `BdIssue`. **If bd output schema changes, `bd.rs` must be updated.** This is a hard coupling by design (see PRD §6).

---

## 8. Frontend Architecture

The frontend is intentionally minimal — vanilla JS, no framework, no build step.

```
static/
├── index.html       # Shell: header, agent tiles, kanban lanes, cron section
├── app.css          # Layout: flexbox kanban, agent tiles, card styles
└── app.js           # Fetch API, WebSocket client, DOM rendering
```

### 8.1 Rendering Flow

1. On page load, `app.js` calls `GET /api/kanban` and renders the snapshot.
2. A WebSocket connection is opened to `/ws`.
3. On each `refresh` event, `app.js` re-fetches `/api/kanban` and re-renders.
4. Drag-and-drop on cards triggers `POST /api/task/:id/move`.

### 8.2 Kanban Lanes

```javascript
const lanes = ["Backlog", "Ready", "Doing", "Blocked", "Done", "Waiting Room"];
```

Tasks are bucketed into lanes based on their `lane` field. Unknown lane values fall back to `Ready`.

---

## 9. Security Considerations

| Area | Approach |
|---|---|
| Password storage | argon2 with random salt per user |
| Session management | `mc_session=<username>` cookie, HttpOnly, SameSite=Lax |
| Auth enforcement | `AuthedUser` axum extractor on all `/api/*` routes |
| bd CLI execution | Subprocess with no shell interpolation (args passed as array) |
| CSRF | SameSite=Lax cookie provides basic protection; POST-only mutations |
| Network | Designed for LAN/VPN deployment, not public internet (see PRD §5) |

**Known limitations** (MVP):
- Session cookie stores username in plaintext (no signed token). Acceptable for single-operator LAN deployment.
- No rate limiting on login attempts.
- No HTTPS termination (expected to be behind reverse proxy).

---

## 10. Deployment

MC is designed to run as a Docker container or bare-metal binary:

```bash
# Bare metal
MC_ADMIN_PASS=hunter2 MC_BD_BIN=/usr/local/bin/bd cargo run

# Docker (bd binary bind-mounted from host)
docker run -d \
  -v /usr/local/bin/bd:/hostbin/bd:ro \
  -v mc-data:/data \
  -e MC_ADMIN_PASS=hunter2 \
  -p 3000:3000 \
  mission-control:latest
```

SQLite database is stored at the path specified by `MC_SQLITE_URL`. Migrations run automatically on startup.

---

## 11. Testing Strategy (Planned)

| Level | Approach |
|---|---|
| Unit | Rust `#[cfg(test)]` modules for bd parsing, lane mapping |
| Integration | Test against a mock bd binary that returns fixture JSON |
| E2E | Playwright or similar for login → kanban render → drag card flow |

**Current state**: No tests exist yet. This is tracked as a follow-up task.
