# Mission Control — Architecture Decision Records

> **bd id:** `clawd-ml1`
> **Related:** [PRD.md](./PRD.md) · [SPEC.md](./SPEC.md)

This document records significant architectural and product decisions for the Mission Control project. Each decision follows the ADR format: context, decision, consequences.

**When to add an entry:** Any choice that (a) has alternatives worth noting, (b) is hard to reverse, or (c) will confuse future readers if unexplained. See PRD §8.

---

## ADR-001: Rust + axum for the backend {#adr-001}

**Date:** 2026-02-15
**Status:** Accepted

### Context

Mission Control needs a web server that can:
- Serve a REST API and WebSocket connections
- Spawn bd CLI subprocesses efficiently
- Run a background poller on a timer
- Be deployed as a single binary with no runtime dependencies (except bd)

Candidates considered:
1. **Node.js (Express/Fastify)** — Fast to prototype, but runtime dependency, larger attack surface.
2. **Python (FastAPI)** — Familiar, but async subprocess handling is clunky; deployment requires Python runtime.
3. **Rust (axum)** — Single binary, excellent async runtime (tokio), strong typing, good WebSocket support.
4. **Go (net/http)** — Good alternative, but the team already has Rust toolchain in the workspace.

### Decision

Use **Rust with axum 0.7** and **tokio** async runtime. Configuration via environment variables (12-factor style).

### Consequences

- **Positive:** Single static binary deploys easily to Docker or bare metal. No runtime dependencies. Memory-safe concurrent polling.
- **Positive:** axum's extractor pattern makes auth middleware clean (`AuthedUser` extractor).
- **Negative:** Slower iteration speed than scripting languages. Frontend-only changes still require no rebuild (static files served from disk).
- **Negative:** Compile times are non-trivial (mitigated by incremental builds).

---

## ADR-002: bd CLI integration via subprocess, not library {#adr-002}

**Date:** 2026-02-15
**Status:** Accepted

### Context

MC needs to read and write task data from bd (the project task tracker). Two integration approaches:

1. **Library/crate import** — Import bd's data layer as a Rust crate. Direct database access.
2. **CLI subprocess** — Shell out to `bd list --json`, `bd update`, etc.

### Decision

Use **CLI subprocess integration**. MC spawns `bd` as a child process with args passed as arrays (no shell interpolation).

### Consequences

- **Positive:** Zero coupling to bd internals. bd can be upgraded independently. Works with any bd version that supports `--json` output.
- **Positive:** bd binary can be bind-mounted from the host into a Docker container, keeping MC containerized while using the host's bd config/database.
- **Negative:** Subprocess overhead per poll tick (~50-200ms per `bd list` call). Acceptable at 5s poll intervals.
- **Negative:** Error handling is string-based (parsing stderr). Less precise than library errors.
- **Negative:** MC is tightly coupled to bd's JSON output schema. Schema changes require updating `bd.rs`.

**Mitigation:** The `BdIssue` struct uses `#[serde(default)]` liberally to tolerate missing fields gracefully.

---

## ADR-003: Cookie-based session auth, not JWT {#adr-003}

**Date:** 2026-02-15
**Status:** Accepted

### Context

MC needs authentication to protect the dashboard and API. Options:

1. **JWT tokens** — Stateless, standard, but requires token refresh logic and client-side storage.
2. **Cookie-based sessions** — Simple, HttpOnly cookies prevent XSS token theft, well-understood.
3. **No auth** — MC is LAN-only, maybe just skip it? → Rejected: audit trail requires user identity.

### Decision

Use **cookie-based sessions** with `mc_session=<username>` cookies. Passwords hashed with argon2 + random salt.

### Consequences

- **Positive:** Dead simple implementation (~40 lines). HttpOnly + SameSite=Lax provides decent XSS/CSRF protection.
- **Positive:** No token refresh complexity. Session persists until cookie expires or user logs out.
- **Negative:** Cookie stores username in plaintext (not cryptographically signed). Acceptable for single-operator LAN deployment where the threat model is "prevent accidental unauthorized access," not "resist determined attackers."
- **Negative:** Not suitable for public internet deployment without a signed/encrypted session token.

