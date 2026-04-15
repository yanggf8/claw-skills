"""Storage helpers for oilcon time-series data."""
import os
import sqlite3

try:
    import libsql_experimental as libsql
except ImportError:  # pragma: no cover - dependency is optional in tests
    libsql = None


CREATE_TABLE_SQL = """
CREATE TABLE IF NOT EXISTS oil_daily (
  symbol TEXT NOT NULL,
  date   TEXT NOT NULL,
  close  REAL NOT NULL,
  PRIMARY KEY (symbol, date)
)
"""

CREATE_INDEX_SQL = """
CREATE INDEX IF NOT EXISTS idx_oil_daily_symbol_date
  ON oil_daily(symbol, date DESC)
"""


class MissingCredentialsError(RuntimeError):
    """Raised when Turso connection credentials are absent."""


def connect(database_url: str, auth_token: str | None = None):
    if database_url == ":memory:" or database_url.startswith("file:"):
        conn = sqlite3.connect(database_url)
        conn.row_factory = sqlite3.Row
        return conn

    if libsql is None:
        raise RuntimeError("libsql-experimental not installed")
    return libsql.connect(database_url, auth_token=auth_token)


def connect_from_env():
    database_url = os.environ.get("TURSO_DATABASE_URL")
    auth_token = os.environ.get("TURSO_AUTH_TOKEN")
    if not database_url or not auth_token:
        raise MissingCredentialsError("turso credentials missing")
    return connect(database_url, auth_token=auth_token)


def ensure_schema(conn) -> None:
    conn.execute(CREATE_TABLE_SQL)
    conn.execute(CREATE_INDEX_SQL)
    conn.commit()


def needs_backfill(conn, symbol: str) -> bool:
    row = conn.execute(
        "SELECT 1 FROM oil_daily WHERE symbol = ? LIMIT 1",
        (symbol,),
    ).fetchone()
    return row is None


def insert_many(conn, symbol: str, rows: list[tuple[str, float]]) -> None:
    conn.executemany(
        """
        INSERT INTO oil_daily(symbol, date, close)
        VALUES (?, ?, ?)
        ON CONFLICT(symbol, date) DO UPDATE SET close = excluded.close
        """,
        [(symbol, day, close) for day, close in rows],
    )
    conn.commit()


def upsert(conn, symbol: str, day: str, close: float) -> None:
    insert_many(conn, symbol, [(day, close)])


def window(conn, symbol: str, limit: int) -> list[tuple[str, float]]:
    rows = conn.execute(
        """
        SELECT date, close
        FROM oil_daily
        WHERE symbol = ?
        ORDER BY date DESC
        LIMIT ?
        """,
        (symbol, limit),
    ).fetchall()
    ordered = [(row[0], float(row[1])) for row in rows]
    ordered.reverse()
    return ordered


def latest_row(conn, symbol: str) -> tuple[str, float] | None:
    row = conn.execute(
        """
        SELECT date, close
        FROM oil_daily
        WHERE symbol = ?
        ORDER BY date DESC
        LIMIT 1
        """,
        (symbol,),
    ).fetchone()
    if row is None:
        return None
    return row[0], float(row[1])
