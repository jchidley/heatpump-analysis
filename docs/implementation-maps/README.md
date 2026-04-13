# Implementation Maps

These documents preserve the retired `docs/code-truth/` snapshots so implementation-discovery detail was not lost when the folder was removed.

They describe the implementation structure of the repository as observed in source at the time they were generated/audited.

Use them for:
- locating code
- understanding module boundaries
- onboarding to the repo
- checking whether higher-level docs still match implementation

Do **not** treat them as the primary source for:
- current heating or DHW operating policy
- deployment procedures
- secrets handling
- live production state

For those, see:
- `../heating-plan.md`
- `../dhw-plan.md`
- `../../deploy/SECRETS.md`
- `../../AGENTS.md`

## Files

- `REPO_OVERVIEW.md` — what the repo does at a high level
- `ARCHITECTURE.md` — major components, dependencies, and data flow in code
- `REPOSITORY_MAP.md` — where important files and responsibilities live
- `PATTERNS.md` — recurring implementation patterns and conventions
- `DECISIONS.md` — implementation-level architectural decisions captured from source
