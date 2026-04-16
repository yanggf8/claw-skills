# Turso Consolidation — Persona Registry + History

**Status:** Draft for review
**Date:** 2026-04-16
**Author:** yanggf
**Supersedes (in part):** `persona-skill/SKILL.md` runtime contract; `ainews DESIGN.md §2` persona-skill description; `ainews DESIGN.md §6` `writer_hash` semantics.

---

## 1. Purpose

Consolidate all persona- and article-related persistence into a single Turso (libsql) database named `persona-registry`. Replace persona-skill's YAML-on-disk backend with three Turso tables: `persona`, `persona_secret`, `persona_history`. Per-persona dev.to API keys move from shared `DEV_TO_API_KEY` env var to `persona_secret` rows. One database, one auth token, one source of truth across mindfulness-spirit (live), ainews (spec'd), and any future writer skill.

**Explicitly not:**

- Not moving article bodies into Turso. `persona_history` continues to store metadata only (title, stance, key_links, dev.to id/url); article Markdown stays on dev.to + local `runs/` / `failed/`.
- Not moving oilcon's data. Oil prices stay in their own Turso DB with their own token.
- Not keeping a local-file fallback. `file://` URLs are removed from both `lib/persona_history.py` and the new `lib/persona_registry.py`. `:memory:` stays for unit tests only.
- Not keeping YAML as a runtime source. YAML is an upsert input format, held in git, never read at runtime.

## 2. Motivation

Three current pain points this consolidates:

1. **Split storage.** persona-skill stores identity in YAML; mindfulness-spirit reads secrets from `~/.nullclaw/.env`; `persona_history` stores articles in Turso. Three backends, three failure modes.
2. **Per-persona dev.to identity is blocked.** A persona named "Ping W." deserves a dev.to profile that matches the voice. Per-persona API keys require per-persona secret storage; `.env` does not scale to this without sprawl.
3. **Future skills inherit the mess.** ainews is spec'd against YAML-backed persona-skill. Pivoting to Turso now avoids writing a second skill on top of a backend we're about to replace.

One Turso DB is the least complex answer that covers all three.

## 3. Scope of change

| Component | Change |
|---|---|
| `lib/persona_history.py` | Drop `file://` fallback. Rename env to `PERSONA_REGISTRY_DB_URL` / `PERSONA_REGISTRY_DB_TOKEN`; alias the old names for one version with a deprecation warning. |
| `lib/persona_registry.py` | **New.** Typed Persona dataclass, `upsert`/`get`/`list_slugs`/`delete`, secret ops (`set_secret`/`get_secret`/`rotate_secret`/`list_secret_kinds`), whitelist-based `persona_hash`, keyed `schema_version` table for migrations (Turso rejects `PRAGMA user_version` writes). |
| `persona-skill/scripts/persona_skill.py` | Rewritten as a thin CLI over `lib/persona_registry`. Subcommands: `get`, `list`, `upsert <file.yaml>`, `set-secret <slug> <kind>` (reads stdin), `list-secrets <slug>`, `delete <slug>`, `migrate-from-yaml <dir>`. `validate` becomes YAML-only (no DB round-trip). |
| `persona-skill/personas/*.yaml` | Move canonical copies into `~/a/claw-skills/persona-skill/personas/` under version control. Delete from install path (`~/.nullclaw/skills/persona-skill/personas/`). |
| `persona-skill/SKILL.md` | Tagline + data contract updated. Describes the CLI unchanged; backend described as Turso. |
| `mindfulness-spirit/scripts/run.py` | Replace `_fetch_from_persona_skill` subprocess with direct `persona_registry.get(conn, slug)` import. Read dev.to key via `persona_registry.get_secret(slug, "devto_api_key")`, fall back to `DEV_TO_API_KEY` env with a warning for back-compat. |
| `ainews DESIGN.md` | v0.3 pass (not part of this spec, deferred until this one lands). §2 persona-skill description + §6 `writer_hash` provenance semantics updated. |

## 4. Database

**Name:** `persona-registry` (single Turso DB).

