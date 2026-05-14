"""Shared agent-first skill runtime helpers.

Every scheduled nullclaw skill that follows the agent-first pattern needs
the same plumbing: log lines tagged with skill+job_id, cron skill-contract
markers (`[skill-status:...]`, `[trace:...]`), subprocess wrappers that
redact long prompts, label parsing for persona-core stdout, agent
invocation with optional output-marker extraction, tempfile writers.

This module is the single source of truth for that plumbing. Skills
import it instead of duplicating the same ~125 lines each.

Per the 2026-05-15 agent-first correction (persona-core/DESIGN.md §5.0),
LLM phases stay in the calling skill's process — persona-core does not
spawn nested agent processes. Skills shell out to `nullclaw agent -m`
themselves; this module is the canonical wrapper for that call.

Usage:

    import sys, os
    sys.path.insert(0, os.path.expanduser("~/clawd/skills/lib"))
    import skill_runner as sr

    sr.init("liko-finance-weekly")
    sr.log("starting run")
    persona = sr.pc("personas", "get", "liko-finance")
    body = sr.call_agent(prompt, timeout=900,
                         body_marker=("BEGIN_ISSUE_BODY", "END_ISSUE_BODY"))
    sr.emit_status("ok"); sr.emit_trace()
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

# Module-level skill identity. Set via init() before calling other helpers.
# Reads NULLCLAW_JOB_ID from env at import time so cron contexts get the
# right trace id without each skill having to fish for it.
_SKILL_ID: str = ""
JOB_ID: str = os.environ.get("NULLCLAW_JOB_ID", "")


def init(skill_id: str) -> None:
    """Register this skill's identity. Call once at the top of a skill run."""
    global _SKILL_ID
    if not skill_id or not skill_id.strip():
        raise ValueError("skill_id must be a non-empty string")
    _SKILL_ID = skill_id.strip()


def _require_init() -> str:
    if not _SKILL_ID:
        raise RuntimeError("skill_runner.init(<skill_id>) must be called first")
    return _SKILL_ID


# ── log + cron skill-contract markers ────────────────────────────────


def log(message: str) -> None:
    """Tagged stderr log line: '[<skill>/<job_id>] <message>' or '[<skill>] <message>'."""
    skill = _require_init()
    prefix = f"[{skill}/{JOB_ID}]" if JOB_ID else f"[{skill}]"
    print(f"{prefix} {message}", file=sys.stderr, flush=True)


def emit_status(status: str) -> None:
    """nullclaw cron skill-contract status marker. Emit exactly once per run."""
    print(f"[skill-status:{status}]", flush=True)


def emit_trace() -> None:
    """nullclaw cron skill-contract trace marker. Emit exactly once per run.

    Uses NULLCLAW_JOB_ID when set (real cron run); otherwise synthesises a
    manual-<unix_ts> trace id for ad-hoc invocations.
    """
    trace = JOB_ID or f"manual-{int(datetime.now(timezone.utc).timestamp())}"
    print(f"[trace:{trace}]", flush=True)


# ── subprocess + persona-core / nullclaw agent ───────────────────────


def _display_args(args: list[str]) -> str:
    """Redact long prompts in agent invocations so log lines stay readable.

    Pattern: when args looks like ``[nullclaw, agent, -m, <long-prompt>, ...]``
    replace the prompt with ``<prompt N chars>``. Other commands unchanged.
    """
    if len(args) >= 4 and args[0] == "nullclaw" and args[1] == "agent" and "-m" in args:
        shown = list(args)
        prompt_index = shown.index("-m") + 1
        if prompt_index < len(shown):
            shown[prompt_index] = f"<prompt {len(shown[prompt_index])} chars>"
        return " ".join(shown)
    return " ".join(args)


def run_cmd(
    args: list[str],
    *,
    timeout: int = 120,
    input_text: str | None = None,
    cwd: str | Path | None = None,
) -> str:
    """Subprocess wrapper with logging, timeout, and friendly error messages.

    Raises RuntimeError on non-zero exit. Returns stdout on success.
    The agent-prompt redaction in _display_args keeps log lines bounded.
    """
    log("$ " + _display_args(args))
    proc = subprocess.run(
        args,
        cwd=str(cwd) if cwd else None,
        input=input_text,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    if proc.returncode != 0:
        detail = proc.stderr.strip() or proc.stdout.strip() or f"exit {proc.returncode}"
        raise RuntimeError(f"{args[0]} failed: {detail}")
    return proc.stdout


def pc(*args: str, timeout: int = 120, input_text: str | None = None) -> str:
    """Convenience wrapper: run `persona-core <args>` and return stdout."""
    return run_cmd(["persona-core", *args], timeout=timeout, input_text=input_text)


def call_agent(
    prompt: str,
    *,
    timeout: int,
    body_marker: tuple[str, str] | None = None,
) -> str:
    """Run `nullclaw agent -m <prompt>` and return the agent's stdout.

    If body_marker is set (e.g., ("BEGIN_ISSUE_BODY", "END_ISSUE_BODY")),
    extract the substring between the markers. Use this when the agent is
    prompted to wrap its real output in markers so it can speak freely
    before/after the body without polluting downstream consumers.

    Raises RuntimeError if the agent returns empty body (post-extraction).
    """
    output = run_cmd(["nullclaw", "agent", "-m", prompt], timeout=timeout)
    if body_marker is not None:
        start, end = body_marker
        pattern = re.escape(start) + r"\s*(.*?)\s*" + re.escape(end)
        match = re.search(pattern, output, re.S)
        body = match.group(1).strip() if match else output.strip()
    else:
        body = output.strip()
    if not body:
        raise RuntimeError("agent returned an empty body")
    return body


# ── persona-core stdout parsing ──────────────────────────────────────


def parse_labels(text: str) -> dict[str, str]:
    """Parse persona-core's `key: value` plain-text output into a dict.

    Lines without a colon are skipped. Repeated keys keep the last value.
    Used for `streams issues prepare`, `validate-body`, `personas get` etc.
    where persona-core's stable contract is label-formatted text.
    """
    result: dict[str, str] = {}
    for line in text.splitlines():
        if ":" in line:
            key, _, value = line.partition(":")
            result[key.strip()] = value.strip()
    return result


# ── tempfile ─────────────────────────────────────────────────────────


def write_temp_text(prefix: str, suffix: str, text: str) -> Path:
    """Write text to a fresh tempfile and return its Path.

    Ensures trailing newline. Caller owns cleanup.
    """
    fd, raw_path = tempfile.mkstemp(prefix=prefix, suffix=suffix)
    path = Path(raw_path)
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        handle.write(text)
        if not text.endswith("\n"):
            handle.write("\n")
    return path
