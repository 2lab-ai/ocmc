# Mission Control — Technical Specification

> **bd id:** `clawd-ml1`  
> **Status:** Draft v2.0  
> **Date:** 2026-02-19  
> **Related:** [PRD.md](./PRD.md) · [DECISIONS.md](./DECISIONS.md)

---

## 1. System Overview

Mission Control (MC) is a **Rust (axum) web application** that serves a local-first operations dashboard and control plane.

It integrates with:

- **bd CLI** (tasks are SSOT)
- **SQLite** (users, agent hierarchy metadata, audit)
- **Gateway RPC** (cron; may start as stub)

PRD v2 introduces an explicit **agent hierarchy and routing policy** (`CEO → main → pdpm → dev`) that MC must encode as:

- **UI gating** (root-only direct control by default)
- **API guardrails** (deny policy-violating mutations + audit)
- **CEO override** (break-glass mode; explicit + auditable)

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
                    │            │  │  audit)    │  │         │
                    └───────────┘  └───────────┘  └─────────┘
```

### Decision References

- **ADR-001**: Rust + axum — [DECISIONS.md](./DECISIONS.md#adr-001)
- **ADR-002**: bd integration via subprocess — [DECISIONS.md](./DECISIONS.md#adr-002)
- **ADR-003**: Cookie-based auth direction — [DECISIONS.md](./DECISIONS.md#adr-003)
- **ADR-015**: Agent hierarchy + routing guardrails — [DECISIONS.md](./DECISIONS.md#adr-015)

---

## 2. Technology Stack

| Layer | Choice | Notes |
|---|---|---|
| Language | Rust (edition 2024) | Single binary, async runtime |
| Web framework | axum 0.7 | REST + WebSocket |
| DB | SQLite via sqlx 0.8 | Embedded persistence |
| Auth (human) | Cookie session | Signed token cookie (HttpOnly, SameSite=Lax) |
| Auth (agent) | Token header (planned) | For automation/routing endpoints |
| Frontend | Vanilla JS + static HTML/CSS | No build step |
| Task SSOT | bd CLI (subprocess) | `bd list --json` contract |
| Cron | Gateway WS RPC | May be stubbed until protocol is stable |

---

## 3. Data Model

### 3.1 SQLite Schema (persisted)

#### Users

```sql
CREATE TABLE users (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  username      TEXT NOT NULL UNIQUE,
  pass_hash     TEXT NOT NULL,
  created_at    TEXT NOT NULL
);
```

#### Agents (hierarchy-aware)

Agents are registered with **role** and **hierarchy relationships**.

```sql
CREATE TABLE agents (
  id            TEXT PRIMARY KEY,            -- e.g. "main", "pdpm", "opus46"
  display_name  TEXT NOT NULL,
  role          TEXT NOT NULL,               -- enum-like: root|pdpm|dev|qa|observer
  parent_id     TEXT NULL REFERENCES agents(id),
  created_at    TEXT NOT NULL
);

CREATE INDEX idx_agents_parent_id ON agents(parent_id);
```

Notes:
- `role` + `parent_id` encode the tree: `main (root)` → `pdpm` → `dev*`.
- `parent_id` is NULL for the root agent.

#### Audit Events (actor + override-aware)

To support routing policy, audit must record *who* attempted an action, *what* was attempted, and whether it was an override or a denial.

```sql
CREATE TABLE audit_events (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  at              TEXT NOT NULL,
  actor_kind      TEXT NOT NULL,             -- user|agent
  actor_id        TEXT NOT NULL,             -- username OR agent_id
  action          TEXT NOT NULL,             -- e.g. task.assign, policy.deny, cron.toggle
  payload_json    TEXT NOT NULL,
  override        INTEGER NOT NULL DEFAULT 0,
  override_reason TEXT NULL
);

CREATE INDEX idx_audit_at ON audit_events(at);
CREATE INDEX idx_audit_actor ON audit_events(actor_kind, actor_id);
```

> If the current implementation still uses `username`-only audit rows, migrations should extend the schema and keep backward compatibility by writing `actor_kind="user"` and `actor_id=username`.

#### Agent auth (planned)

For agent-to-MC calls (automation endpoints), MC should support per-agent tokens.

Minimal approach:

```sql
CREATE TABLE agent_tokens (
  agent_id     TEXT PRIMARY KEY REFERENCES agents(id),
  token_hash   TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  last_used_at TEXT NULL
);
```

### 3.2 In-Memory Cache

```rust
pub struct CacheState {
    pub last_snapshot_at: Option<DateTime<Utc>>,
    pub snapshot: Option<KanbanSnapshot>,
}

pub struct KanbanSnapshot {
    pub generated_at: DateTime<Utc>,
    pub agents: Vec<Agent>,
    pub tasks: Vec<TaskCard>,
    pub cron: Vec<CronCard>,
    pub ui_policy: UiPolicy,   // new in v2
}