**Auth:** one token. Env vars:

- `PERSONA_REGISTRY_DB_URL` — libsql URL.
- `PERSONA_REGISTRY_DB_TOKEN` — auth token.

Both must be set at runtime. Missing either → `MissingCredentialsError` raised by the lib. `:memory:` URLs bypass the token check (for tests).

**Deprecation alias:** `PERSONA_HISTORY_DB_URL` + `PERSONA_HISTORY_DB_TOKEN` continue to work for one version. When used, `lib/persona_history.py` emits a single `[deprecation]` warning to stderr per process and the new names are tried first. Remove after mindfulness-spirit runs cleanly on new names for one week.

**Backup:** `turso db shell persona-registry .dump > backup.sql`. Not a runtime concern of the library; an operator action.

## 5. Schema

Three tables, all in the same DB. Migrations applied via a keyed `schema_version(name TEXT PRIMARY KEY, version INTEGER)` table — one row per module (`persona_history`, `persona_registry`) so they can share the DB without clobbering each other's version. PRAGMA user_version would be natural on sqlite but Turso's Hrana protocol rejects writes to it.

```sql
-- v1: persona registry
CREATE TABLE IF NOT EXISTS persona (
  slug          TEXT PRIMARY KEY,
  role          TEXT NOT NULL,
  name          TEXT,
  expression    TEXT,
  mental_models TEXT,
  heuristics    TEXT,
  antipatterns  TEXT,
  limits        TEXT,
  hash_version  INTEGER NOT NULL DEFAULT 1,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- v1: per-persona secrets (kind: 'devto_api_key' today; future: 'medium_token', 'mastodon_token')
CREATE TABLE IF NOT EXISTS persona_secret (
  persona_slug  TEXT NOT NULL,
  kind          TEXT NOT NULL,
  value         TEXT NOT NULL,
  rotated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  PRIMARY KEY (persona_slug, kind),
  FOREIGN KEY (persona_slug) REFERENCES persona(slug) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_ps_kind ON persona_secret(kind);

-- v1 (already shipped, unchanged): article history
-- See lib/persona_history.py — the migration that created persona_history is at
-- version 1. Tracked in the keyed schema_version table, keyed by `persona_history`
-- (persona_registry lives alongside it in the same DB, keyed by `persona_registry`).
```

**Design notes:**

- No FK from `persona_history.persona_slug` to `persona.slug`. History is append-only; deleting a persona must not cascade to their history. A deleted persona's `persona_history` rows survive with `persona_slug` as an orphaned string.
- `hash_version` on `persona`, not on `persona_history`. The hash *algorithm* is the thing that versions, and `persona_history.writer_hash` already records which hash was computed — the version lives alongside the persona row so future lookups can recompute.
- No JSON columns. Every identity field is typed. A new field (e.g. `voice_tone`) is a migration step.

## 6. `persona_hash` whitelist

**Whitelist (hash_version = 1):** `slug`, `role`, `name`, `expression`, `mental_models`, `heuristics`, `antipatterns`, `limits`.

**Excluded:** timestamps, `hash_version`, every field in `persona_secret`.

```python
HASH_WHITELIST_V1 = (
    "slug", "role", "name",
    "expression", "mental_models", "heuristics", "antipatterns", "limits",
)

def persona_hash(persona: Persona) -> str:
    if persona.hash_version != 1:
        raise ValueError(f"no hasher for hash_version {persona.hash_version}")
    payload = {k: getattr(persona, k) for k in HASH_WHITELIST_V1}
    canonical = json.dumps(payload, sort_keys=True, ensure_ascii=False)
    return "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()[:16]
```

**Pre-pivot history compatibility:** `persona_history` rows written before the pivot used `persona_hash(persona: dict)` that hashed the *whole resolved dict* including fields that aren't in the whitelist (`source`, `raw`, etc. from mindfulness-spirit's `resolve_persona`). Those rows' `writer_hash` values are incomparable to post-pivot hashes. **Accepted.** One day of history. The module docstring records the pivot date. A future `hash_version = 2` that adds a field goes the same way.

