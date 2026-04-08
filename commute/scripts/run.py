#!/usr/bin/env python3
"""Commute skill: traffic-only — fetches route travel time via traffic sub-skill.

Weather is handled by a separate standalone cron job to avoid repetition
when multiple commute legs fire in sequence.
"""
import argparse
import os
import subprocess
import sys

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
import telegram

SKILLS_DIR = os.path.expanduser("~/.nullclaw/skills")
ERRORS_LOG = os.path.expanduser("~/.nullclaw/skill-errors.log")


def run_skill(script_path: str, extra_args: list[str]) -> str:
    """Run a skill script and return its stdout. Logs stderr to errors log."""
    cmd = [sys.executable, script_path] + extra_args
    env = {k: v for k, v in os.environ.items() if k != "NULLCLAW_JOB_ID"}
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=30, env=env)
        if result.stderr:
            with open(ERRORS_LOG, "a", encoding="utf-8") as f:
                f.write(result.stderr)
        return result.stdout.strip()
    except subprocess.TimeoutExpired:
        with open(ERRORS_LOG, "a", encoding="utf-8") as f:
            f.write(f"[TIMEOUT] {script_path} {extra_args}\n")
        return ""
    except Exception as e:
        with open(ERRORS_LOG, "a", encoding="utf-8") as f:
            f.write(f"[ERROR] {script_path}: {e}\n")
        return ""


def main():
    parser = argparse.ArgumentParser(description="Commute: traffic only")
    parser.add_argument("--from", dest="origin", required=True, metavar="LOCATION")
    parser.add_argument("--to", dest="dest", required=True, metavar="LOCATION")
    parser.add_argument("--via", dest="via", default=None, metavar="LOCATION")
    parser.add_argument("--location", action="append", default=None, dest="locations",
                        metavar="LOCATION", help="(ignored, kept for backward compat)")
    parser.add_argument("--deliver-to", dest="deliver_to", default=None, metavar="CHAT_ID",
                        help="Telegram chat ID to deliver output to directly")
    parser.add_argument("--account", default="main", help="Telegram bot account name")
    args = parser.parse_args()

    traffic_args = ["--from", args.origin, "--to", args.dest]
    if args.via:
        traffic_args += ["--via", args.via]

    traffic_script = os.path.join(SKILLS_DIR, "traffic", "scripts", "run.py")
    traffic_out = run_skill(traffic_script, traffic_args)

    if not traffic_out:
        traffic_out = "[traffic unavailable]"

    output = traffic_out
    job_id = os.environ.get("NULLCLAW_JOB_ID")
    if job_id:
        output += f"\n\n`{job_id}`"
    if args.deliver_to:
        telegram.send(args.deliver_to, output, account=args.account)
    else:
        print(output)


if __name__ == "__main__":
    main()
