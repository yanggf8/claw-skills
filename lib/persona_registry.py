"""Persona registry backed by Turso (libsql).

Stores persona profiles and per-persona secrets (dev.to API keys, etc.)
in a shared Turso DB alongside persona_history. One DB, one token.

Schema versioning via the `schema_version` table, same pattern as
persona_history (PRAGMA user_version is rejected by Turso/Hrana for writes).
This module owns the `persona` and `persona_secret` tables. The
`persona_history` table is owned by persona_history.py and shares the DB.

Policy (strict vs. lenient) lives in the caller, not here.
"""
import hashlib
import json
import os
import sqlite3
from dataclasses import dataclass
from typing import Optional

try:
    import libsql_experimental as libsql
except ImportError:  # pragma: no cover - optional dependency
    libsql = None


SCHEMA_VERSION = 1

DB_URL_ENV = "PERSONA_REGISTRY_DB_URL"
DB_TOKEN_ENV = "PERSONA_REGISTRY_DB_TOKEN"

SQLITE_BUSY_TIMEOUT_SECONDS = 20

HASH_WHITELIST_V1 = (
    "slug", "role", "name",
    "expression", "mental_models", "heuristics", "antipatterns", "limits",
)


class MissingCredentialsError(RuntimeError):
    pass


class PersonaNotFound(KeyError):
    pass


@dataclass(frozen=True)
class Persona:
    slug: str
    role: str
    name: Optional[str] = None
    expression: Optional[str] = None
    mental_models: Optional[str] = None
    heuristics: Optional[str] = None
    antipatterns: Optional[str] = None
    limits: Optional[str] = None
    hash_version: int = 1  # kept for legacy hash compatibility


def connect(database_url: str, auth_token: Optional[str] = None):
    if database_url == ":memory:":
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(f"PRAGMA busy_timeout = {SQLITE_BUSY_TIMEOUT_SECONDS * 1000}")
        conn.execute("PRAGMA foreign_keys = ON")
        return conn

    if libsql is None:
        raise RuntimeError("libsql-experimental not installed")
    conn = libsql.connect(database_url, auth_token=auth_token)
    conn.execute("PRAGMA foreign_keys = ON")
    return conn


def connect_from_env():
    database_url = os.environ.get(DB_URL_ENV)
    auth_token = os.environ.get(DB_TOKEN_ENV)

    if not database_url:
        raise MissingCredentialsError(
            f"{DB_URL_ENV} must be set"
        )
    if database_url != ":memory:" and not auth_token:
        raise MissingCredentialsError(
            f"{DB_TOKEN_ENV} required when {DB_URL_ENV} points at a remote DB"
        )
    return connect(database_url, auth_token=auth_token)


SCHEMA_NAME = "persona_registry"


def _ensure_schema_version_table(conn) -> None:
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version ("
        "  name TEXT PRIMARY KEY,"
        "  version INTEGER NOT NULL"
        ")"
    )


def _current_schema_version(conn) -> int:
    _ensure_schema_version_table(conn)
    row = conn.execute(
        "SELECT version FROM schema_version WHERE name = ?",
        (SCHEMA_NAME,),
    ).fetchone()
    if row is None:
        return 0
    return int(row[0])


def _set_schema_version(conn, version: int) -> None:
    conn.execute(
        "INSERT INTO schema_version (name, version) VALUES (?, ?) "
        "ON CONFLICT(name) DO UPDATE SET version = excluded.version",
        (SCHEMA_NAME, int(version)),
    )


def _migrate_to_v1(conn) -> None:
    conn.execute(
        """
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
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS persona_secret (
          persona_slug  TEXT NOT NULL,
          kind          TEXT NOT NULL,
          value         TEXT NOT NULL,
          rotated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
          PRIMARY KEY (persona_slug, kind),
          FOREIGN KEY (persona_slug) REFERENCES persona(slug) ON DELETE CASCADE
        )
        """
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ps_kind ON persona_secret(kind)"
    )


MIGRATIONS = [
    (1, _migrate_to_v1),
]


def _has_webapp_schema(conn) -> bool:
    """Return True if the DB was created by the webapp seed (has user_id on persona)."""
    row = conn.execute(
        "SELECT COUNT(*) FROM pragma_table_info('persona') WHERE name='user_id'"
    ).fetchone()
    return bool(row and row[0])