**Future:** If MC is ever exposed publicly, replace with signed session tokens or OAuth2.

---

## ADR-004: SQLite for persistence, not Postgres {#adr-004}

**Date:** 2026-02-15
**Status:** Accepted

### Context

MC needs to persist users, agent registrations, and audit events. Task data lives in bd, not MC.

Options:
1. **PostgreSQL** — Robust, but requires running a separate database server.
2. **SQLite** — Embedded, zero-ops, single-file database.
3. **Flat files (JSON/YAML)** — Simplest, but no query capability, concurrency issues.

### Decision

Use **SQLite** via the `sqlx` crate with compile-time checked queries.

### Consequences

- **Positive:** Zero external dependencies. Database is a single file, easy to backup/restore.
- **Positive:** sqlx provides async SQLite access with connection pooling.
- **Negative:** SQLite has limited concurrent write throughput. Not a problem for MC's write volume (audit events + rare user/agent creation).
- **Negative:** No remote database access. MC must run on the same machine as (or have filesystem access to) the database file.

---

## ADR-005: Push-only WebSocket, not request/response {#adr-005}

**Date:** 2026-02-15
**Status:** Accepted

### Context

The frontend needs real-time updates when task state changes. Options:

1. **Polling from client** — Simple but wasteful and introduces visible lag.
2. **Server-Sent Events (SSE)** — One-directional, good fit, but axum's SSE support is less mature than WS.
3. **Full-duplex WebSocket** — Client sends requests, server responds. More complex protocol.
4. **Push-only WebSocket** — Server sends events, client ignores any incoming messages (except close).

### Decision

Use a **push-only WebSocket**. The server broadcasts a `{"type":"refresh"}` event after each poll tick. The client, upon receiving this event, fetches the latest snapshot via `GET /api/kanban`.

### Consequences

- **Positive:** Minimal WebSocket protocol. No request/response framing to design or maintain.
- **Positive:** The REST API remains the single source of truth for data. WebSocket is just a notification channel.
- **Negative:** Client makes an extra HTTP request per refresh event. At 5s intervals with a small payload, this is negligible.
- **Negative:** No differential updates — client always fetches the full snapshot. Fine for current data volume (dozens of tasks, not thousands).

---

## ADR-006: Cron integration via Gateway RPC, not CLI {#adr-006}

**Date:** 2026-02-16
**Status:** Accepted (stub implementation)

### Context

MC needs to display and control OpenClaw cron jobs. Unlike bd (which has a stable CLI), the OpenClaw cron system is managed by the Gateway daemon. Options:

1. **CLI subcommand** — `openclaw cron list --json`. But the CLI subcommand wiring may be unavailable in some deployments.
2. **Gateway WebSocket RPC** — Connect to the Gateway's WS endpoint and send JSON-RPC commands.
3. **Read cron YAML files directly** — Parse the cron job definition files from disk.

### Decision

Target **Gateway WebSocket RPC** as the integration path. For MVP, ship a **stub module** (`cron.rs`) that returns empty results. Implement the real RPC client when the Gateway API is stable.

### Consequences

- **Positive:** Correct long-term architecture. Gateway is the authority for cron state.
- **Positive:** Stub allows shipping the dashboard UI without blocking on Gateway API.
- **Negative:** Cron section is empty in MVP. Users see the UI shell but no data.
- **Negative:** Gateway RPC protocol is not yet documented. Integration will require reverse-engineering or collaborating with the Gateway team.

---

## ADR-007: Vanilla JS frontend, no framework {#adr-007}

**Date:** 2026-02-15
**Status:** Accepted

### Context

MC needs a browser frontend. Options range from React/Vue/Svelte to vanilla JS.

### Decision

Use **vanilla JavaScript** with static HTML/CSS served from a `static/` directory. No bundler, no npm, no build step.

### Consequences

