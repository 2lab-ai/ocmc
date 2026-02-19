# Mission Control — Product Requirements Document

> **bd id:** `clawd-ml1`
> **Status:** Draft v1.0
> **Author:** CEO (materialized by agent)
> **Date:** 2026-02-19
> **Related:** [SPEC.md](./SPEC.md) · [DECISIONS.md](./DECISIONS.md)

---

## 1. Vision

Mission Control (MC) is the **CEO-facing operations dashboard** for the clawd AI agent fleet. It provides real-time visibility into what every agent is doing, what tasks are queued, and what cron jobs are scheduled — all in a single browser tab.

The core insight: as the number of AI agents and automated workflows grows, the human operator needs a **single pane of glass** to observe, steer, and audit agent behavior without SSHing into machines or reading raw CLI output.

---

## 2. Problem Statement

| Pain Point | Current Workaround |
|---|---|
| No visibility into agent task queues | Run `bd list` manually in terminal |
| Can't see which agent is doing what | Grep subagent logs or ask the agent |
| Cron jobs are invisible | Read cron YAML files, hope for the best |
| No audit trail for task movements | Trust that agents report correctly |
| Context-switching between terminal, chat, browser | Accept cognitive overhead |

---

## 3. Target Users

1. **CEO / Human Operator** (primary) — Needs at-a-glance situational awareness. Must be able to intervene (reassign, block, prioritize) without touching CLI.
2. **Agents** (secondary, read-only consumers) — May query their own task state via API during orchestration loops.

---

## 4. Core Requirements

### 4.1 Kanban Board (P0)

- Display all `bd` issues as cards organized into swim lanes: **Backlog → Ready → Doing → Blocked → Done** (plus **Waiting Room** for uncategorized).
- Each card shows: id, title, priority badge, assignee, labels, last-updated timestamp.
- Cards are **draggable** between lanes — drag triggers a `bd` status update under the hood.
- Assignee can be changed via card action menu → calls `bd assign`.

### 4.2 Agent Fleet Status (P0)

- Show each registered agent as a status tile: name, current state (doing/waiting), currently assigned card.
- Auto-refresh via WebSocket push — no manual polling from the browser.

### 4.3 Cron Dashboard (P1)

- Display cron jobs as cards in Scheduled / Disabled lanes.
- Toggle enable/disable per job.
- "Run Now" button for manual trigger.
- Integration target: OpenClaw Gateway RPC (not CLI, because CLI wiring may not exist in all deployments).

### 4.4 Authentication (P0)

- Cookie-based session auth with argon2-hashed passwords.
- Bootstrap admin user from environment variables (`MC_ADMIN_USER`, `MC_ADMIN_PASS`).
- Login page, logout action. No OAuth/SSO in MVP.

### 4.5 Audit Trail (P1)

- Every user action (task move, task assign, cron toggle, cron run) logged to `audit_events` table.
- Fields: timestamp, username, action, payload JSON.
- Queryable via future API endpoint (not in MVP UI).

### 4.6 Real-time Updates (P0)

- WebSocket endpoint (`/ws`) pushes refresh events to all connected clients.
- Backend poller ticks every N ms (configurable via `MC_POLL_MS`, default 5000), fetches `bd list --json`, and broadcasts diffs.

---

## 5. Non-Requirements (Explicit Exclusions)

- **Agent-to-agent communication** — MC is observation + control, not a message bus.
- **Code deployment** — MC doesn't deploy or build anything.
- **Multi-tenancy** — Single-operator deployment only.
- **Mobile-first** — Desktop browser is the target. Mobile is acceptable but not optimized.

---

## 6. bd Integration Rules

Mission Control is tightly coupled to `bd` (the task tracker CLI). The following rules govern this integration:

1. **bd is the single source of truth** for task state. MC reads from bd and writes back to bd. MC's SQLite stores only users, agents, and audit logs — never task data.
2. **All task mutations flow through bd CLI** — `bd update`, `bd assign`, `bd label`. MC never writes task state directly to any bd storage.
3. **bd binary path** is configurable via `MC_BD_BIN` env var (default: `/hostbin/bd`). This allows MC to run in Docker with the host bd binary bind-mounted.
4. **Polling frequency** is configurable via `MC_POLL_MS`. Default 5000ms. Lower values increase bd CLI call volume.
5. **bd JSON output** (`bd list --json`) is the contract format. If bd output schema changes, MC's `bd.rs` deserializer must be updated.

---

## 7. Git Workflow Rules

All Mission Control development follows the clawd repository git workflow:

1. **Single branch**: `master` (the ocmc monorepo main branch). No feature branches for now.
2. **Commit messages** must include the relevant `bd` issue id (e.g., `clawd-ml1`).
3. **Push is mandatory** — work is not done until `git push` succeeds (see AGENTS.md "Landing the Plane").
4. **No production code in docs commits** — documentation changes must be pure docs (this constraint exists for this initial materialization).
5. **Subagent commits** should be 1–2 meaningful commits, not a trail of micro-fixes.

---

## 8. Decision Logging Requirements

All significant architectural and product decisions related to Mission Control must be recorded in [DECISIONS.md](./DECISIONS.md) using the ADR (Architecture Decision Record) format:

1. **When to log**: Any choice that (a) has alternatives worth noting, (b) is hard to reverse, or (c) will confuse future readers if unexplained.
2. **Format**: Sequential numbering, status (Accepted/Superseded/Deprecated), context, decision, consequences.
3. **Cross-reference**: PRD sections and SPEC sections should reference decision IDs where applicable.
4. **Who logs**: The agent performing the work. CEO may also add decisions directly.

---

## 9. Success Metrics

| Metric | Target |
|---|---|
| Time from bd update → UI reflects change | < 10 seconds |
| Page load to interactive | < 2 seconds |
| Task move (drag) → bd updated | < 3 seconds |
| Concurrent connected clients | ≥ 5 without degradation |

---

## 10. Future Roadmap (Post-MVP)

- **Agent log streaming** — Tail agent subagent logs in MC UI.
- **Cron Gateway RPC** — Replace stub module with real OpenClaw Gateway WebSocket RPC.
- **Timeline view** — Gantt-like view of task history.
- **Notifications** — Push alerts when agents are stuck or tasks are blocked too long.
- **Multi-user roles** — Read-only viewer vs admin operator.
