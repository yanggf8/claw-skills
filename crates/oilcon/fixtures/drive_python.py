#!/usr/bin/env python3
"""Drive oilcon/scripts/run.py over committed fixtures (no network).

Does NOT edit run.py. Substitutes three module attributes on the loaded
module object (the mechanism proven by task6_probe_skeleton.py):

  1. cst_now           — fixed clock so 更新： and the record line are stable
  2. oil_fetch         — fixture-backed history / latest (no sockets)
  3. oil_store         — in-memory sqlite standing in for Turso

Prints, for each fixture set, the three artefacts the Rust differential
compares byte-for-byte:

  1. skill status
  2. full rendered deliver message
  3. history record line

Plus a machine-readable seed/window dump so the differential can assert
row sequences match before rendering.

Usage:
  python3 drive_python.py
  FIXTURE_ROOT=... python3 drive_python.py
"""
from __future__ import annotations

import importlib.util
import json
import os
import sqlite3
import sys
import types
from pathlib import Path

# Must match the clock the Rust differential passes to format_message /
# format_record_line / needs_backfill.
PINNED_NOW = "2026-07-30 22:00"
PINNED_NOW_SECS = "2026-07-30 22:00:00 CST"
PINNED_TODAY = "2026-07-30"

HERE = Path(__file__).resolve().parent
FIXTURE_ROOT = Path(os.environ.get("FIXTURE_ROOT", str(HERE)))
REPO = (HERE / ".." / ".." / "..").resolve()
LIB = REPO / "lib"
RUN_PY = REPO / "oilcon" / "scripts" / "run.py"

sys.path.insert(0, str(LIB))

SYMBOL_FILES = {
    "CL=F": "CL_F_rows.json",
    "BZ=F": "BZ_F_rows.json",
    "HO=F": "HO_F_rows.json",
}
LABELS = {"CL=F": "WTI", "BZ=F": "Brent", "HO=F": "HO"}


def load_module():
    spec = importlib.util.spec_from_file_location("oilcon_run", str(RUN_PY))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def load_rows(set_dir: Path) -> dict[str, list[tuple[str, float]]]:
    out: dict[str, list[tuple[str, float]]] = {}
    for sym, fname in SYMBOL_FILES.items():
        path = set_dir / fname
        if not path.is_file():
            raise FileNotFoundError(path)
        raw = json.loads(path.read_text(encoding="utf-8"))
        out[sym] = [(str(d), float(c)) for d, c in raw]
    return out


def fixture_sets() -> list[tuple[str, Path]]:
    """(name, directory) pairs under FIXTURE_ROOT."""
    sets: list[tuple[str, Path]] = []
    live = FIXTURE_ROOT / "live"
    if (live / "CL_F_rows.json").is_file():
        sets.append(("live", live))
    # Root-level rows also count as live if live/ missing
    elif (FIXTURE_ROOT / "CL_F_rows.json").is_file():
        sets.append(("live", FIXTURE_ROOT))
    synth = FIXTURE_ROOT / "synthetic"
    if synth.is_dir():
        for child in sorted(synth.iterdir()):
            if child.is_dir() and (child / "CL_F_rows.json").is_file():
                sets.append((child.name, child))
    if not sets:
        print("ERROR: no fixture sets under", FIXTURE_ROOT, file=sys.stderr)
        sys.exit(2)
    return sets


class ConnProxy:
    """build_snapshot calls conn.close() in a finally block; sqlite3's close is
    read-only so it cannot be patched. Proxy it and swallow the close instead."""

    def __init__(self, real):
        self._real = real

    def close(self):
        pass

    def __getattr__(self, n):
        return getattr(self._real, n)


class MemStore:
    """In-memory stand-in for lib/oil_store.py against oil_daily.

    Schema mirrors the real table (date column, not day).
    MissingCredentialsError must be a class attribute — build_snapshot has
    `except oil_store.MissingCredentialsError` and looks it up on the module.
    """

    MissingCredentialsError = RuntimeError

    def __init__(self):
        self.db = ConnProxy(sqlite3.connect(":memory:"))

    def connect_from_env(self):
        return self.db

    def ensure_schema(self, conn):
        conn.execute(
            "CREATE TABLE IF NOT EXISTS oil_daily("
            "symbol TEXT NOT NULL, date TEXT NOT NULL, close REAL NOT NULL, "
            "PRIMARY KEY(symbol, date))"
        )
        conn.commit()

    def needs_backfill(self, conn, symbol):
        # Faithful to lib/oil_store.py: presence check only.
        row = conn.execute(
            "SELECT 1 FROM oil_daily WHERE symbol = ? LIMIT 1", (symbol,)
        ).fetchone()
        return row is None

    def insert_many(self, conn, symbol, rows):
        conn.executemany(
            "INSERT INTO oil_daily(symbol, date, close) VALUES (?, ?, ?) "
            "ON CONFLICT(symbol, date) DO UPDATE SET close = excluded.close",
            [(symbol, d, c) for d, c in rows],
        )
        conn.commit()

    def upsert(self, conn, symbol, day, close):
        self.insert_many(conn, symbol, [(day, close)])

    def window(self, conn, symbol, limit):
        r = conn.execute(
            "SELECT date, close FROM oil_daily WHERE symbol = ? "
            "ORDER BY date DESC LIMIT ?",
            (symbol, limit),
        ).fetchall()
        ordered = [(row[0], float(row[1])) for row in r]
        ordered.reverse()
        return ordered