- **Positive:** Zero build complexity. Edit a file, refresh the browser. No node_modules.
- **Positive:** Entire frontend is ~200 lines of JS. Easy for any developer (or agent) to understand.
- **Positive:** Static files served via `tower-http::ServeDir` — no separate dev server needed.
- **Negative:** No component model. As the UI grows, may hit maintainability limits.
- **Negative:** No TypeScript type safety. Mitigated by the small codebase size.

**Future:** If the frontend grows beyond ~500 lines of JS, consider migrating to a lightweight framework (Preact, Svelte).

---

## ADR-008: Remove legacy Leptos UI code {#adr-008}

**Date:** 2026-02-19
**Status:** Accepted
**Decided by:** Agent (auto-decision during task decomposition, mc-b1j.1)

### Context

The repo contains `src/ui/mod.rs` with a Leptos-based frontend (Dashboard component using reqwasm/gloo-net). This code is dead — `main.rs` does not declare `mod ui`, and the project uses vanilla JS (`static/`) per ADR-007. The Leptos code references hardcoded `ws://localhost:3000/ws` and imports crates not in Cargo.toml.

### Decision

Remove `src/ui/` entirely. It's dead code that confuses contributors and may cause grep false positives.

### Consequences

- **Positive:** Cleaner codebase, no confusion about which UI is active.
- **Negative:** None meaningful. If Leptos is ever reconsidered (unlikely per ADR-007), it can be rebuilt.

---

## ADR-009: Waiting Room lane for uncategorized tasks {#adr-009}

**Date:** 2026-02-19
**Status:** Accepted
**Decided by:** Agent (auto-decision during task decomposition, mc-b1j.2)

### Context

PRD §4.1 specifies "Waiting Room for uncategorized" tasks. However, `bd.rs::lane_from_issue()` defaults unknown status to "Ready", conflating truly ready tasks with tasks that haven't been triaged. The JS frontend already includes "Waiting Room" in its lanes array.

### Decision

Change the default lane in `lane_from_issue()` from "Ready" to "Waiting Room" for tasks that have:
- No `mc/*` label, AND
- Status not in `{in_progress, blocked, closed}`

Tasks explicitly labeled `mc/ready` or with a clear ready-equivalent status retain the "Ready" lane.

### Consequences

- **Positive:** New/uncategorized tasks are visually separated from triaged-ready tasks.
- **Positive:** Matches PRD intent and existing frontend support.
- **Negative:** Existing tasks with no labels will shift from Ready to Waiting Room on upgrade (one-time visual change, no data mutation).

---

## ADR-010: Session tokens over plaintext username cookies {#adr-010}

**Date:** 2026-02-19
**Status:** Accepted
**Decided by:** Agent (auto-decision during task decomposition, mc-b1j.10)

### Context

SPEC §9 acknowledges that `mc_session=<username>` is a known limitation. Anyone on the LAN could forge a session by setting the cookie value. While the threat model is low (single-operator LAN), this is a defense-in-depth improvement that's cheap to implement.

### Decision

Replace plaintext username cookies with UUID session tokens:
1. Add `sessions` table (id UUID PK, username, created_at, expires_at).
2. On login, insert session row, set cookie to UUID.
3. `AuthedUser` extractor looks up UUID in sessions table.
4. Default session TTL: 24 hours (configurable via `MC_SESSION_TTL_HOURS`).

### Consequences

- **Positive:** Cookie forgery requires guessing a UUID (infeasible).
- **Positive:** Sessions can be revoked server-side (delete row).
- **Negative:** Extra DB lookup per authenticated request (~0.1ms with SQLite, negligible).
- **Negative:** Slightly more complex auth code (~20 extra lines).

---

## ADR-011: Agent CRUD API for dynamic registration {#adr-011}

**Date:** 2026-02-19
**Status:** Accepted
**Decided by:** Agent (auto-decision during task decomposition, mc-b1j.6)

### Context

