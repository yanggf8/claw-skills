"""Emit scheduler verification markers for cron skill jobs.

Supported scheduler-owned verification modes:
- content_has_trace: emit_trace() on successful completion
- skill_contract: emit_skill_status(...) then emit_trace() on separate lines

Call the marker helpers only after delivery confirmation so delivery failures
remain hard exec errors instead of semantic verification failures.
"""
import os
import sys


VALID_SKILL_STATUSES = {"ok", "degraded", "failed"}


def emit_skill_status(status, stream=sys.stdout):
    """Print [skill-status:<status>] for skill_contract verification.

    No-op when NULLCLAW_JOB_ID is unset, so manual invocations of migrated
    skills don't pollute stdout with marker lines.
    """
    if status not in VALID_SKILL_STATUSES:
        raise ValueError(f"invalid skill status: {status}")
    if not os.environ.get("NULLCLAW_JOB_ID"):
        return
    print(f"[skill-status:{status}]", file=stream, flush=True)


def emit_trace(stream=sys.stdout):
    """Print [trace:<NULLCLAW_JOB_ID>] to stream. No-op if env var unset."""
    job_id = os.environ.get("NULLCLAW_JOB_ID")
    if job_id:
        print(f"[trace:{job_id}]", file=stream, flush=True)
