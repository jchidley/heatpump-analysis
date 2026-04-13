# Documentation Guide

This repo uses [`../lat.md/`](../lat.md/) as the canonical structured source of current project truth. The files under `docs/` are the human-facing complement: they explain decisions, give task guides, preserve audits, and help you find the right source.

## Start here

Choose the question you are trying to answer.

| Question | Best place to start |
|---|---|
| What does the system currently do? | [`../lat.md/`](../lat.md/) |
| Why was a control or DHW decision made? | [Heating plan](heating-plan.md), [DHW plan](dhw-plan.md) |
| How do I perform an operational task? | [History evidence workflows](history-evidence-workflows.md), [emon installation runbook](emon-installation-runbook.md), [`../deploy/SECRETS.md`](../deploy/SECRETS.md) |
| Where is TSDB migration tracked? | [`../lat.md/tsdb-migration.md`](../lat.md/tsdb-migration.md) for repo-local cutover, `~/github/energy-hub/lat.md/tsdb-migration.md` for shared platform phases |
| Where is this implemented in code? | [`implementation-maps/`](implementation-maps/), [`../lat.md/src/`](../lat.md/src/), and the source tree |
| What should an agent remember while working? | [`../AGENTS.md`](../AGENTS.md) |

## Human-facing docs by type

### Explanation: why the system is designed this way

Use these when you want context, rationale, trade-offs, and lessons learned.

- `heating-plan.md` — space-heating objective, control policy, and next decisions
- `dhw-plan.md` — DHW objective, scheduling strategy, and trade-offs
- `explanation.md` — why the operating-state model uses flow-based classification
- `hydraulic-analysis.md` — why the y-filter diagnosis and threshold tightening happened
- `house-layout.md` — building survey companion to the house facts summarised in `lat.md`
- `room-thermal-model.md` — methodology companion to the canonical thermal-model summary in `lat.md`

### How-to: how to perform a task

Use these when you already understand the system and need steps.

- `history-evidence-workflows.md` — operator workflow for reconstructing heating, DHW, and joined historical evidence
- `emon-installation-runbook.md` — rebuild and recover monitoring devices
- `../deploy/SECRETS.md` — handle tokens and environment secrets safely
- `../heating-monitoring-setup.md` — operate and inspect the monitoring stack in detail

### Reference: facts, inventories, and audit trails

Use these when you need exact supporting details rather than narrative.

- `heating-reference.md` — supporting heating-control reference and field notes
- `dhw-reference.md` — supporting DHW measurements, traces, and usage reference
- `vrc700-settings-audit.md` — full VRC 700 audit trail and timer-encoding investigation
- `octopus-data-inventory.md` — Octopus data audit
- `pico-ebus-plan.md` — planned Pico eBUS replacement reference/build notes

Earlier heavyweight wording now lives in git history rather than a permanent `docs/archive/` mirror. Active docs should retain the durable operator and reference detail they still need.

## Structured project truth ([`../lat.md/`](../lat.md/))

Use [`lat.md/`](../lat.md/) when asking what is true **now**.

- [`../lat.md/domain.md`](../lat.md/domain.md)
- [`../lat.md/constraints.md`](../lat.md/constraints.md)
- [`../lat.md/architecture.md`](../lat.md/architecture.md)
- [`../lat.md/heating-control.md`](../lat.md/heating-control.md)
- [`../lat.md/thermal-model.md`](../lat.md/thermal-model.md)
- [`../lat.md/history-evidence.md`](../lat.md/history-evidence.md)
- [`../lat.md/infrastructure.md`](../lat.md/infrastructure.md)
- [`../lat.md/tsdb-migration.md`](../lat.md/tsdb-migration.md)

Use this for:
- architecture and implicit contracts
- current domain facts and operating assumptions
- constraints and gotchas
- infrastructure inventory and baseline settings
- the repo-local TSDB migration tracker, alongside the shared `energy-hub` migration file for platform-wide phases

## Implementation maps (`implementation-maps/` + `lat.md/src/` + source tree)

Use these when the question is: **where in the repo do I change this?**

- `implementation-maps/` — preserved implementation snapshots migrated from the retired `code-truth/` folder
- `../lat.md/src/` — file-level source pages that have dedicated `lat.md` entries
- `../src/` — primary Rust implementation
- `../model/` — controller and thermal config inputs
- `../scripts/` — deployment and verification helpers

Thematic current truth still lives in `../lat.md/`; these paths are for code location and implementation discovery.

## Practical rule of thumb

- **What should happen, and why?** → `heating-plan.md`, `dhw-plan.md`
- **What is true right now?** → [`../lat.md/`](../lat.md/)
- **How do I perform an operational task?** → runbooks and how-to docs
- **How do I review a past window?** → `history-evidence-workflows.md`
- **Where do I change the code?** → `implementation-maps/`, `../lat.md/src/`, and the source tree
- **What should an agent keep in mind?** → `../AGENTS.md`
