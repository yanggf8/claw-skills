---
name: persona-skill
description: Writer-persona registry backed by Turso — stores personas and per-persona secrets, serves them via CLI
version: 0.3.0
author: yanggf
always: false
requires_bins:
  - python3
requires_env:
  - PERSONA_REGISTRY_DB_URL
  - PERSONA_REGISTRY_DB_TOKEN
---

# persona-skill

Writer-persona registry backed by a single Turso (libsql) database. Other skills (e.g. `mindfulness-spirit`, `ainews`) import `lib/persona_registry` directly to fetch personas and per-persona secrets. This CLI is the admin interface — upsert from YAML, rotate secrets, list slugs.

YAML files in `personas/` are the version-controlled input format. Turso is the runtime source of truth. Secrets (e.g. dev.to API keys) live in `persona_secret` rows and are only ever set via stdin — never argv.

## CLI Usage

```bash
# Get a persona (JSON, no secrets)
python3 scripts/persona_skill.py get ping-w

# List all slugs
python3 scripts/persona_skill.py list

# Create a persona directly (no YAML file needed)
python3 scripts/persona_skill.py create new-writer --role "科技記者" --name "New Writer"

# Update specific fields (unspecified fields keep their current value)
python3 scripts/persona_skill.py update ping-w --name "Ping W. v2" --expression "更銳利的語氣"

# Upsert one persona from YAML (legacy; prefer create/update)
python3 scripts/persona_skill.py upsert personas/ping-w.yaml

# Bulk upsert from a directory (idempotent)
python3 scripts/persona_skill.py migrate-from-yaml personas/

# Validate YAML without touching the DB
python3 scripts/persona_skill.py validate
python3 scripts/persona_skill.py validate personas/ping-w.yaml

# Set a per-persona secret — value read from stdin only
echo "$DEV_TO_API_KEY_PING_W" | python3 scripts/persona_skill.py set-secret ping-w devto_api_key

# Verify a secret exists (masked by default; --reveal requires interactive TTY)
python3 scripts/persona_skill.py get-secret ping-w devto_api_key
python3 scripts/persona_skill.py get-secret ping-w devto_api_key --reveal

# List secret kinds for a persona (never prints values)
python3 scripts/persona_skill.py list-secrets ping-w

# Delete a single secret kind
python3 scripts/persona_skill.py delete-secret ping-w old_api_key

# Delete a persona (cascades secrets; history rows are kept)
python3 scripts/persona_skill.py delete ping-w

# View recent publish history
python3 scripts/persona_skill.py history --skill mindfulness-spirit
python3 scripts/persona_skill.py history --persona ping-w --limit 5 --json
```

### Exit Codes

| Code | Meaning |
|------|---------|
| 0    | Success |
| 1    | General error (I/O, missing credentials, unexpected) |
| 2    | Unknown slug |
| 3    | Validation error (YAML, empty secret value, forbidden keys) |

## Environment

| Variable | Required | Purpose |
|----------|----------|---------|
| `PERSONA_REGISTRY_DB_URL` | yes | libsql URL. Use `:memory:` for tests. |
| `PERSONA_REGISTRY_DB_TOKEN` | yes (remote) | Turso auth token. Not required when URL is `:memory:`. |

## Data Contract

Every persona is a flat record with these fields:

```json
{
  "slug": "ping-w",
  "role": "在宗教界服務的心行者",
  "name": "Ping W.",
  "expression": null,
  "mental_models": null,
  "heuristics": null,
  "antipatterns": null,
  "limits": null
}
```

### Field Reference

| Field | Type | Required | Purpose |
|-------|------|----------|---------|
| `slug` | string | yes | Stable identifier. Kebab-case. Matches filename stem. |
| `role` | string | yes | Short descriptor for LLM prompts (`你是{role}…`). |
| `name` | string \| null | no | Byline / signature text. |
| `expression` | string \| null | no | 表達 DNA — 語氣、節奏、用詞偏好。 |
| `mental_models` | string \| null | no | 認知框架。 |
| `heuristics` | string \| null | no | 決策啟發式。 |
| `antipatterns` | string \| null | no | 反模式、不會做的事。 |
| `limits` | string \| null | no | 誠實邊界。 |

### Validation Rules

- `slug` must match `^[a-z][a-z0-9-]*$` and equal the filename stem.
- `role` must be a non-empty string after strip.
- `secrets` key is forbidden in YAML — use `set-secret` CLI.
- Unknown top-level keys are rejected (fail-fast).
- Optional fields omitted in YAML default to `null`.

## Secrets

Per-persona secrets live in the `persona_secret` table, keyed by `(slug, kind)`. They are **only** writable via `set-secret`, which reads the value from stdin. Argv is never accepted — a secret on argv would land in shell history, `ps` output, or agent transcripts.

```bash
# Piped (non-interactive)
echo "<key>" | python3 scripts/persona_skill.py set-secret ping-w devto_api_key

# Interactive prompt (no echo)
python3 scripts/persona_skill.py set-secret ping-w devto_api_key
```

`get` never returns secrets. `list-secrets` returns kinds only, never values. `get-secret` shows masked output (length + prefix) by default; `--reveal` prints the full value but is blocked when stdout is not an interactive TTY (prevents leakage into logs, agent transcripts, or CI).

## Library API

Consumer skills should import `lib/persona_registry` directly instead of shelling out:

```python
import persona_registry

conn = persona_registry.connect_from_env()
persona_registry.ensure_schema(conn)

persona = persona_registry.get(conn, "ping-w")              # → Persona dataclass
devto_key = persona_registry.get_secret(conn, "ping-w", "devto_api_key")  # → str | None
```

## Dependencies

- Python 3.9+
- pyyaml (YAML input format)
- libsql-experimental (Turso client; not required when `PERSONA_REGISTRY_DB_URL=:memory:`)
