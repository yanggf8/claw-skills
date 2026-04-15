"""Shared Telegram delivery helper for nullclaw / openclaw skills."""
import json
import os
import urllib.request
import urllib.error

DEFAULT_CONFIG_PATH = os.path.expanduser("~/.nullclaw/config.json")


def _resolve_config_path(config_path: str | None) -> str:
    return config_path or os.environ.get("CLAW_CONFIG") or DEFAULT_CONFIG_PATH


def get_bot_token(account: str = "main", config_path: str | None = None) -> str | None:
    """Read Telegram bot token from config. Resolution order:
       explicit config_path arg → $CLAW_CONFIG → ~/.nullclaw/config.json.
       Supports both nullclaw multi-account schema and openclaw single-token schema."""
    try:
        with open(_resolve_config_path(config_path)) as f:
            cfg = json.load(f)
    except Exception:
        return None
    telegram_cfg = cfg.get("channels", {}).get("telegram", {})
    nullclaw_token = (
        telegram_cfg.get("accounts", {}).get(account, {}).get("bot_token")
    )
    if nullclaw_token:
        return nullclaw_token
    return telegram_cfg.get("botToken")


def send(chat_id: str, text: str, account: str = "main", config_path: str | None = None) -> bool:
    """Send a message to a Telegram chat. Returns True on success."""
    token = get_bot_token(account, config_path)
    if not token:
        return False
    url = f"https://api.telegram.org/bot{token}/sendMessage"
    payload = json.dumps({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "Markdown",
        "disable_web_page_preview": True,
    }).encode()
    req = urllib.request.Request(
        url,
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return resp.status == 200
    except urllib.error.HTTPError as e:
        body = e.read().decode(errors="replace")
        print(f"[WARN: telegram delivery failed {e.code}] {body}", flush=True)
        return False
    except Exception as e:
        print(f"[WARN: telegram delivery failed] {e}", flush=True)
        return False