## 7. API — `lib/persona_registry.py`

```python
from dataclasses import dataclass
from typing import Optional

@dataclass(frozen=True)
class Persona:
    slug: str
    role: str
    name: Optional[str]
    expression: Optional[str]
    mental_models: Optional[str]
    heuristics: Optional[str]
    antipatterns: Optional[str]
    limits: Optional[str]
    hash_version: int

class MissingCredentialsError(RuntimeError): ...
class PersonaNotFound(KeyError): ...

# Connection
def connect(database_url: str, auth_token: Optional[str] = None)
def connect_from_env()                        # raises MissingCredentialsError if unset
def ensure_schema(conn) -> None               # idempotent; walks keyed schema_version table

# Profile
def upsert(conn, persona: Persona) -> None    # single transaction; updates updated_at
def get(conn, slug: str) -> Persona           # raises PersonaNotFound
def list_slugs(conn) -> list[str]
def delete(conn, slug: str) -> None           # cascades persona_secret; DOES NOT touch persona_history

# Secrets
def set_secret(conn, slug: str, kind: str, value: str) -> None    # rotates rotated_at
def get_secret(conn, slug: str, kind: str) -> Optional[str]
def rotate_secret(conn, slug: str, kind: str, new_value: str) -> None    # alias for set_secret
def list_secret_kinds(conn, slug: str) -> list[str]               # names only, never values
def delete_secret(conn, slug: str, kind: str) -> None

# Hashing
HASH_WHITELIST_V1: tuple[str, ...]
def persona_hash(persona: Persona) -> str
```

**Transactional upsert:** When the CLI's `upsert` command parses YAML containing both profile fields and (eventually) a request to rotate a secret, both writes go in one transaction. The library's `upsert(conn, persona)` handles only the profile; the CLI orchestrates the full operation and passes the conn through so `BEGIN`/`COMMIT` wraps both.

