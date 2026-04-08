"""Shared Telegram delivery helper for nullclaw skills."""
import json
import os
import urllib.request
import urllib.error

CONFIG_PATH = os.path.expanduser("~/.nullclaw/config.json")


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


def send(chat_id: str, text: str, account: str = "main") -> bool:
    """Send a message to a Telegram chat. Returns True on success."""
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
