"""Per-persona article history, shared by mindfulness-spirit and ainews.

Append-only log of what each writer persona has produced, across skills.
Consumers pull `recent(persona_slug=...)` rows into writer prompts so drafts
don't repeat themselves.

Schema versioning is explicit — `ensure_schema` walks the `schema_version`
table from 0 up to SCHEMA_VERSION, applying each migration in order. Adding
a column later means appending a new migration, never editing an existing
one. (PRAGMA user_version would be the natural choice on sqlite, but Turso's
Hrana protocol rejects writes to it.)

Policy (strict vs. lenient fallback) lives in the caller, not here. The
library raises `MissingCredentialsError` / `sqlite3.OperationalError` etc.
and lets the caller decide whether to degrade or fail.

Pivot 2026-04-16: env vars renamed from PERSONA_HISTORY_DB_* to
PERSONA_REGISTRY_DB_*. Deprecated aliases removed 2026-04-17.
file:// fallback removed — Turso-only at runtime, :memory: for tests.
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


SCHEMA_VERSION = 2

DB_URL_ENV = "PERSONA_REGISTRY_DB_URL"
DB_TOKEN_ENV = "PERSONA_REGISTRY_DB_TOKEN"

SQLITE_BUSY_TIMEOUT_SECONDS = 20


class MissingCredentialsError(RuntimeError):
    pass


@dataclass(frozen=True)
class HistoryRow:
    """Typed view of a persona_history row as returned by `recent()`."""
    id: int
    skill: str
    stream: str
    persona_slug: str
    editor_slug: Optional[str]
    date: str
    title: str
    stance: str
    key_links: list[str]
    draft_sha: Optional[str]
    git_dirty: bool
    writer_hash: str
    editor_hash: Optional[str]
    devto_id: Optional[int]
    devto_url: Optional[str]
    created_at: str


@dataclass(frozen=True)
class PlanRow:
    """A long-lived series plan. `month` is the starting month, not a
    per-month constraint — UNIQUE(skill, series_slug) allows only one
    plan per series regardless of month."""
    id: int
    skill: str
    series_slug: str
    month: str
    series_title: str
    series_theme: Optional[str]
    created_at: str


@dataclass(frozen=True)
class TopicRow:
    """A single topic within a series plan."""
    id: int
    plan_id: int
    week: int
    target_date: str
    title_hint: str
    angle: str
    lens: str
    direction: str
    key_question: Optional[str]
    status: str
    history_id: Optional[int]
    created_at: str


def connect(database_url: str, auth_token: Optional[str] = None):
    if database_url == ":memory:":
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(f"PRAGMA busy_timeout = {SQLITE_BUSY_TIMEOUT_SECONDS * 1000}")
        conn.execute("PRAGMA foreign_keys = ON")
        return conn

    if libsql is None:
        raise RuntimeError("libsql-experimental not installed")
    return libsql.connect(database_url, auth_token=auth_token)


def connect_from_env():
    database_url = os.environ.get(DB_URL_ENV)
    auth_token = os.environ.get(DB_TOKEN_ENV)

    if not database_url:
        raise MissingCredentialsError(f"{DB_URL_ENV} must be set")
    if database_url != ":memory:" and not auth_token:
        raise MissingCredentialsError(
            f"{DB_TOKEN_ENV} required when {DB_URL_ENV} points at a remote DB"
        )
    return connect(database_url, auth_token=auth_token)


SCHEMA_NAME = "persona_history"


def _ensure_schema_version_table(conn) -> None:
    # PRAGMA user_version works on sqlite but Turso/Hrana rejects writes to it,
    # so version tracking lives in a keyed table. Keyed by name so multiple
    # modules can share the same DB (persona_history + persona_registry).
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
        CREATE TABLE IF NOT EXISTS persona_history (
          id              INTEGER PRIMARY KEY AUTOINCREMENT,
          skill           TEXT    NOT NULL,
          stream          TEXT    NOT NULL,
          persona_slug    TEXT    NOT NULL,
          editor_slug     TEXT,
          date            TEXT    NOT NULL,
          title           TEXT    NOT NULL,
          stance          TEXT    NOT NULL,
          key_links       TEXT    NOT NULL,
          draft_sha       TEXT,
          git_dirty       INTEGER NOT NULL DEFAULT 0,
          writer_hash     TEXT    NOT NULL,
          editor_hash     TEXT,
          devto_id        INTEGER,
          devto_url       TEXT,
          created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        )
        """
    )
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_ph_persona_date_id
          ON persona_history(persona_slug, date DESC, id DESC)
        """
    )
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_ph_skill_stream_date_id
          ON persona_history(skill, stream, date DESC, id DESC)
        """
    )