**`PersonaNotFound` vs. exit 2:** Lib raises a typed exception; the CLI converts it to exit code 2 (matching today's `persona-skill` contract).

## 8. CLI — `persona-skill`

Rewritten as thin wrapper over `lib/persona_registry`. Same exit-code contract as today (`0` success, `1` I/O/unexpected, `2` unknown slug, `3` validation error).

```
persona-skill get <slug>                       # stdout: JSON profile dict (NOT secrets)
persona-skill list                             # stdout: JSON array of slugs
persona-skill list-secrets <slug>              # stdout: JSON array of kinds (not values)
persona-skill upsert <file.yaml>               # reads YAML, validates, writes persona row
persona-skill set-secret <slug> <kind>         # reads value from stdin (never argv), writes persona_secret row
persona-skill delete <slug>                    # cascades to persona_secret, preserves persona_history
persona-skill validate [<file.yaml>]           # YAML-only, no DB access
persona-skill migrate-from-yaml <dir>          # one-shot bulk upsert during pivot
```

**`get` never returns secrets.** Callers that need a secret call the lib's `get_secret` directly, or a future `persona-skill get-secret <slug> <kind>` that writes the raw value to stdout only when stdout is not a tty (guard against shoulder-surfing).

**`set-secret` reads stdin.** This is non-negotiable. Never accept a secret as an argv argument — it would land in shell history, `ps` output, or agent transcripts. Invocation:

```bash
echo "$DEV_TO_API_KEY_PING_W" | persona-skill set-secret ping-w devto_api_key
# or interactively:
persona-skill set-secret ping-w devto_api_key   # prompts without echo when stdin is a tty
```

**`upsert` YAML contract:**

```yaml
# ping-w.yaml
slug: ping-w
role: 在宗教界服務的心行者
name: Ping W.
expression: |
  語氣沉穩、不急不徐。
  偏好具體意象而非抽象名詞。
# NO `secrets:` key allowed. Parser rejects.
```

The `_validate_persona` parser rejects unknown top-level keys (existing behavior, §6 of persona-skill's current validation) — the `secrets:` key is rejected by that exact rule. Explicit test coverage added to make the intent load-bearing, not incidental.

## 9. Migration

One-shot sequence, run manually in order:

1. `turso db create persona-registry`
2. `turso db tokens create persona-registry` → new auth token
3. Add `PERSONA_REGISTRY_DB_URL` and `PERSONA_REGISTRY_DB_TOKEN` to `~/.nullclaw/.env`.
4. Implement `lib/persona_registry.py` + tests. `ensure_schema` runs v1 migration (creates `persona`, `persona_secret`, leaves `persona_history` for the next step).
5. Data-move `persona_history` rows from the existing `persona-history` Turso DB to `persona-registry`:
   ```
   turso db shell persona-history ".dump persona_history" > /tmp/ph.sql
   turso db shell persona-registry < /tmp/ph.sql
   ```
   Verify row counts match, then drop `persona-history` DB.
6. Run `persona-skill migrate-from-yaml ~/a/claw-skills/persona-skill/personas/`. Three upserts (default, ping-w, skeptical-editor). Idempotent.
7. Delete `~/.nullclaw/skills/persona-skill/personas/` YAML files. Canonical copies remain in `~/a/claw-skills/persona-skill/personas/`, git-tracked.
8. For each persona that should publish under its own dev.to account:
   ```
   echo "<key>" | persona-skill set-secret <slug> devto_api_key
   ```
9. Update mindfulness-spirit:
   - Replace `_fetch_from_persona_skill(slug)` subprocess with `persona_registry.get(conn, slug)`.
   - Add `resolve_devto_key(persona_slug, config)` that tries `persona_registry.get_secret(slug, "devto_api_key")` first, then `DEV_TO_API_KEY` env, then `config.skills.dev_to_api_key`.
10. Run the skill end-to-end once with `--dry-run` to confirm writer prompt composes, persona resolves, history query returns.
11. One live run in production.
12. After one week of clean runs, remove the `PERSONA_HISTORY_DB_URL` deprecation alias from `lib/persona_history.py`.

**Rollback:** The YAML files are still in the git repo. `persona-skill migrate-from-yaml` is idempotent. If the pivot goes wrong, a `git revert` of the code and a re-run of the old YAML-backed persona-skill gets us back. The Turso DB rows stay; they just aren't read.

## 10. Failure policy

Library: raises typed exceptions, decides no policy.

- `MissingCredentialsError` — env vars missing.
- `PersonaNotFound` — unknown slug.
- `sqlite3.OperationalError` / libsql errors — connection/query failures, propagate to caller.

Caller skills:

- **mindfulness-spirit (lenient, unchanged ethos):** catches `MissingCredentialsError` and `PersonaNotFound`, warns to stderr, falls through to `DEFAULT_PERSONA_ROLE` + shared `DEV_TO_API_KEY` + empty history. Skill still runs.
- **ainews (strict, per DESIGN v0.2 §11):** any of the above → hard fail. No silent fallback.
- **persona-skill CLI:** maps `MissingCredentialsError` → exit 1; `PersonaNotFound` → exit 2; validation → exit 3.

## 11. Testing

`lib/test_persona_registry.py` mirrors `lib/test_persona_history.py`:

- `:memory:` connection for all tests.
- `ensure_schema` idempotent across repeated calls.
- `ensure_schema` bumps the module's row in `schema_version` to current and skips on rerun.
- `upsert` + `get` round-trip all fields.
- `upsert` on existing slug updates `updated_at` without changing `created_at`.
- `delete` cascades to `persona_secret` but leaves `persona_history` untouched.
- `set_secret` + `get_secret` round-trip; `rotate_secret` updates `rotated_at`.
- `list_secret_kinds` returns names only, never values.
- `persona_hash` whitelist: adding a non-whitelisted attribute to the Persona doesn't change the hash.
- `persona_hash` is stable across Python runs (sort_keys).
- `connect_from_env` raises `MissingCredentialsError` when either env var is unset.
- CLI: `set-secret` rejects a value passed as argv (test by invoking with value-on-argv and asserting exit 3).
- CLI: `upsert` rejects YAML with a `secrets:` key.

`persona-skill/scripts/test_persona_skill.py` (existing) updated:
- YAML tests stay as YAML parse/validation tests.
- DB-dependent tests use a tempfile-backed `:memory:` connection injected via `PERSONA_REGISTRY_DB_URL=:memory:`.
- Subprocess CLI tests use a throwaway Turso URL pointing at `:memory:` via env injection.

No end-to-end test against real Turso. Integration verification is a single `persona-skill upsert && persona-skill get` manual run after step 6 of §9.

## 12. Concurrency

- **Multi-host writes.** Two machines running `persona-skill upsert ping-w.yaml` simultaneously: libsql's server picks a winner deterministically; both commands succeed from the client's perspective; the losing write is simply the older `updated_at`. Acceptable — operator, not production app.
- **SQLite `:memory:` in tests.** Single-threaded, no concern.
- **History writes during a persona upsert.** Safe by design: `persona_history` writes do not touch `persona` or `persona_secret`. Two distinct row-sets, no locking interaction.

## 13. Security considerations

- **Blast radius of `PERSONA_REGISTRY_DB_TOKEN` leak.** Every writer persona's dev.to API key is readable. Mitigations:
  - Keep the token out of shell history: `~/.nullclaw/.env` with mode `0600`.
  - Rotate on any suspected exposure: `turso db tokens rotate persona-registry` → update `.env`.
  - Do not embed in CI/CD unless scoped with short TTL.
- **`persona-skill set-secret` stdin-only contract is load-bearing.** A developer typing the command with the secret as an argv argument burns the key. Test coverage for the "argv rejection" path prevents regression.
- **YAML files in git.** Profile only. Parser enforces no `secrets:` key. A pre-commit hook (optional, not blocking) could grep for likely secret tokens as defense-in-depth.
- **`get` never returns secrets.** Accidentally dumping a persona dict to a log does not leak keys.

## 14. Out of scope

- Migrating oilcon to the consolidated DB. Oil prices are time-series, unrelated. Stays separate.
- Web UI for persona editing. CLI + YAML + editor is sufficient.
- Multi-tenant support (multiple humans sharing one `persona-registry`). Single-operator project.
- Persona versioning beyond `hash_version`. If a persona's identity changes meaningfully, operator bumps `hash_version` and future `persona_history` rows record the new hash. No per-field history table.
- Automatic YAML-to-DB sync (pre-commit hook or file watcher). Deliberately rejected: explicit `persona-skill upsert` is the operator gesture that says "apply this change," not a side effect of saving a file.
- Article-body storage in Turso. Bodies live on dev.to; local copies in `runs/` and `failed/` are for debugging.

## 15. What "done" looks like

- [ ] `lib/persona_registry.py` + tests land, all pass.
- [ ] `persona-skill` CLI rewritten, all existing CLI tests pass against the new backend.
- [ ] Turso `persona-registry` DB exists; env vars set; schema at v1.
- [ ] `persona_history` rows migrated from `persona-history` DB; row count verified; old DB dropped.
- [ ] Three YAML files upserted; YAML files deleted from install path; canonical copies remain in repo.
- [ ] At least one persona has a `devto_api_key` secret set.
- [ ] mindfulness-spirit runs clean end-to-end reading persona + secret from Turso.
- [ ] `PERSONA_HISTORY_DB_URL` alias still works; deprecation warning fires once per run.
- [ ] Documentation updated: `persona-skill/SKILL.md` tagline, `CLAUDE.md` env var section, ainews DESIGN.md v0.3 (deferred, tracked separately).

## 16. Open questions

None at draft time. All prior questions resolved:
- Local file fallback → dropped.
- DB topology → one shared DB.
- Schema shape → typed columns, no JSON.
- YAML role → input-only, git-tracked, not read at runtime.
- Secrets in YAML → forbidden at parser level.
- Hash compatibility → accept stale, document pivot date.
- Direct lib import vs. subprocess CLI → both supported; in-process skills use lib, humans use CLI.

If any surface during implementation, implementer pauses and asks rather than guesses.

End of document.
