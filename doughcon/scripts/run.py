#!/usr/bin/env python3
"""Doughcon skill: fetch PizzINT DOUGHCON level and deliver or record."""
import argparse
import json
import os
import sys
import urllib.request
from datetime import datetime, timezone, timedelta

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
    args = parser.parse_args()

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
    updated = data.get("updated_at", "") or cst_now()

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
