# Mission Control — Product Requirements Document (PRD)

> **bd id:** `clawd-ml1`  
> **Status:** Draft v2.2 (SSOT)  
> **Date:** 2026-02-19  
> **Related:** [SPEC.md](./SPEC.md) · [DECISIONS.md](./DECISIONS.md)

---

## 0. Executive Summary

Mission Control (MC) is the **CEO-facing operations dashboard + control plane** for the clawd agent fleet.

PRD v2.x adds a non-negotiable operating model:

- **Agent hierarchy:** `CEO (human) → main (root) → pdpm* (project managers) → dev*/qa* (workers)`
- **Instruction routing / governance:** default chain-of-command must be reflected in the UI and enforced by API guardrails.
- **CEO direct control defaults to root-only:** CEO can directly control **root = `main`** by default.
- **CEO override exists:** CEO can “break glass” to directly control any agent/task, but it must be explicit, time-bounded, and audited.
- **New project intake:** MC must support initiating a new project in a structured way (create project + assign a pdpm + create an initial bd epic/task skeleton).
- **Team & prompt design:** each project may require a different team. `pdpm*` must design the team and **generate/maintain each agent’s system prompt** as a first-class artifact.

This PRD is the **single source of truth** for product behavior. SPEC must follow.

Decision anchors:
- **ADR-015** (hierarchy + routing + break-glass)
- **ADR-016** (project intake)
- **ADR-017** (prompt artifacts + PD/PM prompt lifecycle)

See [DECISIONS.md](./DECISIONS.md).

---

## 1. Vision

As the number of AI agents and automated workflows grows, the operator needs a **single pane of glass** to:

1) **Observe** what each agent is doing (tasks + scheduling + health)
2) **Steer** work safely (assign / reprioritize / unblock)
3) **Govern** work (encode chain-of-command, prevent accidental misrouting, preserve auditability)

MC is not “just a kanban.” It is an **ops cockpit** that encodes organizational structure so we can scale the fleet without chaos.

---

## 2. Concepts & Operating Model — P0

### 2.1 Roles (conceptual)

- **CEO (human operator):** sets priorities, can override governance in emergencies.
- **`main` (root agent):** top-level coordinator; by default, **the only agent the CEO directly controls**.
- **`pdpm*` (manager agents):** one (or more) manager agents per project; they break down work and dispatch to workers.
- **`dev*` / `qa*` (worker agents):** implement, test, ship.

> `pdpm*` means there may be multiple pdpm agents concurrently (e.g. `pdpm-mc`, `pdpm-growth`, …). This is required to support parallel projects.

### 2.2 Agent hierarchy (tree/forest)

- The fleet forms a **directed forest** with a configured **root agent id** (default `main`).
- `main` is expected to be the parent of multiple `pdpm*` agents.
- Each `pdpm*` is expected to parent multiple workers (`dev*`, `qa*`).

MC must be able to **store and render** this hierarchy.

### 2.3 “Direct control” definition

Any action that **mutates** state is “direct control”, including:

- task assignment / reassignment
- task lane/status movement
- agent registration / hierarchy changes
- cron toggle/run
- project intake actions (project create/archive)
- instruction dispatch (if/when implemented)

Read-only observability is always allowed.

---

## 3. Instruction Routing Policy (normative) — P0

### 3.1 Default routing (encouraged by UX, enforced by API)

The default chain-of-command is:

- CEO → `main`
- `main` → `pdpm*`
- `pdpm*` → its child `dev*`/`qa*`

### 3.2 Prohibited routes (unless CEO override)

- `main` → `dev*` / `qa*` **direct control** (including “dispatch” style actions and/or task assignment to workers)

### 3.3 What “routing policy enforcement” means in MC

MC must implement guardrails such that:

- If an action is **prohibited**, MC denies it (e.g. HTTP 403) and **audit-logs the denial**.
- If an action is allowed only under **CEO Override**, MC requires an override session (time-bounded + reason) and **audit-logs the override**.

> Even when the CEO is the actor (human), MC still defaults to root-only to reduce accidental scope creep. The CEO can always override.

---

## 4. UX Requirements — Views, Control Surfaces, and Override — P0

### 4.1 Default UI mode: Root-only direct control

The dashboard defaults to **Root-only control**:

- System-wide read-only visibility is allowed.
- Mutating controls (drag/drop, assign, reparent, cron toggle/run, etc.) are **enabled only** when the target is within the **root control surface** (root=`main`).
- Targets outside root scope must render read-only controls (disabled state) with an explicit explanation.

### 4.2 View toggle: Root-only vs All (presentation)

The user may switch **view scope**:

- **Root-only view:** focus on root and its immediate surface (reduces noise).
- **All view:** full fleet view.

This is a *presentation* toggle; it must not bypass guardrails.

