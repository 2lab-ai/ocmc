# CLAUDE.md — Mission Control (2lab-ai/ocmc)

## Project Identity

Mission Control (MC) is a Rust/Axum dashboard for managing AI agents, tasks (kanban), and cron jobs. Repo: `2lab-ai/ocmc`.

## CEO Workflow Rules

### 1. Orchestrator, Not Implementer

The CEO (Z) orchestrates via task decomposition and cross-verification. Agents do the implementation work. The CEO:

- Decomposes high-level goals into bd tasks with clear acceptance criteria
- Dispatches tasks to agents via bd comments
- Cross-verifies deliverables (code review, AC checklist, build checks)
- Does **not** write production code directly (except emergency hotfixes)

### 2. Cross-Verification via bd Comments

Every deliverable must be verified and evidenced in bd comments:

- Agent posts completion evidence (commit SHA, PR link, test results)
- CEO or another agent cross-reviews against acceptance criteria
- Verification result (PASS/FAIL + notes) recorded as a bd comment
- No task is marked `closed` without a verification comment

### 3. Git Discipline

- **Commit messages** must reference the bd issue id: `feat(scope): description [mc-xxx]`
- **Co-authorship** is mandatory: every commit must include `Co-authored-by: Z <z@2lab.ai>`
- **Push early, push often** — work must be visible on origin
- **PRs must be merged when green** — no "PR parking" (open PR with passing checks left unmerged)
- **Branch naming**: `feat/mc-xxx-slug`, `fix/mc-xxx-slug`, `docs/mc-xxx-slug`
- **Rebase over merge** for feature branches (clean linear history)

### 4. Folder Discipline

```
/
├── CLAUDE.md          # This file — project rules for Claude agents
├── AGENTS.md          # Agent role definitions and workflow
├── Cargo.toml
├── src/               # Rust source (Axum server, bd integration, cron, auth)
├── static/            # Frontend (vanilla JS dashboard)
├── docs/
│   └── mission-control/  # Project docs (PRD, SPEC, DECISIONS, runbooks)
├── tests/             # Integration tests
├── Dockerfile
├── docker-compose.yml
└── .beads/            # bd task database (do NOT manually edit issues.jsonl)
```

- Production code lives in `src/` and `static/` only
- Documentation goes in `docs/mission-control/`
- Never commit secrets, `.env`, or `target/` directory
- `.beads/` is managed by the `bd` CLI — do not hand-edit

### 5. Commit Co-authorship (REQUIRED)

Every commit in this repo **must** include:

```
Co-authored-by: Z <z@2lab.ai>
```

This is enforced via git commit template. See `.gitmessage` in repo root.

## Build & Test

```bash
cargo build          # Build
cargo test           # Run tests
cargo run            # Start server (needs MC_ADMIN_PASS env)
```

## Key Architecture

- **Axum** web server with tower middleware
- **SQLite** for sessions/audit (via rusqlite)
- **bd CLI** for task management (polled periodically)
- **HMAC-SHA256** signed session tokens (not plaintext cookies)
- **Vanilla JS** frontend (no framework, no build step)