def _migrate_to_v2(conn) -> None:
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS editorial_plan (
          id            INTEGER PRIMARY KEY AUTOINCREMENT,
          skill         TEXT    NOT NULL,
          series_slug   TEXT    NOT NULL,
          month         TEXT    NOT NULL,
          series_title  TEXT    NOT NULL,
          series_theme  TEXT,
          created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
          UNIQUE(skill, series_slug)
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS editorial_topic (
          id            INTEGER PRIMARY KEY AUTOINCREMENT,
          plan_id       INTEGER NOT NULL REFERENCES editorial_plan(id),
          week          INTEGER NOT NULL,
          target_date   TEXT    NOT NULL,
          title_hint    TEXT    NOT NULL,
          angle         TEXT    NOT NULL,
          lens          TEXT    NOT NULL,
          direction     TEXT    NOT NULL DEFAULT 'tech-first',
          key_question  TEXT,
          status        TEXT    NOT NULL DEFAULT 'planned',
          history_id    INTEGER REFERENCES persona_history(id),
          created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        )
        """
    )
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_et_plan_week
          ON editorial_topic(plan_id, week)
        """
    )
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_et_status
          ON editorial_topic(status, target_date)
        """
    )


MIGRATIONS = [
    (1, _migrate_to_v1),
    (2, _migrate_to_v2),
]


def ensure_schema(conn) -> None:
    """Apply any pending migrations. Idempotent.

    Intended to be called once per process at startup, not per connection —
    each call does a `schema_version` round-trip.
    """
    current = _current_schema_version(conn)
    for version, migrate in MIGRATIONS:
        if version <= current:
            continue
        migrate(conn)
        _set_schema_version(conn, version)
    conn.commit()


def persona_hash(persona: dict) -> str:
    """Stable hash of a resolved persona dict. `sha256:<16 hex chars>`.

    Not a security hash — identifies which persona state produced a draft.
    Sort keys + ensure_ascii=False so hash is stable across Python runs.
    """
    canonical = json.dumps(persona, sort_keys=True, ensure_ascii=False)
    digest = hashlib.sha256(canonical.encode("utf-8")).hexdigest()[:16]
    return f"sha256:{digest}"


def record(
    conn,
    *,
    skill: str,
    stream: str,
    persona_slug: str,
    date: str,
    title: str,
    stance: str,
    key_links: list[str],
    writer_hash: str,
    editor_slug: Optional[str] = None,
    editor_hash: Optional[str] = None,
    draft_sha: Optional[str] = None,
    git_dirty: bool = False,
    devto_id: Optional[int] = None,
    devto_url: Optional[str] = None,
) -> int:
    """Insert one history row, return its `id`.

    `key_links` is serialized to JSON inside the library; callers pass a list.
    """
    if not isinstance(key_links, list):
        raise TypeError("key_links must be a list of strings")
    key_links_json = json.dumps(key_links, ensure_ascii=False)
    cursor = conn.execute(
        """
        INSERT INTO persona_history (
          skill, stream, persona_slug, editor_slug,
          date, title, stance, key_links,
          draft_sha, git_dirty, writer_hash, editor_hash,
          devto_id, devto_url
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            skill,
            stream,
            persona_slug,
            editor_slug,
            date,
            title,
            stance,
            key_links_json,
            draft_sha,
            1 if git_dirty else 0,
            writer_hash,
            editor_hash,
            devto_id,
            devto_url,
        ),
    )
    conn.commit()
    row_id = cursor.lastrowid
    if row_id is None:
        # libsql-experimental cursors may not set lastrowid; query directly.
        row = conn.execute("SELECT last_insert_rowid()").fetchone()
        row_id = int(row[0])
    return int(row_id)


