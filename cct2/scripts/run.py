#!/usr/bin/env python3
"""CCT2: dual-LLM market sentiment analysis using Yahoo Finance data."""
import argparse
import json
import os
import subprocess
import sys
import textwrap
import urllib.request
import urllib.error
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone

JOB_ID = os.environ.get("NULLCLAW_JOB_ID", "")


def log(msg: str) -> None:
    prefix = f"[cct2/{JOB_ID}]" if JOB_ID else "[cct2]"
    print(f"{prefix} {msg}", file=sys.stderr)


SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
from delivery import deliver_or_fail
from trace_marker import emit_skill_status, emit_trace

SKILL_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SKILL_CONFIG_PATH = os.path.join(SKILL_DIR, "config.json")
NULLCLAW_CONFIG_PATH = os.path.expanduser("~/.nullclaw/config.json")

DEFAULT_TICKERS = ["AAPL", "MSFT", "GOOGL", "TSLA", "NVDA"]
DEFAULT_PRIMARY_PROVIDER = "anthropic-custom:minimax"
DEFAULT_PRIMARY_MODEL = "MiniMax-M2.7"
DEFAULT_BACKUP_PROVIDER = "glm-direct"   # direct BigModel Anthropic-compatible endpoint
DEFAULT_BACKUP_MODEL = "GLM-5.1"

# BigModel Anthropic-compatible endpoint (used by zcode, same API key as glm-cn)
BIGMODEL_ANTHROPIC_URL = "https://open.bigmodel.cn/api/anthropic/v1/messages"

SENTIMENT_EMOJI = {"bullish": "看漲 🟢", "bearish": "看跌 🔴", "neutral": "中性 ⚪"}


# ── Config ────────────────────────────────────────────────────────────────────

def load_secrets() -> dict:
    """Load ~/.secrets shell exports into a dict."""
    secrets = {}
    path = os.path.expanduser("~/.secrets")
    try:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line.startswith("export ") and "=" in line:
                    key, _, val = line[7:].partition("=")
                    secrets[key.strip()] = val.strip().strip("'\"")
    except Exception:
        pass
    return secrets


def load_skill_config() -> dict:
    try:
        with open(SKILL_CONFIG_PATH) as f:
            return json.load(f)
    except Exception:
        return {}


def load_tickers() -> list[str]:
    """Load tickers from nullclaw memory (key: cct2:tickers), fall back to default."""
    try:
        result = subprocess.run(
            ["nullclaw", "memory", "get", "cct2:tickers"],
            capture_output=True, text=True, timeout=10
        )
        if result.returncode == 0 and result.stdout.strip():
            # Output format: "cct2:tickers: AAPL MSFT GOOGL TSLA NVDA"
            raw = result.stdout.strip()
            # Strip key prefix if present
            if ":" in raw:
                parts = raw.split(":", 1)
                raw = parts[-1].strip() if len(parts) > 1 else raw
            tickers = [t.strip().upper() for t in raw.split() if t.strip()]
            if tickers:
                return tickers
    except Exception:
        pass
    return DEFAULT_TICKERS


# ── Market data ───────────────────────────────────────────────────────────────

def fetch_yahoo_quote(ticker: str) -> dict | None:
    """Fetch current/latest quote from Yahoo Finance query API."""
    url = f"https://query1.finance.yahoo.com/v8/finance/chart/{ticker}?interval=1d&range=2d"
    req = urllib.request.Request(url, headers={
        "User-Agent": "Mozilla/5.0 (compatible; nullclaw-cct2/1.0)",
        "Accept": "application/json",
    })
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            data = json.loads(resp.read().decode())
        result = data["chart"]["result"][0]
        meta = result["meta"]
        closes = result.get("indicators", {}).get("quote", [{}])[0].get("close", [])
        # Filter None values
        closes = [c for c in closes if c is not None]
        if len(closes) < 2:
            prev_close = meta.get("previousClose") or meta.get("chartPreviousClose")
            current = meta.get("regularMarketPrice") or (closes[-1] if closes else None)
        else:
            prev_close = closes[-2]
            current = closes[-1]

        if not current or not prev_close:
            return None

        pct = (current - prev_close) / prev_close * 100
        return {
            "ticker": ticker,
            "price": round(current, 2),
            "prev_close": round(prev_close, 2),
            "pct_change": round(pct, 2),
        }
    except Exception as e:
        log(f"WARN {ticker} quote fetch failed: {e}")
        return None


