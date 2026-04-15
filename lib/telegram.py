"""Shared Telegram delivery helper for nullclaw skills.

Single-attempt POST was upgraded to bounded retry (2026-04-13) to survive
transient 5xx / network hiccups without breaking long-tail skill timeouts.
See ~/.claude/plans/zazzy-petting-hejlsberg.md (B2) for the design.
"""
import json
import os
import socket
import sys
import time
import urllib.error
import urllib.request

CONFIG_PATH = os.path.expanduser("~/.nullclaw/config.json")

# Per-attempt urlopen timeout (seconds). The whole call is additionally
# bounded by `deadline_s` (caller) or DEFAULT_DEADLINE_S (fallback).
PER_ATTEMPT_TIMEOUT = 15.0

# Wall-clock cap when the caller does not pass `deadline_s`. Chosen so
# legacy callers cannot be silently surprised by a multi-minute blocking
# call after this upgrade. 30s comfortably covers 1 retry on a bad day.
DEFAULT_DEADLINE_S = 30.0

# Backoff schedule between attempts (seconds). len() = max retries.
BACKOFFS = (2.0, 5.0)


def get_bot_token(account: str = "main") -> str | None:
    """Read Telegram bot token from nullclaw config."""
    try:
        with open(CONFIG_PATH) as f:
            cfg = json.load(f)
        return (
            cfg.get("channels", {})
               .get("telegram", {})
               .get("accounts", {})
               .get(account, {})
               .get("bot_token")
        )
    except Exception:
        return None


def _is_retryable_http(code: int) -> bool:
    return code == 429 or 500 <= code < 600


def _log(msg: str) -> None:
    print(f"[telegram] {msg}", file=sys.stderr, flush=True)


def send(
    chat_id: str,
    text: str,
    account: str = "main",
    *,
    deadline_s: float | None = None,
) -> bool:
    """Send a message to a Telegram chat. Returns True on success.

    Retries up to 3 attempts on retryable errors (URLError, socket.timeout,
    HTTP 5xx, HTTP 429). Never retries on permanent 4xx (bad token, bad
    chat_id).

    The retry loop is bounded by `deadline_s` (a wall-clock budget). When
    omitted the cap is DEFAULT_DEADLINE_S so legacy callers stay bounded.
    Per-attempt timeout is clamped to min(PER_ATTEMPT_TIMEOUT, remaining).
    """
    token = get_bot_token(account)
    if not token:
        return False

    url = f"https://api.telegram.org/bot{token}/sendMessage"
    payload = json.dumps({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "Markdown",
        "disable_web_page_preview": True,
    }).encode()

    budget = float(deadline_s) if deadline_s is not None else DEFAULT_DEADLINE_S
    if budget <= 0:
        _log(f"deadline already exhausted (budget={budget:.1f}s); skipping send")
        return False

    start = time.monotonic()
    max_attempts = 1 + len(BACKOFFS)

    for attempt in range(1, max_attempts + 1):
        elapsed = time.monotonic() - start
        remaining = budget - elapsed
        if remaining <= 0:
            _log(f"deadline exhausted before attempt {attempt}/{max_attempts}")
            return False
        per_attempt = min(PER_ATTEMPT_TIMEOUT, remaining)

        req = urllib.request.Request(
            url,
            data=payload,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=per_attempt) as resp:
                if resp.status == 200:
                    return True
                # 2xx-non-200 is unexpected for sendMessage; treat as failure.
                _log(f"unexpected status {resp.status} on attempt {attempt}/{max_attempts}")
                return False
        except urllib.error.HTTPError as e:
            body = ""
            try:
                body = e.read().decode(errors="replace")[:200]
            except Exception:
                pass
            if not _is_retryable_http(e.code):
                _log(f"permanent HTTP {e.code} on attempt {attempt}/{max_attempts}: {body}")
                return False
            _log(
                f"attempt {attempt}/{max_attempts} got HTTP {e.code}"
                f" (remaining={budget - (time.monotonic() - start):.1f}s): {body}"
            )
        except (urllib.error.URLError, socket.timeout, TimeoutError) as e:
            _log(
                f"attempt {attempt}/{max_attempts} got {type(e).__name__}"
                f" (remaining={budget - (time.monotonic() - start):.1f}s): {e}"
            )
        except Exception as e:
            _log(f"non-retryable error on attempt {attempt}/{max_attempts}: {type(e).__name__}: {e}")
            return False

        # If this was the last attempt, don't sleep — fall out of loop.
        if attempt >= max_attempts:
            break
        backoff = BACKOFFS[attempt - 1]
        if backoff > 0:
            remaining_budget = max(0.0, budget - (time.monotonic() - start))
            if remaining_budget <= 0:
                _log("no time left for backoff; giving up")
                return False
            time.sleep(min(backoff, remaining_budget))

    _log(f"all {max_attempts} attempts failed")
    return False