Currently agents are hardcoded via `db::seed_default_agents()` (main, opus46). PRD §4.2 references "registered agents" implying dynamic registration should be possible. As the fleet grows, hard-coding agent lists is unsustainable.

### Decision

Add REST API for agent management:
- `GET /api/agents` — list all
- `POST /api/agents` — register new agent `{id, display_name}`
- `DELETE /api/agents/:id` — unregister

Keep `seed_default_agents()` as a bootstrap convenience. API mutations are audit-logged.

### Consequences

- **Positive:** Operators can add/remove agents without code changes.
- **Positive:** Opens the door for agent self-registration in future.
- **Negative:** No role/permission model yet — any authenticated user can add/remove agents.

---

## ADR-012: Keep Cron Gateway RPC as stub until protocol is documented {#adr-012}

**Date:** 2026-02-19
**Status:** Accepted
**Decided by:** Agent (auto-decision during task decomposition, mc-b1j.11)

### Context

ADR-006 established Gateway RPC as the target for cron integration, with a stub for MVP. The Gateway WS protocol is still undocumented. Attempting to implement against an unstable API would waste effort.

### Decision

Keep cron.rs as a stub. Create a placeholder task (mc-b1j.11) that becomes actionable when Gateway API docs are available. The UI shell for cron remains, showing empty state. Mark mc-b1j.11 as P2.

### Consequences

- **Positive:** No wasted effort on unstable integration.
- **Positive:** Task is tracked and ready when the dependency resolves.
- **Negative:** Cron section continues to show empty state.

---

## ADR-013: Multi-stage Dockerfile with bd bind-mount {#adr-013}

**Date:** 2026-02-19
**Status:** Accepted
**Decided by:** Agent (opus46, mc-b1j.12)

### Context

SPEC §10 outlines both bare-metal and Docker deployment. We need a production-ready Dockerfile and Docker Compose setup for single-machine deployment.

Key constraints:
1. The bd CLI binary lives on the host (`~/.local/bin/bd`) and is statically linked — can be bind-mounted.
2. bd reads `.beads/` relative to CWD to resolve issues.
3. SQLite database must persist across container restarts.
4. Dashboard must be local-only (not exposed on 0.0.0.0 on the host).

Considered approaches:
1. **Install bd inside the container** — Requires copying the binary or building from source inside Docker. Fragile if bd is updated on the host.
2. **Bind-mount bd + .beads from host** — Simple, always uses the host's current bd version and data.
3. **Volume for everything** — Over-sharing; mounts too much of the host filesystem.

### Decision

- **Multi-stage Dockerfile**: Build stage uses `rust:1.85-bookworm`, runtime uses `debian:bookworm-slim` (~80MB final image).
- **Bind-mount bd binary** at `/hostbin/bd` (read-only).
- **Bind-mount `.beads/`** at `/app/.beads/` (read-only) — container CWD is `/app`, so bd finds its data.
- **Named volume** `mc-data` for SQLite persistence at `/data/`.
- **Port binding**: `127.0.0.1:3000:3000` — local-only by design.
- **`MC_ADMIN_PASS` required** — docker-compose.yml uses `${MC_ADMIN_PASS:?...}` to fail fast if unset.

### Consequences

- **Positive:** Final image under 100MB. Single `docker compose up --build` to deploy.
- **Positive:** bd binary stays on the host — no version drift, no rebuild needed when bd updates.
- **Positive:** `.beads/` is read-only in container, preventing accidental writes.
- **Negative:** Host must have a linux/amd64 static bd binary. Won't work on ARM hosts without a compatible binary.
- **Negative:** Dependency caching in Docker is approximate (skeleton project trick); full recompile on any Cargo.toml change.

---

## Decision Log Rules

1. New ADRs are appended with the next sequential number.
2. To reverse a decision, mark the old ADR as `Superseded by ADR-XXX` and create the new one.
3. Never delete an ADR — they are historical records.
4. Each ADR should reference the relevant PRD section and SPEC section where applicable.
5. Agents performing work on Mission Control are expected to add ADRs for any non-trivial architectural choices.
