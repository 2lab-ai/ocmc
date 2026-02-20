#!/usr/bin/env bash
# Mock bd binary for integration tests (mc-b1j.14)
# Returns fixture JSON for `list --json` and records/echoes args for `update`.

# If MOCK_BD_ARGS_LOG is set, log all invocations
if [ -n "$MOCK_BD_ARGS_LOG" ]; then
    echo "$@" >> "$MOCK_BD_ARGS_LOG"
fi

case "$1" in
    list)
        # bd list --json --limit 0
        cat <<'EOF'
[
  {"id": "mc-b1j.1", "title": "Remove Leptos UI", "status": "closed", "labels": ["mc/done"], "assignee": "opus46", "priority": "high", "updated_at": "2026-02-10T10:00:00Z"},
  {"id": "mc-b1j.2", "title": "Fix waiting-room lane", "status": "open", "labels": ["mc/doing", "bug"], "assignee": "opus46", "updated_at": "2026-02-15T12:00:00Z"},
  {"id": "mc-b1j.3", "title": "Enhance task cards", "status": "open", "labels": ["mc/ready"], "updated_at": "2026-02-16T09:00:00Z"},
  {"id": "mc-b1j.4", "title": "Kanban lane headers", "status": "open", "labels": ["mc/blocked"], "updated_at": "2026-02-17T14:00:00Z"},
  {"id": "mc-b1j.5", "title": "WS connection status", "status": "open", "labels": ["mc/backlog"]},
  {"id": "mc-b1j.6", "title": "Future feature", "status": "open", "labels": []},
  {"id": "mc-b1j.7", "title": "Another item", "status": "in_progress", "labels": []}
]
EOF
        exit 0
        ;;
    update)
        # Echo back args so tests can verify
        echo "updated: $@"
        exit 0
        ;;
    *)
        echo "mock_bd: unknown command: $@" >&2
        exit 1
        ;;
esac
