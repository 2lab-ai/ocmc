# Mission Control — Technical Specification

> **bd id:** `clawd-ml1`  
> **Status:** Draft v2.2  
> **Date:** 2026-02-19  
> **Related:** [PRD.md](./PRD.md) · [DECISIONS.md](./DECISIONS.md)

---

## 1. System Overview

Mission Control (MC) is a **Rust (axum) web application** that serves a local-first operations dashboard and control plane.

It integrates with:

- **bd CLI**: task state is SSOT (read + mutate via subprocess)
- **SQLite**: persistence for users, agent hierarchy metadata, projects, prompt metadata, audit logs
- **Gateway WS RPC**: cron state/control (may start as stub)

PRD v2.2 requires MC to encode governance:

- **Agent hierarchy**: `CEO → main → pdpm* → dev*/qa*`
- **Routing guardrails**: prohibit `main → dev/qa` direct control without CEO override
- **Default control scope**: CEO direct control defaults to **root=`main` only**
- **CEO break-glass override**: explicit, time-bounded, reason-required, audited
- **Project intake**: create bd epic skeleton + persist project metadata + link pdpm
- **Prompt lifecycle visibility**: PD/PM designs teams and generates/maintains system prompts; MC must store/display **prompt metadata**

```
┌──────────────┐     HTTP/WS      ┌──────────────┐
│   Browser     │ ◄──────────────► │  MC Server    │
│  (static JS)  │                  │  (axum)       │
└──────────────┘                  └──────┬───────┘
                                         │
                          ┌──────────────┼──────────────┐
                          │              │              │
                    ┌─────▼─────┐  ┌─────▼─────┐  ┌────▼──────┐
                    │  bd CLI    │  │  SQLite    │  │ Gateway   │
                    │ (tasks)    │  │ (users,    │  │ WS RPC    │
                    │            │  │ agents,    │  │ (cron)    │
                    │            │  │ projects,  │  │ [stub ok] │
                    │            │  │ prompts,   │  └───────────┘
                    │            │  │ audit)     │
                    └───────────┘  └───────────┘
```

### Decision references