### 4.3 CEO Override (“break glass”)

Provide a break-glass UX:

- Toggle: **“CEO Override: enable direct control for all agents/tasks”**
- When enabling, require a **reason string** (stored in audit events)
- Override is **time-bounded** (default 10 minutes)
- Override status must be highly visible while active

---

## 5. Core Product Requirements

### 5.1 Kanban Board (P0)

- Display `bd` issues as cards in lanes: **Backlog → Ready → Doing → Blocked → Done** (+ **Waiting Room**).
- Each card shows: id, title, priority, assignee, labels, last-updated timestamp.
- Lane changes and assignments are **policy-gated**.

### 5.2 Agent Fleet Status + Team View (P0)

- Show each registered agent as a status tile:
  - id / display name
  - role (`root/main`, `pdpm`, `dev`, `qa`, …)
  - parent (for hierarchy)
  - current state (doing/waiting)
  - currently assigned card (if any)
- Provide a **hierarchy tree view** to visualize teams: `main` → `pdpm*` → `dev*/qa*`.

### 5.3 Audit trail (P0 for logging, P1 for rich UI)

Audit must include:

- every allowed mutation (task move/assign, cron actions, hierarchy changes, intake actions)
- every **denied** action (policy violations) with actor + attempted target
- every override action (`override=true`, `override_reason`, session id)

### 5.4 Authentication (P0)

- Human sessions required for dashboard access.
- Agent-to-MC authentication is planned for automation/routing endpoints (see SPEC).

### 5.5 Real-time updates (P0)

- WebSocket push-only refresh events
- Browser fetches the latest snapshot via REST

### 5.6 New Project Intake (P1)

MC must support a structured “start a new project” flow.

**Inputs (minimum):**
- project name (human-friendly)
- request/brief (free text)
- desired priority / urgency
- optional: choose existing pdpm or “create a new pdpm agent record”

**Outputs:**
- a `bd` **project epic** (root issue) + initial child skeleton tasks
- a project record in MC (for grouping/filters)
- an agent hierarchy update (attach pdpm under `main`; optionally attach team members)

#### 5.6.1 Team design + system prompt generation (required procedure)

Because projects require different teams, the intake flow must explicitly create a **prompt + team design loop**:

1) **`main` writes the project-specific `pdpm*` system prompt** (meta-cognition based), using the template in:
   - `docs/mission-control/PDPM_PROMPT_TEMPLATE.md`
2) The `pdpm*` produces a **Team Plan** (roles, milestones, risks, boundaries).
3) The `pdpm*` generates and maintains a **system prompt per child agent** (dev/qa/etc), aligned with:
   - chain-of-command rules
   - evidence protocol (bd comments)
   - merge-when-green policy

> MVP does not need to automatically spawn agent processes. It must make the procedure explicit and auditable.

#### 5.6.2 Prompt artifact metadata (P1)

MC must store and display **prompt metadata** (at minimum) so the CEO can audit who is responsible for what:

- project → pdpm id
- agent id → role → parent id
- prompt version / updated_at
- prompt content hash (e.g. sha256)
- prompt author (main vs pdpm)

(Full prompt text storage may be added later; see SPEC guardrails.)

### 5.7 Cron dashboard (P1)

- List cron jobs (Scheduled/Disabled)
- Toggle enable/disable
- Run-now
- Integration target: OpenClaw Gateway RPC

---

## 6. Non-Requirements (Explicit Exclusions)

- Agent-to-agent chat/messaging bus (MC is governance + ops, not a general chat system)
- Multi-tenancy
- Mobile-first UI
- Public-internet hardening beyond basic hygiene (this is local-first)

---

## 7. Integration Rules

### 7.1 bd integration

1) **bd is SSOT** for tasks. MC reads and writes via bd CLI.
2) MC’s SQLite persists: users, agents (hierarchy metadata), projects, audit.
3) bd binary path configurable via `MC_BD_BIN`.
4) Poll interval configurable via `MC_POLL_MS`.

### 7.2 Control-plane mutations

All state mutations are subject to:

- **authentication** (human session; agent token when applicable)
- **policy guardrails** (routing rules)
- **audit logging** (allow/deny/override)

---

## 8. Success Metrics

- bd update → UI reflects change: **< 10s**
- Page load to interactive: **< 2s**
- Task move/assign → bd updated: **< 3s**
- Policy violation detection: **100%** of prohibited routes denied and logged
- Override safety: **100%** of override mutations include reason + are time-bounded

---

## 9. Roadmap (indicative)

- **P0:** Observability + Kanban + agent registry + hierarchy view + policy gating + audit logging + root-only defaults
- **P1:** CEO override UX + audit query UI + cron controls + **project intake** + prompt metadata display
- **P2:** Instruction dispatch console (if needed) + agent health/stuck detection integration + notifications
