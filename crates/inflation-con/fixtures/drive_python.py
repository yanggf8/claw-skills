#!/usr/bin/env python3
"""Drive inflation-con/scripts/run.py over captured FRED CSV fixtures.

Does NOT edit run.py. Replaces `run.fred_fetch` on the module object so
`fetch_all` reads fixture files — the same seam inflation-con/scripts/test_run.py
uses with monkeypatch.setattr(run, "fred_fetch", …).

Prints, for each fixture set, the three values the Rust differential compares:
  1. history record line (--mode record shape)
  2. full rendered deliver message
  3. skill status ("ok" / "degraded")

Clock handling: Python's `record_line` calls `datetime.now` internally.
This driver post-processes the leading timestamp to PINNED_NOW so both sides
share one string. Rust's `record_line` already takes `now` as a parameter.

Usage:
  python3 drive_python.py                  # live + hot under this dir
  FIXTURE_ROOT=... python3 drive_python.py
"""
from __future__ import annotations

import os
import sys
import types
from pathlib import Path

# PINNED_NOW must match the string the Rust differential passes to record_line.
PINNED_NOW = "2026-07-30 12:00:00 CST"

HERE = Path(__file__).resolve().parent
FIXTURE_ROOT = Path(os.environ.get("FIXTURE_ROOT", str(HERE)))

# inflation-con/scripts (run.py + fred_fetch.py)
SCRIPTS = (HERE / ".." / ".." / ".." / "inflation-con" / "scripts").resolve()
# repo lib/ (delivery, trace_marker) — not used when we call pure helpers
LIB = (HERE / ".." / ".." / ".." / "lib").resolve()
sys.path.insert(0, str(LIB))
sys.path.insert(0, str(SCRIPTS))

import fred_fetch  # noqa: E402  — real parse_csv only
import run  # noqa: E402


def fixture_sets() -> list[tuple[str, Path]]:
    """(name, directory) pairs. live = root CSVs; hot = hot/ subdir if present."""
    sets: list[tuple[str, Path]] = []
    if (FIXTURE_ROOT / "PCEPILFE.csv").is_file():
        sets.append(("live", FIXTURE_ROOT))
    hot = FIXTURE_ROOT / "hot"
    if (hot / "PCEPILFE.csv").is_file():
        sets.append(("hot", hot))
    if not sets:
        print("ERROR: no fixture sets under", FIXTURE_ROOT, file=sys.stderr)
        sys.exit(2)
    return sets


def make_stub(fixture_dir: Path):
    """fred_fetch-shaped namespace: fetch_series(series_id) -> rows from CSV."""

    def fetch_series(series_id: str, **kwargs):
        path = fixture_dir / f"{series_id}.csv"
        if not path.is_file():
            raise FileNotFoundError(f"fixture missing: {path}")
        text = path.read_text(encoding="utf-8")
        return fred_fetch.parse_csv(text)

    return types.SimpleNamespace(fetch_series=fetch_series)


def pin_record_timestamp(line: str) -> str:
    """Replace Python's live clock with PINNED_NOW.

    record_line format: "{now} INFLATION-CON …" where now is
    "%Y-%m-%d %H:%M:%S CST". We keep everything from the first
    " INFLATION-CON" onward.
    """
    marker = " INFLATION-CON"
    idx = line.find(marker)
    if idx < 0:
        raise RuntimeError(f"record line missing status prefix: {line!r}")
    return PINNED_NOW + line[idx:]


def drive_one(name: str, fixture_dir: Path, policy_stance: str = "restrictive") -> None:
    run.fred_fetch = make_stub(fixture_dir)
    cfg = {
        "series": dict(run.DEFAULT_SERIES),
        "policy_stance": policy_stance,
    }
    series_rows, warning = run.fetch_all(cfg["series"])
    status, details = run.classify(series_rows, cfg["policy_stance"])
    message, skill_status = run.format_message(status, details, cfg, warning)
    record = pin_record_timestamp(run.record_line(status, details, warning))

    # Machine-readable blocks for the Rust test (and humans).
    print(f"===SET==={name}")
    print(f"===NOW==={PINNED_NOW}")
    print(f"===CLASSIFICATION==={status}")
    print(f"===SKILL==={skill_status}")
    print(f"===WARNING==={warning if warning is not None else ''}")
    print("===RECORD===")
    print(record)
    print("===MESSAGE===")
    print(message)
    print("===END===")
    # Diagnostics (not compared): row counts per series
    print("===DIAG===")
    for key, sid in run.DEFAULT_SERIES.items():
        rows = series_rows.get(key, [])
        last = rows[-1] if rows else None
        print(f"{key}={sid} rows={len(rows)} last={last}")
    print(f"reasons={details.get('reasons')}")
    print(f"pce3={details.get('pce3')} pce6={details.get('pce6')} "
          f"cpi3={details.get('cpi3')} cpi6={details.get('cpi6')} "
          f"be={details.get('breakeven')} rising={details.get('breakeven_rising')}")
    print("===END_DIAG===")


def main() -> int:
    stance = os.environ.get("POLICY_STANCE", "restrictive")
    for name, directory in fixture_sets():
        drive_one(name, directory, policy_stance=stance)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