def fetch_yahoo_news(ticker: str, max_items: int = 3) -> list[str]:
    """Fetch recent news headlines from Yahoo Finance."""
    url = f"https://query2.finance.yahoo.com/v1/finance/search?q={ticker}&newsCount={max_items}&quotesCount=0"
    req = urllib.request.Request(url, headers={
        "User-Agent": "Mozilla/5.0 (compatible; nullclaw-cct2/1.0)",
        "Accept": "application/json",
    })
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            data = json.loads(resp.read().decode())
        headlines = []
        for item in data.get("news", [])[:max_items]:
            title = item.get("title", "").strip()
            if title:
                headlines.append(title)
        return headlines
    except Exception as e:
        log(f"WARN {ticker} news fetch failed: {e}")
        return []


def fetch_ticker_data(ticker: str) -> dict:
    quote = fetch_yahoo_quote(ticker)
    headlines = fetch_yahoo_news(ticker)
    return {"ticker": ticker, "quote": quote, "headlines": headlines}


def fetch_all(tickers: list[str]) -> dict[str, dict]:
    with ThreadPoolExecutor(max_workers=len(tickers)) as ex:
        futures = {ex.submit(fetch_ticker_data, t): t for t in tickers}
        results = {}
        for f in as_completed(futures):
            data = f.result()
            results[data["ticker"]] = data
    return results


# ── LLM sentiment ─────────────────────────────────────────────────────────────

SENTIMENT_PROMPT_TEMPLATE = """\
You are a financial analyst. Analyze the following market data and give your sentiment for each stock.

Mode: {mode}
Date: {date}

{ticker_data}

For each ticker, reply with ONLY a JSON object (no markdown, no explanation) like:
{{
  "AAPL": {{"sentiment": "bullish", "confidence": 0.82, "reason": "one sentence"}},
  "MSFT": {{"sentiment": "bearish", "confidence": 0.71, "reason": "one sentence"}},
  ...
}}

Rules:
- sentiment must be exactly: bullish, bearish, or neutral
- confidence is 0.0 to 1.0
- reason is one concise sentence under 80 characters
- Reply with the JSON object only, nothing else
"""


def build_ticker_summary(market_data: dict[str, dict], mode: str) -> str:
    lines = []
    for ticker, d in market_data.items():
        q = d.get("quote")
        if q:
            sign = "+" if q["pct_change"] >= 0 else ""
            lines.append(f"{ticker}: ${q['price']} ({sign}{q['pct_change']}% vs prev close ${q['prev_close']})")
        else:
            lines.append(f"{ticker}: price unavailable")
        headlines = d.get("headlines", [])
        if headlines:
            for h in headlines:
                lines.append(f"  - {h}")
        lines.append("")
    return "\n".join(lines)


def call_bigmodel_direct(model: str, prompt: str) -> dict | None:
    """Call BigModel Anthropic-compatible endpoint directly (no subprocess).
    On 429 (overload), retries once with glm-4-flash as a free-tier fallback."""
    secrets = load_secrets()
    api_key = secrets.get("BIGMODEL_API_KEY") or os.environ.get("BIGMODEL_API_KEY")
    if not api_key:
        # Fall back to nullclaw config glm-cn api_key
        try:
            with open(NULLCLAW_CONFIG_PATH) as f:
                cfg = json.load(f)
            api_key = cfg.get("models", {}).get("providers", {}).get("glm-cn", {}).get("api_key")
        except Exception:
            pass
    if not api_key:
        log("WARN glm-direct: no API key found")
        return None

    def _call(m: str) -> dict | None:
        payload = json.dumps({
            "model": m,
            "max_tokens": 512,
            "messages": [{"role": "user", "content": prompt}],
        }).encode()
        req = urllib.request.Request(
            BIGMODEL_ANTHROPIC_URL,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {api_key}",
                "anthropic-version": "2023-06-01",
            },
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=60) as resp:
            data = json.loads(resp.read().decode())
        text = data["content"][0]["text"].strip()
        return extract_json(text)

    try:
        return _call(model)
    except urllib.error.HTTPError as e:
        body = e.read().decode(errors="replace")[:200]
        if e.code == 429 and model != "glm-4-flash":
            log(f"WARN glm-direct {model} 429 (overload), retrying with glm-4-flash")
            try:
                return _call("glm-4-flash")
            except Exception as e2:
                log(f"WARN glm-direct glm-4-flash fallback failed: {e2}")
                return None
        log(f"WARN glm-direct HTTP {e.code}: {body}")
        return None
    except Exception as e:
        log(f"WARN glm-direct failed: {e}")
        return None