def recent(
    conn,
    *,
    persona_slug: Optional[str] = None,
    skill: Optional[str] = None,
    stream: Optional[str] = None,
    limit: int = 8,
) -> list[HistoryRow]:
    """Return history rows ordered newest first.

    At least one of `persona_slug` or `skill` must be provided — no
    unbounded table scans. Ties on `date` broken by `id DESC` so same-day
    retries keep a deterministic newest-first order.
    """
    if persona_slug is None and skill is None:
        raise ValueError("at least one of persona_slug or skill must be set")
    if limit <= 0:
        raise ValueError("limit must be positive")

    clauses = []
    params: list = []
    if persona_slug is not None:
        clauses.append("persona_slug = ?")
        params.append(persona_slug)
    if skill is not None:
        clauses.append("skill = ?")
        params.append(skill)
    if stream is not None:
        clauses.append("stream = ?")
        params.append(stream)
    where = " AND ".join(clauses)
    params.append(int(limit))

    rows = conn.execute(
        f"""
        SELECT id, skill, stream, persona_slug, editor_slug,
               date, title, stance, key_links,
               draft_sha, git_dirty, writer_hash, editor_hash,
               devto_id, devto_url, created_at
        FROM persona_history
        WHERE {where}
        ORDER BY date DESC, id DESC
        LIMIT ?
        """,
        tuple(params),
    ).fetchall()

    return [_row_to_history(row) for row in rows]


def set_devto_result(
    conn,
    row_id: int,
    devto_id: int,
    devto_url: str,
) -> None:
    """Attach dev.to metadata to a previously-recorded row.

    This is the only mutation supported on the table. All other fields are
    append-only.
    """
    conn.execute(
        """
        UPDATE persona_history
        SET devto_id = ?, devto_url = ?
        WHERE id = ?
        """,
        (devto_id, devto_url, row_id),
    )
    conn.commit()


# ---------------------------------------------------------------------------
# Editorial plan CRUD
# ---------------------------------------------------------------------------


def create_plan(
    conn,
    *,
    skill: str,
    series_slug: str,
    month: str,
    series_title: str,
    series_theme: Optional[str] = None,
) -> int:
    cursor = conn.execute(
        """
        INSERT INTO editorial_plan (skill, series_slug, month, series_title, series_theme)
        VALUES (?, ?, ?, ?, ?)
        """,
        (skill, series_slug, month, series_title, series_theme),
    )
    conn.commit()
    row_id = cursor.lastrowid
    if row_id is None:
        row = conn.execute("SELECT last_insert_rowid()").fetchone()
        row_id = int(row[0])
    return int(row_id)


def get_plan(conn, *, skill: str, series_slug: str) -> Optional[PlanRow]:
    row = conn.execute(
        """
        SELECT id, skill, series_slug, month, series_title, series_theme, created_at
        FROM editorial_plan
        WHERE skill = ? AND series_slug = ?
        """,
        (skill, series_slug),
    ).fetchone()
    if row is None:
        return None
    return PlanRow(
        id=int(row[0]), skill=row[1], series_slug=row[2],
        month=row[3], series_title=row[4], series_theme=row[5], created_at=row[6],
    )


