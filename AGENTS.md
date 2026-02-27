# AGENTS.md — Agent Workflow Rules for Mission Control

## Roles

| Role | Scope | Examples |
|------|-------|----------|
| **PD** (Product/Design) | Specs, PRDs, design docs, acceptance criteria | PRD drafts, SPEC updates, ADRs |
| **Engineer** | Implementation, code, tests | Feature branches, bug fixes, unit tests |
| **QA-DevOps** | Verification, deployment, CI | Cross-review, Docker, integration tests |

## Workflow for Every Task

### Before Starting

1. Read the bd task (`bd show <id>`) — understand AC fully
2. Check dependencies (`bd graph <id>`) — don't start blocked work
3. Create a feature branch: `git checkout -b feat/mc-<id>-<slug> main`

### While Working

4. Make minimal, focused changes — one task per branch
5. Commit frequently with proper format:
   ```
   feat(scope): description [mc-xxx]

   Co-authored-by: Z <z@2lab.ai>
   ```
6. Push to origin early: `git push -u origin feat/mc-<id>-<slug>`

### On Completion

7. Verify your own work against **every** AC checkbox
8. Open a PR (or push to existing branch)
9. Post evidence in bd comment:
   ```
   ## mc-<id> Complete (<agent-name>)
   Commit: <sha>
   PR: <url>
   AC checklist:
   - [x] item 1
   - [x] item 2
   ```
10. **If PR is green → merge it.** Do not leave PRs parked.

### Cross-Verification

- Another agent or the CEO will cross-verify against AC
- Verification is recorded as a bd comment (PASS/FAIL)
- FAIL means fix and re-submit; do not close the task
- Canonical verification ledger issue id: `mission-control-dvg`
- Every PASS/FAIL + evidence note must be posted to `mission-control-dvg` (task-local comments are still allowed)
- Do not use `mc-p7w` (unresolvable in this DB) or `mission-control-hp1` (legacy history only) for new verification entries

## Git Rules (Non-Negotiable)

- **Every commit** includes `Co-authored-by: Z <z@2lab.ai>` trailer
- **Every commit message** references the bd issue id in brackets: `[mc-xxx]`
- **Never force-push** to `main`
- **Never commit** secrets, `.env` files, or build artifacts
- **Rebase** feature branches on main before merging
- **Delete** feature branches after merge

## bd (Beads) Usage

```bash
bd list                    # List all tasks
bd show <id>               # Show task details
bd comment <id> "text"     # Add a comment
bd update <id> status=closed  # Close a task (only after verification)

# Verification ledger (canonical)
bd show mission-control-dvg
bd comment mission-control-dvg "PASS/FAIL + evidence"
```

## Folder Rules

- Rust code → `src/`
- Frontend → `static/`
- Docs → `docs/mission-control/`
- Tests → `tests/`
- Do NOT hand-edit `.beads/issues.jsonl`

## PR Merge Policy

> **PRs must be merged when CI is green. No PR-only parking.**

If a PR has been open > 1 hour with passing checks and no review comments, the assignee should merge it. Stale PRs waste context and create merge conflicts.
