---
name: what-we-have-to-work
description: Bundle existing bd tasks into up to three candidate work chunks, present options with rationale, and get user confirmation before starting do-work execution. Use when ready tasks exist and the user wants a clear choice
---

# What We Have To Work

## Goal

Turn the current bd backlog into up to three clear work bundles and get a fast user selection.

## Sizing rubric (expected code change, added + deleted)

- tiny: ~10 lines
- medium: ~100 lines
- large: ~250 lines
- xlarge: ~500 lines

## Workflow

1. Survey tasks
   - Run bd ready in all relevant project roots.
   - Pull details for top tasks with bd show when needed.
   - Note priority, dependencies, repo, and size estimate (tiny/medium/large/xlarge).

2. Check bundle viability
   - If total expected change is below large or you only have tiny/medium tasks, stop and switch to plan-new-task.
   - Do not offer a single small task as the only option.

3. Build bundles
   - Create one to three bundles, not more.
   - Do not force three bundles if the backlog is small.
   - Aim for xlarge bundles; large is acceptable if xlarge would require unrelated work.
   - Do not exceed xlarge.
   - Prefer grouping by dependency chain, shared area, or repo.
   - Include tiny leftover tasks only as add-ons, not as standalone bundles.
   - Do not mix unrelated tasks just to hit size targets.

4. Present options
   - For each bundle, list tasks with ids, size estimates, and a one line rationale.
   - Ask the user to reply with 1, 2, or 3 and any extra context.

5. After user selection
   - Update the chosen tasks to in_progress.
   - Follow do-work Phase B for execution and quality gates.

6. Empty or tiny backlog
   - If you cannot form a large/xlarge bundle, stop and switch to plan-new-task.

## Output Template

```text
I found [count] ready tasks. Here are the bundles:

1) [Bundle title] ([size])
   - [id] [task title]
   - [id] [task title]
   Rationale: [short reason]

2) ...
3) ...

Reply with 1, 2, or 3 and any context or constraints. If you want a different bundle, say what to include.
```

```text
bd update <id> --status in_progress
```

## Integration

- Use what-to-work for routing decisions.
- Use plan-new-task if the backlog is empty or too small.
- Use do-work after the user picks a bundle.
