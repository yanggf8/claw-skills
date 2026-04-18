# Persona-webapp reconciliation plan

**Date**: 2026-04-18
**Status**: draft, awaiting review (P5 goes to Gemini)

## Context

The webapp implementation diverged from the 2026-04-17 design:

- `editorial_plan` → replaced by `content_column` (adds `persona_slug`, `kind: finite|ongoing`, `status`, `updated_at`)
- `editorial_topic` → renamed to `installment` (field-for-field match)
- New `stream` + `issue` tables carry the `kind=ongoing` case (cadence-based, not week-based)

Schema currently lives only in Turso — no `CREATE TABLE` in repo. Seed and `lib/persona_history.py` still reference the old names.

`persona-skill/scripts/persona_skill.py` has in-progress `_webapp_request` helper — signals movement toward API-based CLI↔webapp integration (P5 Option B).

## Plan

### P1. Schema init script (no migration)

New file `lib/schema_init.py` with a single `init_schema(conn)` that creates every table in dependency order plus indexes. Idempotent (`CREATE TABLE IF NOT EXISTS`). No version table, no migration machinery.

Tables: `user`, `invite`, `persona`, `persona_history`, `content_column`, `installment`, `stream`, `issue`, `audit_log`.

Retire: `MIGRATIONS` list in `lib/persona_history.py`. `ensure_schema` becomes a thin wrapper.

Rationale: data is already in Turso. Schema init is for future DB swaps, not for migrating the existing one.

### P2. Split seed script from seed data

- `lib/seed.py` — data-agnostic orchestrator, idempotent, reads from data files
- `seed_data/personas.jsonl`
- `seed_data/invites.jsonl`
- `seed_data/users.jsonl`

Retire: `docs/superpowers/seed.py` (inline Python literals for data).

### P3. Update design doc

`docs/superpowers/specs/2026-04-17-persona-webapp-design-WIP.md`:

- Rename → drop `-WIP` suffix
- Status → "implemented 2026-04-18"
- Section 2: replace `editorial_plan` subsection with `content_column` + `stream` + `installment` + `issue`
- Section 3: replace `/plans` routes with `/columns` + `/streams`; add `/dashboard`, `/me`
- Section 4: add OAuth state CSRF cookie note
- Section 7: leave as-is (ops items still open)

### P4. Complete persona-skill CLI

Current CLI has stale `plan-list` / `plan-show` reading `editorial_plan`.

Rename + extend to match the webapp model:

- `column-list`, `column-show`, `column-create`, `column-update`, `column-delete`
- `installment-add`, `installment-update`, `installment-delete`
- `stream-list`, `stream-show`, `stream-create`, `stream-update`, `stream-delete`
- `issue-add`, `issue-update`, `issue-delete`

Remove: `plan-list`, `plan-show`.

### P5. Writer-side data access (Gemini review)

**Problem**: persona-skill writer prompt injects topic context (angle, lens, direction, key_question) at publish time. Source of truth moved from `editorial_topic` → `installment` / `issue`. Writer runs in CLI process and needs to read these rows, then mark them published.

**Options:**

| | Read | Write (mark published) | Pros | Cons |
|---|---|---|---|---|
| **A. Direct DB** | Turso `installment` | Turso + `persona_history` | Simplest, matches current history write-back | Two writers (webapp UI + CLI) on same row, theoretical race |
| **B. API-only** | `GET /api/columns/:id` + service token | `POST /api/personas/:slug/history` | Single write path, DB token stays in webapp only | Cron depends on webapp uptime; new allowlist paths |
| **C. Split** | Direct DB for reads | Service token API for writes | Minimal service-token surface | Two code paths, consistency harder to reason about |

**Current direction (in-progress)**: persona-skill already has `_webapp_request` helper — pointing at Option B.

**Lean**: A would be simplest. Race is theoretical for a single-user nightly cron. Option B couples cron availability to webapp deploy.

**For Gemini**: evaluate against failure modes (webapp down during cron, DB token rotation, concurrent webapp edit during cron window) and operational simplicity. Given B is already partway implemented, is the cost of switching back to A worth the simplicity? Or does B's clean separation justify the deploy-dependency cost?

## Execution order

1. **P3** — doc update (cheap, documentation truth first)
2. **P1** — schema init (reproducibility fix)
3. **P2** — seed split (data hygiene)
4. **P5 Gemini review** — decide A vs B vs C
5. **P4** — CLI completion, using the P5 decision for writer path

Everything except P5 can proceed without blocking on review.