def ensure_schema(conn) -> None:
    # Webapp seed already applied a superset schema — skip migrations to avoid conflict.
    if _has_webapp_schema(conn):
        return
    current = _current_schema_version(conn)
    for version, migrate in MIGRATIONS:
        if version <= current:
            continue
        migrate(conn)
        _set_schema_version(conn, version)
    conn.commit()


def upsert(conn, persona: Persona) -> None:
    if _has_webapp_schema(conn):
        # Webapp schema: no hash_version, has persona_updated_at
        conn.execute(
            """
            INSERT INTO persona (slug, role, name, expression, mental_models,
                                 heuristics, antipatterns, limits, persona_updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT(slug) DO UPDATE SET
              role = excluded.role,
              name = excluded.name,
              expression = excluded.expression,
              mental_models = excluded.mental_models,
              heuristics = excluded.heuristics,
              antipatterns = excluded.antipatterns,
              limits = excluded.limits,
              preview_text = NULL,
              persona_updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            """,
            (persona.slug, persona.role, persona.name, persona.expression,
             persona.mental_models, persona.heuristics, persona.antipatterns,
             persona.limits),
        )
    else:
        conn.execute(
            """
            INSERT INTO persona (slug, role, name, expression, mental_models,
                                 heuristics, antipatterns, limits, hash_version)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(slug) DO UPDATE SET
              role = excluded.role,
              name = excluded.name,
              expression = excluded.expression,
              mental_models = excluded.mental_models,
              heuristics = excluded.heuristics,
              antipatterns = excluded.antipatterns,
              limits = excluded.limits,
              hash_version = excluded.hash_version,
              updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            """,
            (persona.slug, persona.role, persona.name, persona.expression,
             persona.mental_models, persona.heuristics, persona.antipatterns,
             persona.limits, persona.hash_version),
        )


def get(conn, slug: str) -> Persona:
    row = conn.execute(
        """
        SELECT slug, role, name, expression, mental_models,
               heuristics, antipatterns, limits
        FROM persona WHERE slug = ?
        """,
        (slug,),
    ).fetchone()
    if row is None:
        raise PersonaNotFound(slug)
    return Persona(
        slug=row[0],
        role=row[1],
        name=row[2],
        expression=row[3],
        mental_models=row[4],
        heuristics=row[5],
        antipatterns=row[6],
        limits=row[7],
    )


def list_slugs(conn) -> list[str]:
    rows = conn.execute("SELECT slug FROM persona ORDER BY slug").fetchall()
    return [row[0] for row in rows]


def delete(conn, slug: str) -> None:
    conn.execute("DELETE FROM persona WHERE slug = ?", (slug,))


def set_secret(conn, slug: str, kind: str, value: str) -> None:
    conn.execute(
        """
        INSERT INTO persona_secret (persona_slug, kind, value)
        VALUES (?, ?, ?)
        ON CONFLICT(persona_slug, kind) DO UPDATE SET
          value = excluded.value,
          rotated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        """,
        (slug, kind, value),
    )


def get_secret(conn, slug: str, kind: str) -> Optional[str]:
    row = conn.execute(
        "SELECT value FROM persona_secret WHERE persona_slug = ? AND kind = ?",
        (slug, kind),
    ).fetchone()
    if row is None:
        return None
    return row[0]


def rotate_secret(conn, slug: str, kind: str, new_value: str) -> None:
    set_secret(conn, slug, kind, new_value)


def list_secret_kinds(conn, slug: str) -> list[str]:
    rows = conn.execute(
        "SELECT kind FROM persona_secret WHERE persona_slug = ? ORDER BY kind",
        (slug,),
    ).fetchall()
    return [row[0] for row in rows]


def delete_secret(conn, slug: str, kind: str) -> None:
    conn.execute(
        "DELETE FROM persona_secret WHERE persona_slug = ? AND kind = ?",
        (slug, kind),
    )


def persona_hash(persona: Persona) -> str:
    if persona.hash_version != 1:
        raise ValueError(f"no hasher for hash_version {persona.hash_version}")
    payload = {k: getattr(persona, k) for k in HASH_WHITELIST_V1}
    canonical = json.dumps(payload, sort_keys=True, ensure_ascii=False)
    return "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()[:16]
