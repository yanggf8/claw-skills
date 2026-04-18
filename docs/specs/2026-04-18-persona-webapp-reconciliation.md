# Persona-webapp reconciliation plan

**Date**: 2026-04-18
**Status**: P3 done, P5 decided (Option B + fallback), P1/P2/P4/P5-impl pending

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

### P5. Writer-side data access — Option B with direct-DB fallback (decided 2026-04-18)

**Problem**: persona-skill writer prompt injects topic context (angle, lens, direction, key_question) at publish time. Source of truth moved from `editorial_topic` → `installment` / `issue`. Writer runs in CLI process and needs to read these rows, then mark them published.

**Race analysis (revised)**:
- Webapp is an editing/management UI with per-user data scope; it does not write articles.
- Articles are written by skills (CLI). Webapp and CLI do not contend for the same `installment` / `issue` rows.
- Only overlap surface is `persona`: admin could edit a persona while CLI is reading it for writer prompt → handled by optimistic concurrency (re-read `persona_updated_at` at write-back; if changed since prompt assembly, warn and skip).
- The theoretical race that originally favoured Option A does not apply.

**Decision**: Option B (API-first) with direct-Turso fallback when the API is unreachable.

- CLI tries webapp API first (`PERSONA_API_URL` + `PERSONA_SERVICE_TOKEN` in env). `_webapp_request` helper in `persona_skill.py` already implements this path.
- On connection error / timeout / 5xx → fall back to direct Turso client (existing `lib/persona_registry` + `lib/persona_history`). Keeps cron robust when webapp is down.
- Personas: optimistic concurrency via `persona_updated_at` compare before history write-back.

**API additions needed** (extend `SERVICE_ALLOWED_PATHS` in `src/auth.ts`):

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/personas/:slug/columns` | Writer lists persona's columns to pick next installment |
| GET | `/api/personas/:slug/streams` | Same for streams/issues |
| GET | `/api/columns/:id/installments/next` | Writer fetches next `planned` installment |
| GET | `/api/streams/:id/issues/next` | Same for issues |
| PUT | `/api/columns/installments/:id` | Mark installment published, set history_id |
| PUT | `/api/streams/issues/:id` | Same for issue |

`POST /api/personas/:slug/history` already exists for history write-back.

**Rationale for B over A**:
- Single write path through webapp preserves scope checks + audit log
- DB token stays in webapp only (smaller blast radius if CLI host compromised)
- Service token infrastructure + allowlist already exist
- Fallback path covers the webapp-downtime concern that was B's main weakness

## Execution order

1. **P3** — doc update ✅ (done 2026-04-18)
2. **P1** — schema init script
3. **P2** — seed script / data split
4. **P4** — CLI rename + extend (`column-*`, `installment-*`, `stream-*`, `issue-*`)
5. **P5** — implement API additions in webapp + wire CLI fallback path

P5 no longer blocked on review.
