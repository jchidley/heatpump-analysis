# Documentation Guide

This repo has four main kinds of documentation. They serve different purposes and should not be treated as interchangeable.

## 1. Strategy and operating policy

Use these when asking **what the system should do and why**:

- `heating-plan.md` — space-heating objective, constraints, control policy, and rationale
- `dhw-plan.md` — domestic hot water objective, cylinder model, operating policy, and rationale

These are the canonical references for:
- comfort targets
- DHW strategy
- measured system constraints
- control decisions and trade-offs
- what is live now vs planned next

## 2. Operations and deployment

Use these when asking **how the live system is deployed, checked, recovered, or configured**:

- `current-production-state.md` — compact live-state summary for pi5data
- `live-queries.md` — how to fetch live readings and snapshots on demand
- `history-evidence-workflows.md` — step-by-step historical evidence recipes for heating, DHW, and joined questions
- `history-evidence-plan.md` — reference / roadmap: authority map, anchor windows, command catalog, maturity, and next-step evidence links
- `../deploy/SECRETS.md` — InfluxDB token handling, dev vs prod
- `emon-installation-runbook.md` — monitoring and infrastructure rebuild/runbook
- `heating-monitoring-setup.md` — broader monitoring topology and setup context

These are the canonical references for:
- secrets
- service environment setup
- infrastructure recovery
- deployment paths and operational procedures

## 3. Implementation maps (`code-truth/`)

Use these when asking **where in the codebase something is implemented**:

- `code-truth/REPO_OVERVIEW.md`
- `code-truth/ARCHITECTURE.md`
- `code-truth/REPOSITORY_MAP.md`
- `code-truth/PATTERNS.md`
- `code-truth/DECISIONS.md`

These documents are derived from source and are best for:
- repo onboarding
- finding the right file to edit
- understanding module boundaries
- checking architectural drift after refactors

They are **not** the primary source for current operational policy or deployment truth. For that, use the plan docs and runbooks above.

## 4. Agent/project context

Use `../AGENTS.md` for compact machine/project context, operational gotchas, and agent-facing reminders.

This is the right place for:
- environment constraints
- short critical facts
- warnings and pitfalls

It is **not** the best place for full strategy explanations or detailed code maps.

## Practical rule of thumb

- **What should happen?** → `heating-plan.md`, `dhw-plan.md`
- **What is live right now?** → `current-production-state.md`, `live-queries.md`
- **How should historical review queries be architected?** → `history-query-architecture-plan.md`
- **How do I reconstruct a past window?** → `history-evidence-workflows.md`
- **What is the authority map / evidence roadmap?** → `history-evidence-plan.md`
- **How do I operate/recover it?** → runbooks / `deploy/SECRETS.md`
- **Where do I change the code?** → `docs/code-truth/`
- **What should an agent remember while working?** → `AGENTS.md`
