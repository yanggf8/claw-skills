"""Canonical delivery helper for nullclaw skills.

Replaces the per-skill `if args.deliver_to: telegram.send(...)` pattern.
Centralizes the "skill silently succeeds after delivery fail" bug class —
on failure the body is preserved on stdout (so cron_runs capture still
has the data for `nullclaw cron run-by-trace`) and the skill exits 1
unless explicitly opted out.

See ~/.claude/plans/zazzy-petting-hejlsberg.md (B1) for the design.
"""
import os
import sys
import time

import telegram


def deliver_or_fail(
    chat_id: str | None,
    body: str,
    *,
    account: str = "main",
    fail_on_delivery_error: bool = True,
) -> bool:
    """Send body to telegram chat_id; exit(1) on failure unless opted out.

    Behavior matrix:
      - chat_id empty / None: print body to stdout, return True.
      - send succeeds:        return True (body NOT echoed — channel has it).
      - send fails AND fail_on_delivery_error=True (default):
          1. print body to STDOUT (so cron_runs capture has the data),
          2. print error line to STDERR,
          3. sys.exit(1). Never returns False in this branch.
      - send fails AND fail_on_delivery_error=False:
          print body to stdout, error to stderr, return False.

    Stdout/stderr split: the body is data → stdout. The diagnostic line
    is reserved for stderr so log aggregators that treat stderr as "errors
    only" stay sane.

    When called inside a cron-spawned skill, the scheduler sets
    NULLCLAW_SKILL_TIMEOUT in the env. We use that as a wall-clock budget
    for telegram retries, so the retry loop cannot starve the skill's own
    timeout. Without that env var we fall back to telegram's built-in
    DEFAULT_DEADLINE_S cap.
    """
    if not chat_id:
        print(body)
        return True

    deadline_s = _resolve_delivery_deadline()
    ok = telegram.send(chat_id, body, account=account, deadline_s=deadline_s)
    if ok:
        return True

    print(body)  # stdout — preserves data for cron_runs capture
    print(
        f"[delivery] telegram send failed for chat={chat_id} account={account}",
        file=sys.stderr,
        flush=True,
    )
    if fail_on_delivery_error:
        sys.exit(1)
    return False


def _resolve_delivery_deadline() -> float | None:
    """Compute the wall-clock budget for delivery from the cron env vars.

    The scheduler sets:
      NULLCLAW_SKILL_TIMEOUT  — the skill's overall timeout, seconds
      NULLCLAW_SKILL_STARTED  — monotonic time the skill started, seconds (optional)

    If neither is set we return None so telegram.send falls back to its
    own DEFAULT_DEADLINE_S. If only timeout is set we treat the skill as
    if it just started — a safe over-estimate that still bounds the call.
    """
    raw_timeout = os.environ.get("NULLCLAW_SKILL_TIMEOUT")
    if not raw_timeout:
        return None
    try:
        timeout = float(raw_timeout)
    except ValueError:
        return None
    if timeout <= 0:
        return None

    raw_started = os.environ.get("NULLCLAW_SKILL_STARTED")
    if raw_started:
        try:
            started = float(raw_started)
            elapsed = max(0.0, time.monotonic() - started)
            remaining = max(0.0, timeout - elapsed)
            # Reserve 1s for the skill to actually exit cleanly after delivery.
            return max(0.0, remaining - 1.0)
        except ValueError:
            pass
    # Fall back to the full timeout, minus 1s safety margin.
    return max(0.0, timeout - 1.0)
