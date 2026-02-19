---
name: what-to-work
description: Decide what to work on next by triaging bd tasks, bundling into up to three options, or routing to plan-new-task when backlog is empty or too small
---

# What To Work

## Goal

Provide clear, user-confirmable next work options. If the backlog is too small to form a meaningful bundle, proactively route to plan-new-task (which uses new-task) to create new work.

## Sizing rubric (expected code change, added + deleted)

- tiny: ~10 lines
- medium: ~100 lines
- large: ~250 lines
- xlarge: ~500 lines

## Workflow

1. Scan backlog (do-work guidance)
   - Run bd ready in all relevant project roots.
   - If multiple known repos exist, follow do-work guidance to scan them all.
   - Use bd show on top items to estimate size and dependencies.
   - Capture id, title, priority, type, dependencies, and size estimate (tiny/medium/large/xlarge).

2. Decide if backlog is bundle-worthy
   - Bundle-worthy means you can form at least one large or xlarge bundle.
   - Target xlarge (~500 lines) and do not exceed xlarge.
   - If only tiny/medium or admin/event tasks remain, or total expected change is below large, treat the backlog as too small.

3. Route
   - If bundle-worthy, use what-we-have-to-work to propose 1-3 bundles.
   - If empty or too small, use plan-new-task to propose 2-4 new feature ideas based on bd list and user context.

4. Present next action
   - State which route you are using and why.
   - Ask for any missing context only if it blocks routing.

## Output Template

### Route: what-we-have-to-work

```text
Backlog check complete: [summary]
Route: what-we-have-to-work
Next step: I'll bundle tasks into up to three options for you to pick.
```

### Route: plan-new-task

```text
Backlog check complete: [summary]
Route: plan-new-task
Next step: I'll propose new features based on bd list and your context.
```

## Integration

- Use do-work Phase A guidance for scanning repos and sizing bundles.
- Use what-we-have-to-work when you can form a large/xlarge bundle (cap at xlarge).
- Use plan-new-task when backlog is empty or too small; plan-new-task uses new-task for decomposition.
- After user selection, follow do-work for execution.