def make_fetch(fix_data: dict[str, list[tuple[str, float]]]):
    def fetch_history(symbol, **kwargs):
        return list(fix_data[symbol])

    def fetch_latest(symbol, **kwargs):
        rows = fix_data[symbol]
        if not rows:
            return None
        return rows[-1]

    return types.SimpleNamespace(fetch_history=fetch_history, fetch_latest=fetch_latest)


def seed_store(store: MemStore, fix_data: dict[str, list[tuple[str, float]]]) -> None:
    """Load fixture rows through the real writer (insert_many), not a raw SQL dump."""
    conn = store.db
    store.ensure_schema(conn)
    for symbol, rows in fix_data.items():
        store.insert_many(conn, symbol, rows)


def drive_one(m, name: str, set_dir: Path) -> None:
    fix_data = load_rows(set_dir)

    # 1. clock
    m.cst_now = lambda with_seconds=False: (
        PINNED_NOW_SECS if with_seconds else PINNED_NOW
    )

    # 2. fetch — fixture-backed, no network
    m.oil_fetch = make_fetch(fix_data)

    # 3. store — in-memory, pre-seeded through insert_many so the store path
    # is inside the differential. needs_backfill then returns false (presence).
    store = MemStore()
    seed_store(store, fix_data)
    m.oil_store = store

    # Assert seed landed: window per symbol (252) before render.
    print(f"===SET==={name}")
    print(f"===NOW==={PINNED_NOW}")
    print(f"===NOW_SECS==={PINNED_NOW_SECS}")
    print(f"===TODAY==={PINNED_TODAY}")
    print("===SEED===")
    for symbol in ("CL=F", "BZ=F", "HO=F"):
        rows = store.window(store.db, symbol, 252)
        print(f"===SYMBOL==={symbol}")
        for day, close in rows:
            # Full precision — no rounding; the differential compares sequences.
            print(f"{day}\t{close!r}")
    print("===END_SEED===")

    snap = m.build_snapshot()
    print(f"===WARNING==={snap.warning if snap.warning is not None else ''}")
    if snap.warning:
        # A warning means every fixture set was supposed to be pre-seeded and was
        # not. Fail loudly rather than compare anything.
        #
        # This branch must NOT reconstruct run.py's warning message. run.py builds
        # that string inline in `main` (run.py:321), so there is no oracle function
        # to call, and a hand-written copy here would compare one reading of the
        # Python against another — exactly the gap this differential exists to
        # close. It would also read as coverage while proving nothing.
        #
        # The Rust warning message is verified instead by hand-diffing run.rs
        # against run.py:321, recorded in the Task 5 outcome. If the warning path
        # ever needs differential coverage, drive `main` — do not rebuild the
        # string here.
        raise SystemExit(
            f"set {name}: build_snapshot returned a warning ({snap.warning!r}); "
            "fixtures must be seeded so the happy path is reached. Refusing to "
            "compare a reconstructed warning message."
        )

    msg, status = m.format_message(snap)
    record = m.format_record_line(snap)
    wti_rows = snap.symbols["WTI"].rows or []
    trend = m.classify_oil_trend(wti_rows) if len(wti_rows) >= 70 else "insufficient-history"

    print(f"===SKILL==={status}")
    print(f"===TREND==={trend}")
    print("===RECORD===")
    print(record)
    print("===MESSAGE===")
    print(msg)
    print("===END===")
    # Diagnostics (not compared)
    print("===DIAG===")
    for label in ("WTI", "Brent", "HO"):
        ss = snap.symbols[label]
        n = len(ss.rows) if ss.rows is not None else 0
        print(f"{label}: rows={n} stale={ss.stale}")
    print(f"trend={trend}")
    print("===END_DIAG===")


def main() -> int:
    m = load_module()
    for name, directory in fixture_sets():
        drive_one(m, name, directory)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
