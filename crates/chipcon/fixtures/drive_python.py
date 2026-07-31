#!/usr/bin/env python3
"""Drive chipcon/scripts/run.py over committed fixtures (no network).

Does NOT edit run.py. Substitutes two module attributes on the loaded
module object (the mechanism proven by the pre-dispatch probe):

  1. oil_fetch  — fixture-backed history (no sockets); chipcon imports the
                  same oil_fetch module oilcon uses
  2. datetime   — CLASS substitution (not a helper). record_line calls
                  datetime.now(tz).strftime(...) INLINE at run.py:248; there
                  is no cst_now-style function to replace. FakeDT.now must
                  accept tz and return a real datetime carrying it, or
                  .strftime() breaks. Capture the real class first.

No store: update_state fetches fresh each run into memory.

Prints, for each fixture set, the three artefacts the Rust differential
compares byte-for-byte:

  1. skill status
  2. full rendered deliver message
  3. history record line

Usage:
  python3 drive_python.py
  FIXTURE_ROOT=... python3 drive_python.py
"""
from __future__ import annotations

import importlib.util
import json
import os
import sys
import types
from pathlib import Path

# Must match the clock the Rust differential passes to record_line.
# Skeleton probe used 2026-04-15 12:00:00; FakeDT.now returns that instant
# with the tz passed by record_line (CST = UTC+8).
PINNED_NOW = "2026-04-15 12:00:00 CST"
PINNED_Y, PINNED_M, PINNED_D = 2026, 4, 15
PINNED_H, PINNED_MIN, PINNED_S = 12, 0, 0

HERE = Path(__file__).resolve().parent
FIXTURE_ROOT = Path(os.environ.get("FIXTURE_ROOT", str(HERE)))
REPO = (HERE / ".." / ".." / "..").resolve()
LIB = REPO / "lib"
RUN_PY = REPO / "chipcon" / "scripts" / "run.py"

sys.path.insert(0, str(LIB))

SYMBOL_FILES = {
    "SMH": "SMH_rows.json",
    "QQQ": "QQQ_rows.json",
    "SOXX": "SOXX_rows.json",
}


def load_module():
    spec = importlib.util.spec_from_file_location("chipcon_run", str(RUN_PY))
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
    if (live / "SMH_rows.json").is_file():
        sets.append(("live", live))
    elif (FIXTURE_ROOT / "SMH_rows.json").is_file():
        sets.append(("live", FIXTURE_ROOT))
    synth = FIXTURE_ROOT / "synthetic"
    if synth.is_dir():
        for child in sorted(synth.iterdir()):
            if child.is_dir() and (child / "SMH_rows.json").is_file():
                sets.append((child.name, child))
    if not sets:
        print("ERROR: no fixture sets under", FIXTURE_ROOT, file=sys.stderr)
        sys.exit(2)
    return sets


def make_fetch(fix_data: dict[str, list[tuple[str, float]]]):
    def fetch_history(symbol, range_name="1y", **kwargs):
        # update_state uppercases the ticker before calling.
        key = str(symbol).upper()
        if key not in fix_data:
            raise KeyError(f"fixture has no symbol {key!r}")
        return list(fix_data[key])

    return types.SimpleNamespace(fetch_history=fetch_history)


def pin_datetime(m, real_dt) -> None:
    """Replace m.datetime with a class whose now(tz) returns a pinned instant.

    record_line does `datetime.now(timezone(...)).strftime(...)` INLINE — there
    is no helper to swap. FakeDT must be a class; FakeDT.now must accept tz and
    return a real datetime carrying it, or .strftime() breaks.

    `real_dt` must be the *original* datetime class captured once at load time.
    Capturing m.datetime on every call is wrong: after the first set it is
    already FakeDT, and FakeDT(...) then raises TypeError.
    """

    class FakeDT:
        @staticmethod
        def now(tz=None):
            return real_dt(
                PINNED_Y, PINNED_M, PINNED_D,
                PINNED_H, PINNED_MIN, PINNED_S,
                tzinfo=tz,
            )

    m.datetime = FakeDT


def drive_one(m, real_dt, name: str, set_dir: Path) -> None:
    fix_data = load_rows(set_dir)

    # 1. fetch — fixture-backed, no network
    m.oil_fetch = make_fetch(fix_data)

    # 2. clock — class substitution (see pin_datetime docstring)
    pin_datetime(m, real_dt)

    cfg = {
        "symbols": {"SMH": "SMH", "QQQ": "QQQ", "SOXX": "SOXX"},
        "manual_events": m.default_events(),
    }

    state, warning = m.update_state(cfg)
    status, details = m.classify(state)
    message, skill_status = m.format_message(status, details, cfg, warning)
    record = m.record_line(status, details, warning)

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
    # Diagnostics (not compared)
    print("===DIAG===")
    for key in ("SMH", "QQQ", "SOXX"):
        print(f"{key}: rows={len(state.get(key, []))}")
    print(f"classification={status}")
    print(f"reasons={details.get('reasons')}")
    print("===END_DIAG===")


def main() -> int:
    m = load_module()
    # Capture once, before any FakeDT substitution (see pin_datetime).
    real_dt = m.datetime
    for name, directory in fixture_sets():
        drive_one(m, real_dt, name, directory)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