def list_plans(conn, *, skill: Optional[str] = None) -> list[PlanRow]:
    if skill:
        rows = conn.execute(
            "SELECT id, skill, series_slug, month, series_title, series_theme, created_at "
            "FROM editorial_plan WHERE skill = ? ORDER BY month DESC",
            (skill,),
        ).fetchall()
    else:
        rows = conn.execute(
            "SELECT id, skill, series_slug, month, series_title, series_theme, created_at "
            "FROM editorial_plan ORDER BY month DESC",
        ).fetchall()
    return [
        PlanRow(id=int(r[0]), skill=r[1], series_slug=r[2],
                month=r[3], series_title=r[4], series_theme=r[5], created_at=r[6])
        for r in rows
    ]


def add_topic(
    conn,
    *,
    plan_id: int,
    week: int,
    target_date: str,
    title_hint: str,
    angle: str,
    lens: str,
    direction: str = "tech-first",
    key_question: Optional[str] = None,
) -> int:
    cursor = conn.execute(
        """
        INSERT INTO editorial_topic
            (plan_id, week, target_date, title_hint, angle, lens, direction, key_question)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (plan_id, week, target_date, title_hint, angle, lens, direction, key_question),
    )
    conn.commit()
    row_id = cursor.lastrowid
    if row_id is None:
        row = conn.execute("SELECT last_insert_rowid()").fetchone()
        row_id = int(row[0])
    return int(row_id)


def list_topics(conn, *, plan_id: int) -> list[TopicRow]:
    rows = conn.execute(
        """
        SELECT id, plan_id, week, target_date, title_hint, angle, lens, direction,
               key_question, status, history_id, created_at
        FROM editorial_topic
        WHERE plan_id = ?
        ORDER BY week
        """,
        (plan_id,),
    ).fetchall()
    return [_row_to_topic(r) for r in rows]


def next_topic(conn, *, plan_id: int) -> Optional[TopicRow]:
    row = conn.execute(
        """
        SELECT id, plan_id, week, target_date, title_hint, angle, lens, direction,
               key_question, status, history_id, created_at
        FROM editorial_topic
        WHERE plan_id = ? AND status = 'planned'
        ORDER BY week
        LIMIT 1
        """,
        (plan_id,),
    ).fetchone()
    if row is None:
        return None
    return _row_to_topic(row)


def mark_topic_published(conn, topic_id: int, history_id: int) -> None:
    cursor = conn.execute(
        "UPDATE editorial_topic SET status = 'published', history_id = ? "
        "WHERE id = ? AND status = 'planned'",
        (history_id, topic_id),
    )
    conn.commit()
    if cursor.rowcount == 0:
        raise ValueError(
            f"topic {topic_id} not updated — either missing or not in 'planned' status"
        )


def mark_topic_skipped(conn, topic_id: int) -> None:
    cursor = conn.execute(
        "UPDATE editorial_topic SET status = 'skipped' "
        "WHERE id = ? AND status = 'planned'",
        (topic_id,),
    )
    conn.commit()
    if cursor.rowcount == 0:
        raise ValueError(
            f"topic {topic_id} not updated — either missing or not in 'planned' status"
        )


def _row_to_topic(row) -> TopicRow:
    return TopicRow(
        id=int(row[0]), plan_id=int(row[1]), week=int(row[2]),
        target_date=row[3], title_hint=row[4], angle=row[5],
        lens=row[6], direction=row[7], key_question=row[8],
        status=row[9], history_id=int(row[10]) if row[10] is not None else None,
        created_at=row[11],
    )


def _row_to_history(row) -> HistoryRow:
    try:
        key_links = json.loads(row[8]) if row[8] else []
    except json.JSONDecodeError:
        key_links = []
    if not isinstance(key_links, list):
        key_links = []
    return HistoryRow(
        id=int(row[0]),
        skill=row[1],
        stream=row[2],
        persona_slug=row[3],
        editor_slug=row[4],
        date=row[5],
        title=row[6],
        stance=row[7],
        key_links=[str(link) for link in key_links],
        draft_sha=row[9],
        git_dirty=bool(row[10]),
        writer_hash=row[11],
        editor_hash=row[12],
        devto_id=int(row[13]) if row[13] is not None else None,
        devto_url=row[14],
        created_at=row[15],
    )
