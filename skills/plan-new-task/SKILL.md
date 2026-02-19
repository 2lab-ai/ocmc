---
name: plan-new-task
description: Propose new bd tasks when the backlog is empty or too small by reviewing bd history and user input, then applying the new-task workflow to create a task tree. Use when there are no ready tasks or only tiny/medium leftovers
---

# Plan New Task

## Goal

When there is no meaningful work left, proactively propose new feature work based on bd history and user context, then turn it into bd tasks using new-task.

## Sizing rubric (expected code change, added + deleted)

- tiny: ~10 lines
- medium: ~100 lines
- large: ~250 lines
- xlarge: ~500 lines

## Workflow

1. Confirm backlog status
   - Run bd ready to confirm no ready tasks.
   - Run bd list to scan recent completed work and gaps.
   - Note patterns, unfinished areas, and obvious follow ups.
   - If tiny or medium leftovers exist, list them separately as carryover.

2. Propose next work
   - Suggest two to four candidate features based on bd history and user context.
   - Tie each idea to recent work or visible gaps.
   - Ask the user to pick one or provide a new idea.

3. Plan with new-task
   - Switch to the new-task workflow for the chosen idea.
   - Follow its phases for requirements, feature list, architecture review, and task tree.
   - Do not create bd tasks until the user approves the plan.

4. Hand off
   - After tasks are created, suggest using do-work to execute.

## Output Template

```text
Backlog is empty or too small. Based on bd history, here are good next features:

1) [Idea A] - [one line reason]
2) [Idea B] - [one line reason]
3) [Idea C] - [one line reason]

Carryover (tiny leftovers, optional):
- [id] [task title]

Pick a number or describe a different feature. I will then break it down with new-task and create bd items after your approval.
```

## Integration

- Use what-to-work to decide when to enter this flow.
- Use new-task for decomposition and task creation.
- Use do-work after tasks exist.