pub struct UiPolicy {
    pub root_agent_id: String,         // default: "main"
    pub default_control_mode: String,  // root_only
    pub can_override: bool,
    pub override_ttl_s: u64,
}
```

### 3.3 Domain Types

| Type | Source | Key Fields |
|---|---|---|
| `TaskCard` | `bd list --json` | id, title, priority, status, labels, assignee, lane, updated_at |
| `Agent` | SQLite `agents` + derived state | id, display_name, role, parent_id, state, current_card_id |
| `CronCard` | Gateway RPC | id, name, enabled, schedule, next_run_at_ms |

Lane mapping for unknown/uncategorized tasks must fall back to **Waiting Room** (PRD v2 + ADR-009).

---

## 4. API Surface

### 4.1 Actor identity

MC differentiates:

- **Human users**: authenticated via session cookie
- **Agents** (planned): authenticated via header token

Request context should produce an `Actor`:

```rust
enum ActorKind { User, Agent }
struct Actor { kind: ActorKind, id: String }
```

### 4.2 Guardrails (routing policy enforcement)

For every mutating endpoint, MC must run:

- `policy::authorize(actor, action, target)`
- If denied: respond `403` (or `409`) and write `audit_events(action="policy.deny", payload=...)`
- If allowed with CEO override: write `audit_events(override=1, override_reason=...)`

Policy rules (PRD v2):
- `main` cannot directly control/dispatch to `dev*` targets.
- `pdpm` can control/dispatch to its `dev*` children.
- CEO can override (explicit flag + reason).

### 4.3 REST Endpoints

| Method | Path | Auth | Notes |
|---|---|---|---|
| GET | `/healthz` | No | returns `ok` |
| GET/POST | `/login` | No | session cookie |
| POST | `/logout` | Yes | clear cookie |
| GET | `/api/kanban` | Yes | returns `KanbanSnapshot` (incl. `ui_policy`) |
| GET | `/api/agents` | Yes | list agents + roles + parent_id |
| POST | `/api/agents` | Yes | create/update agent record (guarded + audited) |
| POST | `/api/agents/:id/reparent` | Yes | hierarchy change (guarded + audited) |
| POST | `/api/task/:id/move` | Yes | guarded by policy + UI mode |
| POST | `/api/task/:id/assign` | Yes | guarded by policy + UI mode |
| GET | `/api/audit` | Yes | query audit events (filters by actor/action/time) |
| POST | `/api/cron/:id/toggle` | Yes | guarded + audited |
| POST | `/api/cron/:id/run` | Yes | guarded + audited |

> Future (optional): `/api/dispatch` for instruction routing (Actor=Agent supported). If/when added, it must be guarded by the same policy engine.

### 4.4 WebSocket

Push-only refresh notifications:

- `/ws`: server → client `{"type":"refresh","at":"...","reason":"poll"}`

Client re-fetches `/api/kanban` on refresh.

---

## 5. Module Architecture

```
src/
├── main.rs
└── mc/
    ├── mod.rs
    ├── auth.rs            # human auth (cookie)
    ├── agent_auth.rs      # (planned) agent token auth
    ├── policy.rs          # routing policy engine + guardrails
    ├── auth_http.rs
    ├── bd.rs
    ├── cron.rs
    ├── db.rs
    ├── handlers.rs
    ├── poller.rs
    └── ws.rs
```

---

## 6. Key Flows

### 6.1 Poll tick

Same as v1: bd list + cron list + agents list → build snapshot → broadcast refresh.

### 6.2 Mutations with policy enforcement

Example: task assignment

1) Browser calls `POST /api/task/:id/assign {agent:"opus46", override:false}`
2) Server extracts `Actor` (user/agent)
3) Server calls `policy::authorize(actor, Action::TaskAssign, target_agent)`
4) If denied:
   - return 403
   - audit `policy.deny`
5) If allowed:
   - call `bd assign`
   - audit `task.assign` (and override fields if override)

---

## 7. Configuration

| Env Var | Default | Description |
|---|---|---|
| `MC_SQLITE_URL` | `sqlite:///data/mc.db` | DB path |
| `MC_BIND_HOST` | `0.0.0.0` | bind host |
| `MC_BIND_PORT` | `3000` | bind port |
| `MC_POLL_MS` | `5000` | poll interval |
| `MC_BD_BIN` | `/hostbin/bd` | bd binary |
| `MC_ROOT_AGENT_ID` | `main` | root of hierarchy |
| `MC_OVERRIDE_TTL_S` | `600` | override validity window |
| `MC_GATEWAY_URL` | `ws://127.0.0.1:18789` | gateway ws |
| `MC_ADMIN_USER` | `admin` | bootstrap user |
| `MC_ADMIN_PASS` | (required) | bootstrap password |

---

## 8. Frontend Architecture (v2 additions)

In addition to v1 rendering:

- Render **agent hierarchy tree** (role + parent relationships)
- Add UI toggles:
  - **View**: root-only vs all
  - **Control**: CEO override (requires reason; time-bounded)
- Render disabled controls with explanatory tooltip when guardrails apply

---

## 9. Security Considerations

| Area | Approach |
|---|---|
| Human sessions | Signed cookie token, HttpOnly, SameSite=Lax |
| Agent calls (planned) | Per-agent token header (hash stored in SQLite) |
| Guardrails | Central policy engine; deny + audit violations |
| bd subprocess | Arg-array exec, no shell |

Known limitations (until implemented):
- Without agent tokens, agent-to-MC automation endpoints must be disabled or admin-only.

---

## 10. Deployment

v1 Docker strategy remains valid (bind-mount bd + .beads; volume for SQLite).

---

## 11. Testing Strategy

Add coverage for:
- policy engine (authorize matrix)
- audit logging on allow/deny/override
- UI: root-only vs override behavior (E2E)
