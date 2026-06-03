#!/usr/bin/env python3
"""Doughcon skill: fetch PizzINT DOUGHCON level and deliver or record."""
import argparse
import json
import os
import sys
import urllib.request
from datetime import datetime, timezone, timedelta

try:
    from zoneinfo import ZoneInfo
    _NY = ZoneInfo("America/New_York")
    _TPE = ZoneInfo("Asia/Taipei")
except Exception:  # pragma: no cover - zoneinfo missing
    _NY = None
    _TPE = None

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
from delivery import deliver_or_fail
from trace_marker import emit_skill_status, emit_trace


HISTORY_LOG = os.path.expanduser("~/.nullclaw/doughcon-history.log")


def fetch_doughcon() -> dict:
    url = "https://pizzint.watch/api/dashboard-data"
    req = urllib.request.Request(
        url,
        headers={"Accept": "application/json", "User-Agent": "nullclaw/1.0"},
    )
    with urllib.request.urlopen(req, timeout=20) as resp:
        return json.load(resp)


def cst_now() -> str:
    dt = datetime.now(timezone(timedelta(hours=8)))
    return dt.strftime("%Y-%m-%d %H:%M:%S CST")


def _parse_api_ts(raw):
    """Parse PizzINT's ISO-8601 UTC timestamp (e.g. '2026-06-03T03:23:38.739Z')."""
    if not raw:
        return None
    try:
        return datetime.fromisoformat(raw.replace("Z", "+00:00"))
    except Exception:
        return None


def format_updated(data: dict) -> str:
    """Return the data's real timestamp in Taiwan + US-East time.

    Uses the API's own ``timestamp`` (when the snapshot was taken) rather than
    the moment this script ran — the API has no ``updated_at`` field, so the old
    ``cst_now()`` fallback always showed the cron fire time, not the data time.
    US-East line auto-switches EDT/EST via zoneinfo so it tracks daylight-saving
    without a hardcoded offset. Falls back to the run time only when the API
    gives no usable timestamp.
    """
    dt = _parse_api_ts(data.get("timestamp"))
    if dt is None:
        return cst_now()
    if _TPE is not None and _NY is not None:
        cst = dt.astimezone(_TPE).strftime("%Y-%m-%d %H:%M CST")
        et = dt.astimezone(_NY)
        return f"{cst}（美東 {et.strftime('%m-%d %H:%M')} {et.tzname()}）"
    cst = dt.astimezone(timezone(timedelta(hours=8))).strftime("%Y-%m-%d %H:%M CST")
    return cst


def main():
    parser = argparse.ArgumentParser(description="Fetch PizzINT DOUGHCON data")
    parser.add_argument(
        "--mode",
        choices=["deliver", "record"],
        default="deliver",
        help="deliver: print/send formatted output; record: append to history log",
    )
    parser.add_argument("--deliver-to", dest="deliver_to", default=None, metavar="CHAT_ID",
                        help="Telegram chat ID to deliver output to directly")
    parser.add_argument("--account", default="main", help="Telegram bot account name")
    parser.add_argument(
        "--et-hour", dest="et_hour", type=int, default=None, metavar="H",
        help="Only run when the current US-Eastern hour equals H (0-23). "
             "Lets a UTC cron auto-track DST: schedule 00:00 and 01:00 UTC and "
             "set --et-hour 20 so exactly one firing passes (summer EDT vs winter EST). "
             "Omit to run unconditionally.",
    )
    args = parser.parse_args()

    # DST gate: a fixed-offset cron can't follow US-Eastern across DST, so we run
    # on both candidate UTC hours and let the script keep only the one that lands
    # on the target Eastern hour. Skipping is a no-op success (exit 0).
    if args.et_hour is not None:
        if _NY is None:
            print("[WARN: --et-hour requires zoneinfo; running unconditionally]",
                  file=sys.stderr)
        else:
            now_et = datetime.now(_NY)
            if now_et.hour != args.et_hour:
                print(f"[skip: US-Eastern hour {now_et.hour:02d} != target "
                      f"{args.et_hour:02d} ({now_et.tzname()})]", file=sys.stderr)
                # A deliberate DST-gate skip is a successful no-op. Emit the
                # skill_contract markers so the scheduler records 'ok' instead
                # of flagging empty stdout as content_invalid and burning a retry.
                emit_skill_status("ok")
                emit_trace()
                sys.exit(0)

    try:
        data = fetch_doughcon()
    except Exception as e:
        if args.mode == "deliver":
            msg = f"[WARN: doughcon unavailable - {e}]"
            # Upstream-fetch failure is a degraded but expected state for a
            # status skill — opt out of exit(1) so cron records a soft warning
            # rather than a hard failure.
            deliver_or_fail(
                args.deliver_to, msg,
                account=args.account, fail_on_delivery_error=False,
            )
            emit_skill_status("degraded")
            emit_trace()
            sys.exit(0)
        else:
            print(f"[ERROR: doughcon unavailable - {e}]", file=sys.stderr)
            sys.exit(1)

    level = data.get("defcon_level", "?")
    raw_index = data.get("overall_index")
    # Check if all places have null popularity — means no data, not true zero
    places = data.get("data", [])
    all_null = all(p.get("current_popularity") is None for p in places) if places else True
    if raw_index is None or (raw_index == 0 and all_null):
        index = -1  # no data available
    else:
        index = raw_index
    updated = format_updated(data)

    if args.mode == "deliver":
        output = f"🍕 DOUGHCON 情報\n目前等級：DOUGHCON {level}\n指數：{index}\n更新：{updated}"
        job_id = os.environ.get("NULLCLAW_JOB_ID")
        if job_id:
            output += f"\n\n`{job_id}`"
        deliver_or_fail(args.deliver_to, output, account=args.account)
        emit_skill_status("ok" if index != -1 else "degraded")
        emit_trace()
    else:
        # record mode: append a single line to history log
        try:
            with open(HISTORY_LOG, "a", encoding="utf-8") as f:
                f.write(f"{cst_now()}  DOUGHCON {level}  index={index}\n")
        except Exception as e:
            print(f"[ERROR: could not write history log - {e}]", file=sys.stderr)
            sys.exit(1)
        emit_skill_status("ok")
        emit_trace()


if __name__ == "__main__":
    main()
