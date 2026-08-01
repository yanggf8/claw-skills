#!/usr/bin/env python3
"""AutoCLI skill: fetch structured data from 55+ websites via autocli."""
import argparse
import json
import os
import shutil
import subprocess
import sys

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
from delivery import deliver_or_fail
from trace_marker import emit_trace

JOB_ID = os.environ.get("NULLCLAW_JOB_ID", "")


def log(msg: str) -> None:
    prefix = f"[autocli/{JOB_ID}]" if JOB_ID else "[autocli]"
    print(f"{prefix} {msg}", file=sys.stderr)


def find_autocli() -> str | None:
    """Find autocli binary. Check PATH, then common install locations."""
    path = shutil.which("autocli")
    if path:
        return path
    for d in ["~/.cargo/bin", "~/.local/bin", "~/bin"]:
        candidate = os.path.expanduser(os.path.join(d, "autocli"))
        if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate
    return None


def run_autocli(
    autocli_path: str,
    site: str,
    command: str | None,
    limit: int | None,
    extra_args: list[str],
    timeout: int,
) -> tuple[list | dict | None, str]:
    """Spawn autocli, return (parsed_json_or_None, raw_stdout).

    Calls sys.exit(1) on non-zero exit code so emit_trace is never reached
    on hard failures.
    """
    cmd = [autocli_path, site]
    if command:
        cmd.append(command)
    cmd.extend(["--format", "json"])
    if limit is not None:
        cmd.extend(["--limit", str(limit)])
    cmd.extend(extra_args)

    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        log(f"autocli timed out after {timeout}s")
        sys.exit(1)

    if result.returncode != 0:
        log(f"autocli exited {result.returncode}: {result.stderr.strip()}")
        stderr_lower = result.stderr.lower()
        if "cookie" in stderr_lower or "browser" in stderr_lower or "login" in stderr_lower:
            print(
                "This site may require the AutoCLI browser extension.\n"
                "See: https://github.com/nashsu/AutoCLI#chrome-extension-setup",
                file=sys.stderr,
            )
        sys.exit(1)

    raw = result.stdout.strip()
    try:
        data = json.loads(raw)
        # Normalize: if top-level is an object with a list value, extract it
        if isinstance(data, dict):
            for v in data.values():
                if isinstance(v, list):
                    data = v
                    break
        return data, raw
    except json.JSONDecodeError:
        log("JSON parse failed; using raw output")
        return None, raw


def format_output(
    data: list | dict | None,
    raw: str,
    site: str,
    command: str | None,
    max_chars: int = 3800,
) -> str:
    """Format JSON array into a numbered list for Telegram delivery.

    Falls back to truncated raw text when data is not a list.
    """
    if data is None or not isinstance(data, list):
        return raw[:max_chars]

    if not data:
        return f"{site} {command or ''}: no results"

    header = site
    if command:
        header += f" {command}"
    lines = [header, ""]

    for i, item in enumerate(data):
        if not isinstance(item, dict):
            line = f"{i + 1}. {item}"
        else:
            # Pick a title field
            title = (
                item.get("title")
                or item.get("name")
                or item.get("text")
                or str(next(iter(item.values()), ""))
            )
            # Build detail from remaining fields (skip title/url)
            skip = {"title", "name", "text", "url", "link", "href"}
            detail_parts = []
            for k, v in list(item.items())[:5]:
                if k in skip:
                    continue
                detail_parts.append(f"{k}: {v}")
            detail = " | ".join(detail_parts[:3])
            line = f"{i + 1}. {title}"
            if detail:
                line += f"\n   {detail}"

        candidate = "\n".join(lines + [line])
        if len(candidate) > max_chars:
            remaining = len(data) - i
            lines.append(f"... (+{remaining} more)")
            break
        lines.append(line)

    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Fetch structured data from 55+ websites via AutoCLI"
    )
    parser.add_argument("site", help="Site name (e.g. hackernews, bilibili) or 'list'")
    parser.add_argument(
        "command", nargs="?", default=None, help="Command (e.g. top, hot, search)"
    )
    parser.add_argument("--limit", type=int, default=None, help="Max items to fetch")
    parser.add_argument(
        "--timeout", type=int, default=90, help="Subprocess timeout in seconds"
    )
    parser.add_argument(
        "--raw", action="store_true", help="Output raw JSON (for agent consumption)"
    )
    parser.add_argument("--deliver-to", dest="deliver_to", default=None, metavar="CHAT_ID")
    parser.add_argument("--account", default="main")
    args, extra = parser.parse_known_args()

    autocli_path = find_autocli()
    if not autocli_path:
        print(
            "autocli not found. Install:\n"
            "  curl -fsSL https://raw.githubusercontent.com/nashsu/autocli/main/scripts/install.sh | sh",
            file=sys.stderr,
        )
        sys.exit(1)

    # ── List mode ────────────────────────────────────────────────────────
    if args.site == "list":
        try:
            result = subprocess.run(
                [autocli_path, "--help"],
                capture_output=True,
                text=True,
                timeout=args.timeout,
            )
        except subprocess.TimeoutExpired:
            log(f"autocli --help timed out after {args.timeout}s")
            sys.exit(1)
        # --help may exit 0 or 2 depending on the tool; accept both
        output = (result.stdout or result.stderr).strip()
        if not output:
            output = "(no commands found)"
        if JOB_ID:
            output += f"\n\n`{JOB_ID}`"
        deliver_or_fail(args.deliver_to, output, account=args.account)
        emit_trace()
        return

    # ── Fetch mode ───────────────────────────────────────────────────────
    if not args.command:
        print(
            "Usage: run.py <site> <command> [--limit N] [--deliver-to CHAT_ID]",
            file=sys.stderr,
        )
        sys.exit(1)

    data, raw = run_autocli(
        autocli_path, args.site, args.command, args.limit, extra, args.timeout
    )
    # run_autocli calls sys.exit(1) on failure — emit_trace never reached

    if args.raw:
        # Raw mode bypasses the canonical delivery helper because the
        # whole point is to dump unformatted bytes to stdout for piping.
        print(raw)
    else:
        output = format_output(data, raw, args.site, args.command)
        if JOB_ID:
            output += f"\n\n`{JOB_ID}`"
        deliver_or_fail(args.deliver_to, output, account=args.account)

    emit_trace()


if __name__ == "__main__":
    main()
