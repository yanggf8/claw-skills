#!/usr/bin/env python3
"""Watchdog for the CCT *generator*: did the day's reports get written, and how late?

Why this exists. The four cct reports are **produced** by a GitHub Actions
`schedule:` trigger (`yanggf8/cct` → `.github/workflows/trading-system.yml` →
POST /api/v1/jobs/trigger) and **consumed** by a nullclaw cron read on a fixed
UTC minute. The consumer is a cron and the producer is not: Actions `schedule`
is best-effort, and between 2026-08-27 and 2026-09-01 it fired the 12:30 UTC
pre-market trigger 10.0, 10.1, 6.7 and 4.3 hours late, which is four mornings of
`[cron] skill 'cct' degraded: failure=contract_degraded` whose every reason
pointed at a worker that was, in fact, fine by the afternoon.

The skill can only ever report what it saw at read time. This script reports the
thing nobody was looking at — the *drift* — so a late producer alerts as "the
generator landed 4.3h late", not as four unexplained degradations.

Read-time truth comes from cron.db, not from a copy in this file: the cct read
schedule has already drifted out of SKILL.md once, and a second place to keep it
current is a second place to be wrong. When the DB cannot be read the drift
verdict still runs and the "did it land after the read" column says unknown.

Usage:
  tools/check-cct-generator.py                  # today's ET trading day
  tools/check-cct-generator.py --date 2026-09-01
  tools/check-cct-generator.py --from-file runs.json   # offline, from a capture
  tools/check-cct-generator.py --grace 2.0             # hours of tolerated lag

Exit: 0 quiet, 1 a finding, 2 could not evaluate (worker unreachable, bad JSON).
"""

import argparse
import datetime as dt
import json
import os
import shutil
import sqlite3
import sys
import tempfile
import urllib.error
import urllib.request
from zoneinfo import ZoneInfo

DEFAULT_BASE = "https://tft-trading-system.yanggf.workers.dev"

# report_type as the worker stores it -> ((UTC hour, UTC minute), weekday mask)
# Mirrors the four `schedule:` crons in the cct repo's trading-system.yml.
GENERATOR = {
    "pre-market": ((12, 30), set(range(0, 5))),
    "intraday": ((16, 0), set(range(0, 5))),
    "end-of-day": ((20, 5), set(range(0, 5))),
    "weekly": ((14, 0), {6}),
}

# report_type -> the `--mode` the cct skill is invoked with, for the cron.db lookup
CONSUMER_MODE = {
    "pre-market": "--mode pre-market",
    "intraday": "--mode intraday",
    "end-of-day": "--mode eod",
    "weekly": "--mode weekly",
}

# NYSE closures. Only the years covered here can be told from a dead trigger;
# any other year reports a missing run with "holiday table has no data for".
HOLIDAYS = {
    2026: {
        dt.date(2026, 1, 1),   # New Year's Day
        dt.date(2026, 1, 19),  # MLK Day
        dt.date(2026, 2, 16),  # Presidents Day
        dt.date(2026, 4, 3),   # Good Friday
        dt.date(2026, 5, 25),  # Memorial Day
        dt.date(2026, 6, 19),  # Juneteenth
        dt.date(2026, 7, 3),   # Independence Day (observed, Jul 4 is a Saturday)
        dt.date(2026, 9, 7),   # Labor Day
        dt.date(2026, 11, 26), # Thanksgiving
        dt.date(2026, 12, 25), # Christmas
    }
}


def api_key() -> str:
    """Same resolution order as crates/cct/src/api.rs: config, then the public default."""
    path = os.environ.get("CLAW_CONFIG") or os.path.expanduser("~/.nullclaw/config.json")
    try:
        with open(path, encoding="utf-8") as fh:
            cfg = json.load(fh)
        key = cfg.get("cct", {}).get("api_key")
        if isinstance(key, str) and key:
            return key
    except Exception:
        pass
    return "yanggf"