def extract_json(text: str) -> dict | None:
    """Extract the outermost JSON object from text, tolerating markdown code fences."""
    # Strip ```json ... ``` or ``` ... ``` fences
    stripped = text.strip()
    if stripped.startswith("```"):
        lines = stripped.splitlines()
        inner = "\n".join(lines[1:-1] if lines[-1].strip() == "```" else lines[1:])
        stripped = inner.strip()
    # Find outermost { ... } by tracking brace depth
    depth = 0
    start = None
    for i, ch in enumerate(stripped):
        if ch == "{":
            if depth == 0:
                start = i
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0 and start is not None:
                candidate = stripped[start:i + 1]
                try:
                    return json.loads(candidate)
                except json.JSONDecodeError:
                    # Try removing trailing commas (common LLM mistake)
                    import re
                    cleaned = re.sub(r",\s*([}\]])", r"\1", candidate)
                    try:
                        return json.loads(cleaned)
                    except json.JSONDecodeError:
                        pass
                start = None
    return None


def call_llm(provider: str, model: str, prompt: str) -> dict | None:
    """Call LLM and return parsed JSON sentiment dict."""
    if provider == "glm-direct":
        return call_bigmodel_direct(model, prompt)

    # nullclaw agent subprocess path
    cmd = ["nullclaw", "agent",
           "--provider", provider,
           "--model", model,
           "-m", prompt]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        output = result.stdout.strip()
        err_output = result.stderr.strip()
        if not output or result.returncode != 0:
            # Prefer "Last provider error:" from either stream; fall back to
            # first stderr line (stdout is mostly nullclaw info/debug noise).
            all_lines = output.splitlines() + err_output.splitlines()
            error_line = next(
                (l for l in all_lines if l.startswith("Last provider error:")),
                next((l for l in err_output.splitlines() if l.strip()), None)
                or (output.splitlines()[0] if output else "no output"),
            )
            log(f"WARN {provider}/{model} failed (rc={result.returncode}): {error_line[:120]}")
            return None
        parsed = extract_json(output)
        if parsed is None:
            log(f"WARN {provider}/{model} no JSON found in: {output[:200]}")
        return parsed
    except subprocess.TimeoutExpired:
        log(f"WARN {provider}/{model} timed out")
        return None
    except Exception as e:
        log(f"WARN {provider}/{model} call failed: {e}")
        return None


def run_dual_llm(prompt: str, cfg: dict) -> tuple[dict | None, dict | None]:
    """Run primary and backup LLM in parallel, return (primary_result, backup_result)."""
    primary_provider = cfg.get("primary_provider", DEFAULT_PRIMARY_PROVIDER)
    primary_model = cfg.get("primary_model", DEFAULT_PRIMARY_MODEL)
    backup_provider = cfg.get("backup_provider", DEFAULT_BACKUP_PROVIDER)
    backup_model = cfg.get("backup_model", DEFAULT_BACKUP_MODEL)

    with ThreadPoolExecutor(max_workers=2) as ex:
        f_primary = ex.submit(call_llm, primary_provider, primary_model, prompt)
        f_backup = ex.submit(call_llm, backup_provider, backup_model, prompt)
        primary_result = f_primary.result()
        backup_result = f_backup.result()

    return primary_result, backup_result


# ── Result merging ────────────────────────────────────────────────────────────

def merge_results(tickers: list[str], primary: dict | None, backup: dict | None) -> list[dict]:
    """
    For each ticker produce:
      ticker, consensus (bool), sentiment, confidence, reason,
      primary_sentiment, primary_confidence, primary_reason,
      backup_sentiment, backup_confidence, backup_reason
    """
    rows = []
    for t in tickers:
        p = (primary or {}).get(t, {})
        b = (backup or {}).get(t, {})

        p_sent = (p.get("sentiment") or "").lower()
        b_sent = (b.get("sentiment") or "").lower()
        p_conf = float(p.get("confidence") or 0)
        b_conf = float(b.get("confidence") or 0)
        p_reason = p.get("reason", "")
        b_reason = b.get("reason", "")

        if not p_sent and not b_sent:
            continue

        # Consensus: both present and agree on direction
        both_present = bool(p_sent and b_sent)
        agree = both_present and (p_sent == b_sent)

        if agree:
            avg_conf = (p_conf + b_conf) / 2
            rows.append({
                "ticker": t,
                "consensus": True,
                "sentiment": p_sent,
                "confidence": avg_conf,
                "reason": p_reason or b_reason,
                "primary_sentiment": p_sent, "primary_confidence": p_conf, "primary_reason": p_reason,
                "backup_sentiment": b_sent, "backup_confidence": b_conf, "backup_reason": b_reason,
            })
        else:
            # Use whichever is available; flag divergence if both present
            dominant_sent = p_sent or b_sent
            dominant_conf = p_conf if p_sent else b_conf
            rows.append({
                "ticker": t,
                "consensus": not both_present,  # only one answered → not a divergence
                "diverged": both_present,
                "sentiment": dominant_sent,
                "confidence": dominant_conf,
                "reason": p_reason or b_reason,
                "primary_sentiment": p_sent, "primary_confidence": p_conf, "primary_reason": p_reason,
                "backup_sentiment": b_sent, "backup_confidence": b_conf, "backup_reason": b_reason,
            })

    # Sort: diverged first, then by confidence desc
    rows.sort(key=lambda r: (0 if r.get("diverged") else 1, -r["confidence"]))
    return rows