- **ADR-001**: Rust + axum — [DECISIONS.md](./DECISIONS.md#adr-001)
- **ADR-002**: bd integration via subprocess — [DECISIONS.md](./DECISIONS.md#adr-002)
- **ADR-014**: Stateless signed session cookies — [DECISIONS.md](./DECISIONS.md#adr-014)
- **ADR-015**: Hierarchy + routing guardrails + CEO override — [DECISIONS.md](./DECISIONS.md#adr-015)
- **ADR-016**: Project intake — [DECISIONS.md](./DECISIONS.md#adr-016)
- **ADR-017**: Prompt artifacts + PD/PM prompt lifecycle — [DECISIONS.md](./DECISIONS.md#adr-017)

---

## 2. Technology stack

| Layer | Choice | Notes |
|---|---|---|
| Language | Rust (edition 2024) | Single binary, async runtime |
| Web framework | axum 0.7 | REST + WebSocket |
| DB | SQLite via sqlx 0.8 | Embedded persistence |
| Auth (human) | Signed session cookie | HttpOnly + SameSite=Lax (ADR-014) |
| Auth (agent, planned) | Token header | For automation endpoints |
| Frontend | Vanilla JS + static HTML/CSS | No build step |
| Task SSOT | bd CLI (subprocess) | `bd list --json` contract |
| Cron | Gateway WS RPC | May remain stub until protocol is stable |

---

## 3. Data model

### 3.1 SQLite schema (persisted)

#### 3.1.1 Users

```sql
CREATE TABLE users (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  username      TEXT NOT NULL UNIQUE,
  pass_hash     TEXT NOT NULL,
  created_at    TEXT NOT NULL
);
```

#### 3.1.2 Agents (hierarchy-aware)

```sql
CREATE TABLE agents (
  id            TEXT PRIMARY KEY,            -- e.g. "main", "pdpm-mc", "dev-a"
  display_name  TEXT NOT NULL,
  role          TEXT NOT NULL,               -- root|pdpm|dev|qa|observer (extendable)
  parent_id     TEXT NULL REFERENCES agents(id),
  project_id    TEXT NULL REFERENCES projects(id),
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

CREATE INDEX idx_agents_parent_id  ON agents(parent_id);
CREATE INDEX idx_agents_project_id ON agents(project_id);
```

Constraints (enforced in application code):
- Exactly one root agent id (config `MC_ROOT_AGENT_ID`, default `main`)
- No cycles in parent pointers

#### 3.1.3 Projects

Projects are a **control-plane grouping** (filters + intake). Task SSOT remains in bd.

```sql
CREATE TABLE projects (
  id              TEXT PRIMARY KEY,          -- slug or uuid
  name            TEXT NOT NULL,
  brief           TEXT NOT NULL,
  bd_root_issue   TEXT NOT NULL,             -- bd epic/root issue id
  pdpm_agent_id   TEXT NULL REFERENCES agents(id),
  status          TEXT NOT NULL,             -- active|archived
  created_at      TEXT NOT NULL,
  archived_at     TEXT NULL
);

CREATE INDEX idx_projects_status ON projects(status);
```

#### 3.1.4 Prompt metadata (system prompt lifecycle visibility)

MC must be able to store/display *prompt metadata* per agent, per project.

MVP schema (single "current" record per agent prompt kind):

```sql
CREATE TABLE agent_prompt_meta (
  agent_id         TEXT NOT NULL REFERENCES agents(id),
  project_id       TEXT NULL REFERENCES projects(id),
  kind             TEXT NOT NULL,            -- system
  version          INTEGER NOT NULL,
  content_sha256   TEXT NOT NULL,
  updated_at       TEXT NOT NULL,
  author_kind      TEXT NOT NULL,            -- user|agent
  author_id        TEXT NOT NULL,            -- username OR agent_id
  location         TEXT NULL,                -- optional pointer (bd comment link, file path)
  PRIMARY KEY (agent_id, kind)
);

CREATE INDEX idx_prompt_project ON agent_prompt_meta(project_id);
```

Notes / guardrails:
- Prefer storing **hash + metadata** by default. Full prompt text storage is optional and must be explicitly enabled.
- If/when full content is stored, it must be treated as sensitive (may contain tokens). Default UI should not reveal full content without explicit action.

(If version history is desired later, add `agent_prompt_revisions` table and point `agent_prompt_meta` to the head revision.)

#### 3.1.5 Audit events (allow/deny + override-aware)

```sql
CREATE TABLE audit_events (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  at              TEXT NOT NULL,
  actor_kind      TEXT NOT NULL,             -- user|agent
  actor_id        TEXT NOT NULL,             -- username OR agent_id
  decision        TEXT NOT NULL,             -- allow|deny
  action          TEXT NOT NULL,             -- e.g. task.assign, agent.reparent, project.create
  payload_json    TEXT NOT NULL,
  override        INTEGER NOT NULL DEFAULT 0,
  override_reason TEXT NULL
);

CREATE INDEX idx_audit_at     ON audit_events(at);
CREATE INDEX idx_audit_actor  ON audit_events(actor_kind, actor_id);
CREATE INDEX idx_audit_action ON audit_events(action);
```

#### 3.1.6 Override sessions (break-glass)

```sql
CREATE TABLE override_sessions (
  id          TEXT PRIMARY KEY,              -- uuid
  username    TEXT NOT NULL,
  reason      TEXT NOT NULL,
  enabled_at  TEXT NOT NULL,
  expires_at  TEXT NOT NULL,
  revoked_at  TEXT NULL
);

CREATE INDEX idx_override_exp  ON override_sessions(expires_at);
CREATE INDEX idx_override_user ON override_sessions(username);
```

#### 3.1.7 Agent auth (planned)

```sql
CREATE TABLE agent_tokens (
  agent_id     TEXT PRIMARY KEY REFERENCES agents(id),
  token_hash   TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  last_used_at TEXT NULL
);
```

---

### 3.2 In-memory cache

```rust
pub struct CacheState {
    pub last_snapshot_at: Option<DateTime<Utc>>,
    pub snapshot: Option<KanbanSnapshot>,
}

pub struct KanbanSnapshot {
    pub generated_at: DateTime<Utc>,
    pub agents: Vec<Agent>,
    pub projects: Vec<Project>,
    pub tasks: Vec<TaskCard>,
    pub cron: Vec<CronCard>,
    pub ui_policy: UiPolicy,
}

pub struct UiPolicy {
    pub root_agent_id: String,          // default: "main"
    pub default_view_scope: String,     // root_only
    pub default_control_scope: String,  // root_only
    pub override_ttl_s: u64,            // default: 600
}
```

---

## 4. Guardrails & authorization

### 4.1 Actor identity

MC differentiates:

- **Human users**: authenticated via signed session cookie
- **Agents** (planned): authenticated via token header

Request context yields:

```rust
enum ActorKind { User, Agent }
struct Actor { kind: ActorKind, id: String }
```

### 4.2 Central policy engine

Every mutating endpoint must go through a central policy function:

- `policy::authorize(actor, action, target, override_ctx)`

Rules (PRD §3 / ADR-015):
- Default CEO/user direct control: **root-only** surface (root=`main`)
- Without override: deny `main → dev/qa` direct control/dispatch
- Allow `pdpm → its child dev/qa` control

### 4.3 Audit logging contract

For any mutation attempt:

- On **deny**: respond 403 (or 409 as appropriate) and write audit row `decision="deny"`.
- On **allow**: perform action and write audit row `decision="allow"`.
- If override was used: set `override=1` + persist `override_reason`.

---

## 5. API surface

### 5.1 REST endpoints (MVP + planned)

| Method | Path | Auth | Notes |
|---|---|---|---|
| GET | `/healthz` | No | returns `ok` |
| GET/POST | `/login` | No | human login |
| POST | `/logout` | Yes | clear cookie |
| GET | `/api/kanban` | Yes | snapshot: agents+projects+tasks+cron+policy |
| GET | `/api/agents` | Yes | list agents |
| POST | `/api/agents` | Yes | create/upsert agent (guarded + audited) |
| POST | `/api/agents/:id/reparent` | Yes | hierarchy change (guarded + audited) |
| GET | `/api/projects` | Yes | list projects |
| POST | `/api/projects` | Yes | project intake (guarded + audited; uses bd) |
| POST | `/api/projects/:id/archive` | Yes | archive project |
| GET | `/api/audit` | Yes | query audit events |
| POST | `/api/override/enable` | Yes | enable break-glass override (reason required) |
| POST | `/api/override/disable` | Yes | revoke override |
| POST | `/api/task/:id/move` | Yes | move lane (guarded + audited) |
| POST | `/api/task/:id/assign` | Yes | assign agent (guarded + audited) |
| POST | `/api/cron/:id/toggle` | Yes | cron toggle (guarded + audited) |
| POST | `/api/cron/:id/run` | Yes | cron run-now (guarded + audited) |
| GET | `/api/agents/:id/prompt_meta` | Yes | prompt metadata (guarded if needed) |
| POST | `/api/agents/:id/prompt_meta` | Yes | update prompt metadata (guarded + audited) |

Optional / future:
- `POST /api/dispatch` — instruction dispatch router (Actor=agent supported). Must use the same policy engine.

### 5.2 WebSocket

Push-only refresh notifications:

- `/ws`: server → client `{"type":"refresh","at":"...","reason":"poll"}`

Client re-fetches `/api/kanban` on refresh.

---

## 6. Frontend requirements (v2.2)

### 6.1 Views & toggles

- View scope toggle: **root-only vs all** (presentation only)
- Control scope: default root-only; CEO override enables global control

### 6.2 Agent hierarchy + prompt metadata

- Render agent hierarchy tree (`main` → `pdpm*` → `dev*/qa*`).
- Agent tiles should display at minimum:
  - role + parent
  - project association (if any)
  - **prompt metadata** (version + updated_at; optionally content hash)

### 6.3 Guardrail UX

- When actions are disabled due to policy, show a clear explanation (tooltip/text).

---

## 7. Configuration

| Env var | Default | Description |
|---|---|---|
| `MC_SQLITE_URL` | `sqlite:///data/mc.db` | DB path |
| `MC_BIND_HOST` | `0.0.0.0` | bind host |
| `MC_BIND_PORT` | `3000` | bind port |
| `MC_POLL_MS` | `5000` | poll interval |
| `MC_BD_BIN` | `/hostbin/bd` | bd binary |
| `MC_ROOT_AGENT_ID` | `main` | root of hierarchy |
| `MC_OVERRIDE_TTL_S` | `600` | override validity |
| `MC_GATEWAY_URL` | `ws://127.0.0.1:18789` | gateway ws |
| `MC_ADMIN_USER` | `admin` | bootstrap user |
| `MC_ADMIN_PASS` | (required) | bootstrap password |
| `MC_PROMPT_STORE_CONTENT` | `0` | if 1, allow storing full prompt text (discouraged by default) |

---

## 8. Security considerations

| Area | Approach |
|---|---|
| Human sessions | Signed cookie token, HttpOnly, SameSite=Lax |
| Agent calls (planned) | Per-agent token header (hash in SQLite) |
| Guardrails | Central policy engine; deny + audit violations |
| bd subprocess | Arg-array exec, no shell |
| Prompt content | Prefer metadata-only; if storing content, treat as sensitive |

---

## 9. Testing strategy (planned)

Add coverage for:

- policy engine (authorize matrix)
- audit logging on allow/deny/override
- project intake: bd epic creation flow with mock bd binary
- prompt metadata endpoints (authorization + audit)
- UI E2E: root-only vs override behavior
