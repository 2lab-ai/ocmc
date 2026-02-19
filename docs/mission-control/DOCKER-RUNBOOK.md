# Docker Runbook — Mission Control

> Quick-start and operations guide for running Mission Control via Docker Compose.

---

## Prerequisites

| Tool | Min version | Check |
|------|-------------|-------|
| Docker Engine | 24+ | `docker --version` |
| Docker Compose | v2+ | `docker compose version` |
| bd CLI | any | `bd --version` (on host) |

## Quick Start

```bash
cd ~/clawd/mission-control

# 1. Create .env with your admin password.
cp .env.example .env
# Edit .env — at minimum set MC_ADMIN_PASS.

# 2. Build and run.
docker compose up --build -d

# 3. Open dashboard.
open http://localhost:3000       # → redirects to /login
```

Login with `admin` / `<your MC_ADMIN_PASS>`.

## Architecture

```
Host                           Docker (mission-control)
─────────────────────          ─────────────────────────
~/.local/bin/bd ──────ro────►  /hostbin/bd
./.beads/       ──────ro────►  /app/.beads/
(named volume)  ◄────rw────►  /data/mc.db
                 127.0.0.1:3000 ◄── :3000
```

- **Local-only**: port bound to `127.0.0.1:3000` — not reachable from other machines.
- **SQLite**: persisted in the `mc-data` Docker volume at `/data/mc.db`.
- **bd CLI**: bind-mounted read-only from host; the container's CWD is `/app` which contains the `.beads/` mount so bd can resolve issues.

## Common Operations

### Start / Stop / Restart

```bash
docker compose up -d            # start (detached)
docker compose down             # stop & remove container
docker compose restart          # restart
docker compose up --build -d    # rebuild image then start
```

### View Logs

```bash
docker compose logs -f          # follow all logs
docker compose logs --tail=50   # last 50 lines
```

### Health Check

```bash
curl -s http://127.0.0.1:3000/healthz
# → "ok"

docker inspect --format='{{.State.Health.Status}}' mission-control
# → "healthy"
```

### Reset Admin Password

```bash
# Stop, update .env, remove the SQLite volume, restart.
docker compose down
docker volume rm mission-control_mc-data   # ⚠ deletes all MC data
# Edit .env — set new MC_ADMIN_PASS
docker compose up --build -d
```

Alternatively, keep the volume and manually update the password hash using `sqlite3` on the volume mount.

### Backup SQLite

```bash
docker compose exec mission-control cp /data/mc.db /data/mc.db.bak
# Or from host:
docker cp mission-control:/data/mc.db ./mc-backup-$(date +%F).db
```

## Volume Reference

| Volume / Mount | Container path | Mode | Purpose |
|---|---|---|---|
| `mc-data` (named) | `/data/` | rw | SQLite database |
| `${MC_BD_BIN_HOST}` | `/hostbin/bd` | ro | bd CLI binary |
| `${MC_BEADS_DIR}` | `/app/.beads/` | ro | bd issue data |

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `MC_ADMIN_USER` | `admin` | Bootstrap admin username |
| `MC_ADMIN_PASS` | *(required)* | Bootstrap admin password |
| `MC_POLL_MS` | `5000` | Poller interval (ms) |
| `MC_BD_BIN_HOST` | `~/.local/bin/bd` | Host path to bd binary |
| `MC_BEADS_DIR` | `./.beads` | Host path to .beads directory |
| `MC_GATEWAY_URL` | `ws://host.docker.internal:18789` | OpenClaw Gateway URL |
| `RUST_LOG` | `info` | Log level |

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `bd list` errors in logs | bd binary not found or wrong arch | Check `MC_BD_BIN_HOST` path; must be linux/amd64 static binary |
| "no snapshot" on `/api/kanban` | Poller hasn't completed first tick | Wait a few seconds; check logs for bd errors |
| Login fails | Wrong password or first-run not seeded | Verify `MC_ADMIN_PASS` matches .env; check if volume was re-created |
| Port already in use | Another process on :3000 | `lsof -i :3000` and stop the conflicting process |
| Container unhealthy | App not starting | `docker compose logs` for startup errors |