# The worker sits behind a WAF that answers 403 to the urllib default
# (`Python-urllib/3.x`) and 200 to `nullclaw-cct/1.0` — the header the skill
# itself sends (crates/cct/src/api.rs). Measured 2026-09-02: the identical
# request was 403 with one UA and 200 with the other, so a watchdog that
# cannot ask is not a watchdog.
USER_AGENT = "nullclaw-cct/1.0"


def fetch_runs(base: str, limit: int, timeout: int) -> list:
    url = f"{base.rstrip('/')}/api/v1/jobs/runs?limit={limit}"
    req = urllib.request.Request(
        url, headers={"X-API-KEY": api_key(), "User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = json.load(resp)
    if body.get("success") is not True:
        raise ValueError(f"envelope not successful: {str(body)[:160]}")
    runs = body.get("data", {}).get("runs")
    if runs is None:
        raise ValueError("envelope carries no data.runs")
    return runs


def read_times(db_path: str) -> dict:
    """mode arg -> (hour, minute, tz_offset_s) for the live cct read jobs.

    cron.db is WAL and the daemon holds it, so a `mode=ro` open fails; the copy
    is the same thing a backup does and writes nothing back.
    """
    out = {}
    try:
        tmp = tempfile.mkdtemp(prefix="cct-check-")
        try:
            for suffix in ("", "-wal", "-shm"):
                src = db_path + suffix
                if os.path.exists(src):
                    shutil.copy2(src, os.path.join(tmp, "cron.db" + suffix))
            con = sqlite3.connect(os.path.join(tmp, "cron.db"))
            try:
                for args, expr, off in con.execute(
                    "select skill_args, expression, tz_offset_s from cron_jobs "
                    "where skill_name = 'cct' and enabled = 1 and paused = 0"
                ):
                    fields = (expr or "").split()
                    if len(fields) < 5:
                        continue
                    out[(args or "").strip()] = (
                        int(fields[1]), int(fields[0]), int(off or 0))
            finally:
                con.close()
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
    except Exception:
        return {}
    return out


def parse_ts(text: str) -> dt.datetime:
    return dt.datetime.strptime(text, "%Y-%m-%dT%H:%M:%S.%fZ").replace(
        tzinfo=dt.timezone.utc)


def evaluate(runs, day: dt.date, grace: float, reads: dict) -> list:
    """One finding per report type that has something to say."""
    findings = []
    for rtype, ((h, m), days) in GENERATOR.items():
        if day.weekday() not in days:
            continue
        expected = dt.datetime(day.year, day.month, day.day, h, m,
                               tzinfo=dt.timezone.utc)
        rows = sorted((r for r in runs
                       if r.get("report_type") == rtype
                       and r.get("scheduled_date") == day.isoformat()),
                      key=lambda r: r.get("started_at") or "")
        if not rows:
            known = HOLIDAYS.get(day.year)
            if known and day in known:
                continue
            note = "" if known else f" (holiday table has no data for {day.year})"
            findings.append((f"{rtype} never ran on {day}{note}",
                             f"expected a github_actions trigger at "
                             f"{expected:%H:%M}Z"))
            continue

        last = rows[-1]
        started = parse_ts(last["started_at"])
        status = last.get("status")

        # Measure against the nearest nominal trigger, not the one the row is
        # stamped with. A run GitHub delivered at 00:49Z on the 28th belongs to
        # the 27th's 16:00Z cron even when the worker filed it under
        # scheduled_date 2026-08-28 — against the 28th's nominal time it prints
        # a "-15.2h" drift that reads like a clock bug instead of an
        # eight-hour-late trigger.
        nominal = min((expected - dt.timedelta(days=k) for k in (0, 1, 2)),
                      key=lambda c: abs((started - c).total_seconds()))
        drift = (started - nominal).total_seconds() / 3600.0
        stamped = "" if nominal.date() == day else \
            f" [row stamped {day:%m-%d}, nearest trigger {nominal:%m-%d %H:%M}Z]"

        read = reads.get(CONSUMER_MODE[rtype])
        after_read = ""
        missed_push = False
        if read:
            rh, rm, roff = read
            read_dt = expected.replace(hour=rh, minute=rm,
                                       tzinfo=dt.timezone.utc) - dt.timedelta(seconds=roff)
            if started > read_dt:
                missed_push = True
                after_read = f", after the read at {read_dt:%H:%M}Z"

        if status != "success":
            findings.append((f"{rtype} {status} on {day}",
                             f"started {started:%m-%d %H:%M}Z (drift {drift:+.1f}h) "
                             f"stage={last.get('current_stage')}{after_read}{stamped}"))
        elif drift > grace or missed_push:
            why = f"landed {drift:+.1f}h late ({started:%H:%M}Z vs {nominal:%H:%M}Z)"
            if missed_push:
                why += after_read
            why += stamped
            findings.append((f"{rtype} {why}",
                             f"status={status} run_id={last.get('run_id')}"))
    return findings


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--date", help="ET business date to check (default: today in America/New_York)")
    ap.add_argument("--base", default=os.environ.get("CCT_BASE", DEFAULT_BASE))
    ap.add_argument("--limit", type=int, default=200)
    ap.add_argument("--grace", type=float, default=2.0,
                    help="hours of generator lateness that still counts as on time")
    ap.add_argument("--from-file", help="evaluate a captured runs JSON instead of the live API")
    ap.add_argument("--timeout", type=int, default=15)
    args = ap.parse_args()

    if args.date:
        try:
            day = dt.date.fromisoformat(args.date)
        except ValueError:
            print(f"[ERROR: --date is not YYYY-MM-DD] {args.date}", file=sys.stderr)
            return 2
    else:
        try:
            now = dt.datetime.now(ZoneInfo("America/New_York"))
        except Exception:
            now = dt.datetime.now(dt.timezone.utc) - dt.timedelta(hours=4)
        day = now.date()

    try:
        if args.from_file:
            with open(args.from_file, encoding="utf-8") as fh:
                payload = json.load(fh)
            runs = payload.get("data", {}).get("runs", payload.get("runs", []))
        else:
            runs = fetch_runs(args.base, args.limit, args.timeout)
    except urllib.error.HTTPError as e:
        print(f"⚠️ cct generator check could not run: HTTP {e.code} from "
              f"{args.base} (UA {USER_AGENT})")
        return 2
    except (urllib.error.URLError, OSError, ValueError, json.JSONDecodeError) as e:
        print(f"⚠️ cct generator check could not run: {e}")
        return 2

    db = os.environ.get("NULLCLAW_CRON_DB") or os.path.expanduser("~/.nullclaw/cron.db")
    reads = read_times(db)
    if not reads:
        print("note: cct read times unknown (could not read cron.db) — drift only")

    findings = evaluate(runs, day, args.grace, reads)

    # nullclaw cuts an alert preview at 200 bytes, so the verdict leads and the
    # per-type detail follows; a green day says so in one short line.
    types = [t for t in GENERATOR if day.weekday() in GENERATOR[t][1]]
    if not findings:
        if not types:
            print(f"✅ cct generator: no scheduled reports for {day} ({day:%a})")
            return 0
        print(f"✅ cct generator on time for {day} "
              f"({', '.join(f'{t} ≤{args.grace:g}h' for t in types)})")
        return 0

    print(f"⚠️ cct generator drifted for {day}: {len(findings)} finding(s)")
    for head, detail in findings:
        print(f"   - {head}\n     {detail}")
    print("   The worker serves the latest snapshot, so the cct push degrades to "
          "yesterday's report until the trigger catches up. Cause is the Actions "
          "`schedule:` start time, not the skill.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
