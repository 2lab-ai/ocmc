# mc-6yy.5 — Agent-to-MC Authentication Design

## Problem
Agent-only endpoints (heartbeat, future automation) are unauthenticated.
Any network caller can impersonate an agent or spam heartbeats.

## Threat Model
| Threat | Impact | Mitigation |
|--------|--------|------------|
| Unauthorized heartbeat spoofing | False agent-alive status | Per-agent API key (Bearer token) |
| Key leakage from env/logs | Full agent impersonation | Store SHA-256 hash in DB; never log raw keys |
| Replay attacks | Stale heartbeats | Acceptable risk for MVP; add nonce/timestamp later |
| Brute-force key guessing | Unauthorized access | 256-bit random keys; rate-limit later |
| Timing attacks on comparison | Key extraction | Constant-time comparison of hex hashes |

## Decision: Per-Agent API Key (Bearer Token)

**Chosen over signed JWT because:**
- Simpler — no key rotation ceremony, no clock-skew issues, no JWT library dependency.
- Stateless verification: compare `Authorization: Bearer <key>` against stored SHA-256 hash.
- Sufficient for agent→MC trust (agents are our own processes, not third-party).

**Tradeoffs:**
- Keys are long-lived (rotate manually or via future automation).
- No embedded claims (agent-id comes from URL path, validated against key's owner).

## Secret Storage

| Environment | Storage | How to set |
|-------------|---------|------------|
| DB | `agents.api_key_hash` column (SHA-256 hex) | Via SQL or future admin endpoint |
| Local dev | Generate key, hash it, store hash in SQLite | `echo -n "my-key" \| sha256sum` then INSERT |

Raw keys are **never stored**. Only SHA-256 hashes live in the database.

## Auth Flow
1. Agent sends `POST /api/agents/:id/heartbeat` with `Authorization: Bearer <key>`.
2. Handler extracts bearer token from header.
3. Looks up `agents.api_key_hash` for the given agent ID.
4. SHA-256 hashes the provided token, constant-time compares against stored hash.
5. **401** if missing/invalid token; **403** if agent not found or has no key.
6. On success, records heartbeat timestamp and returns 200.

## Endpoints Protected
- `POST /api/agents/:id/heartbeat` — agent liveness heartbeat
- Future agent-only endpoints will use the same `validate_agent_bearer()` function.

## Migration
- `agents` table gets two new nullable columns: `api_key_hash TEXT` and `last_heartbeat_at TEXT`.
- Existing agents start with NULL key (cannot authenticate until key is provisioned).