# ── Formatting ────────────────────────────────────────────────────────────────

def fmt_sentiment(s: str) -> str:
    return SENTIMENT_EMOJI.get(s, s)


def fmt_conf(c: float) -> str:
    return f"{int(c * 100)}%"


def format_report(rows: list[dict], mode: str, tickers: list[str]) -> str:
    now_utc = datetime.now(timezone.utc)
    date_str = now_utc.strftime("%Y-%m-%d")
    label = "盤前報告" if mode == "pre-market" else "收盤報告"
    lines = [f"📊 CCT2 {label}｜{date_str}", ""]

    if not rows:
        lines.append("⚠️ 無法取得任何分析結果")
        return "\n".join(lines)

    consensus_rows = [r for r in rows if r.get("consensus") and not r.get("diverged")]
    diverged_rows = [r for r in rows if r.get("diverged")]
    solo_rows = [r for r in rows if not r.get("consensus") and not r.get("diverged")]

    if consensus_rows:
        lines.append("🎯 共識訊號")
        for r in consensus_rows:
            conf_str = fmt_conf(r["confidence"])
            reason = r["reason"]
            line = f"  • {r['ticker']} {fmt_sentiment(r['sentiment'])} {conf_str}"
            if reason:
                line += f" — {reason[:80]}"
            lines.append(line)

    if diverged_rows:
        if consensus_rows:
            lines.append("")
        lines.append("⚠️ 分歧訊號")
        for r in diverged_rows:
            lines.append(f"  • {r['ticker']}")
            if r["primary_sentiment"]:
                lines.append(f"      主模型：{fmt_sentiment(r['primary_sentiment'])} {fmt_conf(r['primary_confidence'])} — {r['primary_reason'][:70]}")
            if r["backup_sentiment"]:
                lines.append(f"      備用模型：{fmt_sentiment(r['backup_sentiment'])} {fmt_conf(r['backup_confidence'])} — {r['backup_reason'][:70]}")

    if solo_rows:
        if consensus_rows or diverged_rows:
            lines.append("")
        lines.append("📊 單一參考")
        for r in solo_rows:
            line = f"  • {r['ticker']} {fmt_sentiment(r['sentiment'])} {fmt_conf(r['confidence'])}"
            if r["reason"]:
                line += f" — {r['reason'][:80]}"
            lines.append(line)

    lines.append("")
    lines.append(f"分析標的：{len(tickers)} 支｜雙模型對照")
    return "\n".join(lines)


# ── Main ──────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="CCT2 dual-LLM market sentiment")
    parser.add_argument("--mode", required=True, choices=["pre-market", "eod"])
    parser.add_argument("--deliver-to", default=None)
    parser.add_argument("--account", default="main")
    args = parser.parse_args()

    cfg = load_skill_config()
    tickers = load_tickers()

    log(f"fetching data for {tickers}...")
    market_data = fetch_all(tickers)

    ticker_summary = build_ticker_summary(market_data, args.mode)
    date_str = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    mode_label = "pre-market (before US open, give outlook for today's session)" \
        if args.mode == "pre-market" \
        else "end-of-day (market just closed, summarize today and give tomorrow's outlook)"

    prompt = SENTIMENT_PROMPT_TEMPLATE.format(
        mode=mode_label,
        date=date_str,
        ticker_data=ticker_summary,
    )

    log("querying dual LLM...")
    primary, backup = run_dual_llm(prompt, cfg)

    rows = merge_results(tickers, primary, backup)
    msg = format_report(rows, args.mode, tickers)
    if JOB_ID:
        msg += f"\n\n`{JOB_ID}`"

    deliver_or_fail(args.deliver_to, msg, account=args.account)

    emit_skill_status("ok" if rows else "failed")
    emit_trace()


if __name__ == "__main__":
    main()
