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


def emit_fallback(skill, primary, fallback, reason, scope, elapsed_ms=None, stream=sys.stderr):
    """Emit a [skill-event] natural-language sentence describing a fallback.

    Audience is an agent reading the trace later — phrasing should be a
    complete sentence with skill, primary, fallback, reason in plain words,
    and scope. Reason should NOT be a short code; use natural language like
    "CWA returned HTTP 502" or "request timed out after 8s".

    Always emits (no NULLCLAW_JOB_ID gate) so manual runs are also diagnosable.
    Goes to stderr by default so it doesn't pollute cron-verified stdout.
    """
    parts = [
        f"[skill-event] {skill} skill fell back from {primary} to {fallback}",
        f"because {reason}.",
        f"Fallback covered {scope}",
    ]
    if elapsed_ms is not None:
        parts.append(f"and took {elapsed_ms}ms.")
    else:
        parts[-1] = parts[-1] + "."
    print(" ".join(parts), file=stream, flush=True)
