#!/usr/bin/env python3
"""News skill: fetch Google News RSS feeds and format a daily summary."""
import argparse
import json
import os
import re
import sys
import tempfile
import time
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from datetime import datetime, timezone, timedelta

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
from delivery import deliver_or_fail
from trace_marker import emit_skill_status, emit_trace
import news_quality

TOPICS_FILE = os.path.expanduser("~/.nullclaw/news-topics.json")
TRACE_FILE = os.path.expanduser("~/.nullclaw/skill-traces.jsonl")
NEWS_CACHE_DIR = os.path.expanduser("~/.nullclaw/.news-cache")
NEWS_FAILURE_LOG = os.path.expanduser("~/.nullclaw/news-failures.log")
NEWS_CACHE_TTL_DAYS = 7
NEWS_FAILURE_LOG_MAX_BYTES = 1_048_576  # 1 MiB; rotate to .1 then truncate
LLM_ITEM_LIMITS = {
    "ai": 30,
    "tech": 12,
    "general": 8,
}
LLM_CUSTOM_TOPIC_LIMIT = 8
LLM_DEFAULT_TIMEOUT_SECS = 180
LLM_CUSTOM_TIMEOUT_SECS = 180
LLM_SECTION_TIMEOUT_SECS = 90
LLM_TRANSLATION_TIMEOUT_SECS = 60
TELEGRAM_RAW_CHUNK_LIMIT = 3800
AI_SUBSTAGE_CACHE_VARIANT = "default_ai_clustered_v3_precheck"

# Content prechecking (lib/news_quality). Tier 1 = deterministic, no network,
# pre-LLM. Tier 2 = body precheck of LLM-picked items, before link attach.
# Off-by-env so cron can disable without code edits: NEWS_PRECHECK=0
PRECHECK_ENABLED = os.environ.get("NEWS_PRECHECK", "1") != "0"
PRECHECK_DECODE_TIMEOUT = float(os.environ.get("NEWS_PRECHECK_DECODE_TIMEOUT", "5"))
PRECHECK_FETCH_TIMEOUT = float(os.environ.get("NEWS_PRECHECK_FETCH_TIMEOUT", "5"))
PRECHECK_TOTAL_DEADLINE = float(os.environ.get("NEWS_PRECHECK_DEADLINE", "25"))
PRECHECK_MAX_WORKERS = int(os.environ.get("NEWS_PRECHECK_WORKERS", "6"))
# Paywall free-replacement lookup: enabled by default, bounded so it can never
# push a cron run past its kill window. Set NEWS_PAYWALL_REPLACE=0 to disable
# (falls back to the single-bullet 付費牆 note).
PAYWALL_REPLACE_ENABLED = os.environ.get("NEWS_PAYWALL_REPLACE", "1") != "0"
PAYWALL_REPLACE_DEADLINE = float(os.environ.get("NEWS_PAYWALL_REPLACE_DEADLINE", "20"))
PAYWALL_REPLACE_MAX = int(os.environ.get("NEWS_PAYWALL_REPLACE_MAX", "4"))
PAYWALL_REPLACE_SOURCES = tuple(
    s.strip().lower()
    for s in os.environ.get("NEWS_PAYWALL_REPLACE_SOURCES", "google,bing").split(",")
    if s.strip()
)
PAYWALL_REPLACE_BING_MKT = os.environ.get("NEWS_PAYWALL_REPLACE_BING_MKT", "en-US")
# Substaging: each Level-2 half (or Level-3 quarter) gets a smaller timeout
# than the original 90s monolithic call — half-size prompts should not need it.
AI_SUBSTAGE_TIMEOUT_SECS = 60
# On a synthetic-timeout (rc=124) the provider stalled after stream-start. A
# single retry recovers a transient stall (the next attempt almost always lands
# in seconds on a healthy provider). The retry uses a SHORTER budget so a wedged
# provider cannot push the whole multi-topic run past the cron kill window: a
# stalled call already cost AI_SUBSTAGE_TIMEOUT_SECS, so the retry caps the extra.
LLM_RETRY_TIMEOUT_SECS = int(os.environ.get("NEWS_LLM_RETRY_TIMEOUT", "30"))
DEFAULT_SECTION_SPECS = {
    "ai": {
        "header": "**🤖 AI 人工智慧**",
        "limit": 30,
        "fallback_limit": 8,
        "pick": "5-8",
        "focus": "重大研究突破、政策變化、產品發布、產業併購、國安與監管等真正有影響力的 AI 新聞",
    },
    "tech": {
        "header": "**💻 科技 & 半導體**",
        "limit": 12,
        "fallback_limit": 5,
        "pick": "3-5",
        "focus": "半導體、晶片、消費電子、太空科技與重要非 AI 科技新聞",
    },
    "general": {
        "header": "**🌏 重大新聞**",
        "limit": 8,
        "fallback_limit": 3,
        "pick": "2-3",
        "focus": "最重大的非科技一般新聞",
    },
}

# Default topics for accounts without a stored preference
DEFAULT_TOPICS = {
    "main": None,  # None means use the hardcoded AI/tech/general feeds
}

_TOPIC_STOPWORDS = {
    "the", "a", "an", "and", "or", "to", "of", "for", "in", "on", "with",
    "new", "ai", "is", "are", "be", "at", "from", "your", "you", "our",
    "its", "it", "more", "all", "how", "why", "what", "as", "by", "this",
    "的", "是", "了", "在", "和", "與", "及", "也", "都", "就", "而", "對",
    "為", "以", "從", "把", "被", "將", "這", "那", "有", "沒",
}
_CJK_STOP_CHARS = {word for word in _TOPIC_STOPWORDS if len(word) == 1 and "\u3400" <= word <= "\ufaff"}
_CJK_STOP_BIGRAMS = {
    "公司", "發布", "布新", "新產", "產品", "股價", "上漲", "下跌",
}
_CLUSTER_OVERLAP = 2

TRANSLATION_RULES_STRICT = (
    "英文標題必須完整翻譯成繁體中文。"
    "只有以下類別可以保留英文原文：公司名（例如 OpenAI、Google、Microsoft）、"
    "人名（例如 Sam Altman）、產品名（例如 ChatGPT、Gemini）、"
    "既定技術術語（例如 AGI、GPU、API、LLM）。"
    "所有普通英文詞彙必須翻譯，包括但不限於副詞（increasingly、significantly、rapidly、notably、effectively）、"
    "動詞、形容詞、連接詞。"
    "輸出中不得保留任何非上述四類的英文單字。"
)


def log_trace(event: str, **fields) -> None:
    """Append structured skill diagnostics without logging secrets."""
    entry = {
        "ts": datetime.now(timezone.utc).isoformat(),
        "job_id": os.environ.get("NULLCLAW_JOB_ID", "interactive"),
        "skill": "news",
        "event": event,
        **fields,
    }
    try:
        with open(TRACE_FILE, "a", encoding="utf-8") as f:
            f.write(json.dumps(entry, ensure_ascii=False) + "\n")
    except OSError as e:
        print(f"[WARN] trace write failed: {e}", file=sys.stderr)


class AlertContext:
    """Captures who to alert when the skill cannot send news.

    Built once in main() from CLI args; threaded explicitly through the
    summarize_* functions so a reader can see exactly which call sites can
    fail-alert. Avoids a global state holder.
    """
    __slots__ = ("deliver_to", "account", "job_id")

    def __init__(self, deliver_to, account, job_id):
        self.deliver_to = deliver_to
        self.account = account
        self.job_id = job_id or "interactive"


def _news_cache_sweep() -> None:
    """Delete cache subdirectories older than NEWS_CACHE_TTL_DAYS. Best-effort."""
    import shutil
    # Also prune the precheck decode cache (separate dir) so it doesn't grow forever.
    try:
        news_quality.sweep_decode_cache(NEWS_CACHE_TTL_DAYS)
    except Exception:
        pass
    if not os.path.isdir(NEWS_CACHE_DIR):
        return
    cutoff = time.time() - NEWS_CACHE_TTL_DAYS * 86400
    try:
        for entry in os.listdir(NEWS_CACHE_DIR):
            p = os.path.join(NEWS_CACHE_DIR, entry)
            try:
                if os.path.isdir(p) and os.path.getmtime(p) < cutoff:
                    shutil.rmtree(p, ignore_errors=True)
            except OSError:
                pass
    except OSError:
        pass


def _news_cache_path(date_str: str, variant: str, start: int, end: int) -> str:
    # date_str format: "YYYY/MM/DD (Mon)" -> safe component "YYYY-MM-DD"
    safe_date = date_str.split()[0].replace("/", "-")
    d = os.path.join(NEWS_CACHE_DIR, safe_date)
    os.makedirs(d, exist_ok=True)
    return os.path.join(d, f"{variant}-{start:03d}-{end:03d}.txt")


def _news_cache_get(date_str: str, variant: str, start: int, end: int):
    path = _news_cache_path(date_str, variant, start, end)
    try:
        with open(path, encoding="utf-8") as f:
            data = f.read()
        log_trace("news_cache_hit", variant=variant, start=start, end=end)
        return data
    except FileNotFoundError:
        return None
    except OSError as e:
        log_trace("news_cache_read_error", variant=variant, error=str(e))
        return None


def _news_cache_put(date_str: str, variant: str, start: int, end: int, body: str) -> None:
    path = _news_cache_path(date_str, variant, start, end)
    try:
        with open(path, "w", encoding="utf-8") as f:
            f.write(body)
        log_trace("news_cache_write", variant=variant, start=start, end=end, bytes=len(body))
    except OSError as e:
        log_trace("news_cache_write_error", variant=variant, error=str(e))


def _recent_failure_count(reason: str, account: str, days: int) -> int:
    """Count prior NEWS_FAILURE_LOG blocks for this (reason, account) in the
    trailing `days` window. Best-effort: any parse/IO error returns 0.

    Purpose: a single degraded run looks benign, but the same alert firing N
    times over weeks is a chronic problem (e.g. an LLM that thinking-stalls)
    that nobody notices unless the count is surfaced. This count is folded into
    the alert detail so the trend rides along with the alert itself — no metrics
    system, just the durable text log we already write.
    """
    cutoff = datetime.now(timezone(timedelta(hours=8))) - timedelta(days=days)
    count = 0
    for path in (NEWS_FAILURE_LOG, NEWS_FAILURE_LOG + ".1"):
        try:
            with open(path, encoding="utf-8") as f:
                text = f.read()
        except OSError:
            continue
        # Blocks are "=== <ts> CST ===\n...reason: <r>\naccount: <a>...".
        for block in text.split("=== "):
            head, _, body = block.partition(" CST ===")
            if not body:
                continue
            try:
                when = datetime.strptime(head.strip(), "%Y-%m-%d %H:%M:%S").replace(
                    tzinfo=timezone(timedelta(hours=8))
                )
            except ValueError:
                continue
            if when < cutoff:
                continue
            if f"reason: {reason}\n" in body and f"account: {account}\n" in body:
                count += 1
    return count


def _alert_failure(ctx: "AlertContext", reason: str, detail: str) -> None:
    """Notify the operator that the news skill could not deliver news.

    Always best-effort — never raises:
      1. Append a plain-text record to NEWS_FAILURE_LOG (durable on-disk).
      2. Send a Telegram message to ctx.deliver_to (immediate, may fail).

    Order matters: file log first so the failure record survives even if
    Telegram itself is the failure mode.
    """
    # Count prior occurrences BEFORE writing this block, so the trend reads as
    # "this happened N times in the last 30d" (excluding the current alert).
    prior_30d = _recent_failure_count(reason, ctx.account, days=30)
    if prior_30d:
        detail = f"{detail} [此告警近30天已出現 {prior_30d} 次]"

    ts = datetime.now(timezone(timedelta(hours=8))).strftime("%Y-%m-%d %H:%M:%S")
    block = (
        f"=== {ts} CST ===\n"
        f"job_id: {ctx.job_id}\n"
        f"deliver_to: {ctx.deliver_to or '(none)'}\n"
        f"account: {ctx.account}\n"
        f"reason: {reason}\n"
        f"detail: {detail}\n"
        "\n"
    )

    # 1. Durable on-disk log. Rotate at NEWS_FAILURE_LOG_MAX_BYTES.
    try:
        if os.path.exists(NEWS_FAILURE_LOG) and os.path.getsize(NEWS_FAILURE_LOG) > NEWS_FAILURE_LOG_MAX_BYTES:
            try:
                os.replace(NEWS_FAILURE_LOG, NEWS_FAILURE_LOG + ".1")
            except OSError:
                pass
        with open(NEWS_FAILURE_LOG, "a", encoding="utf-8") as f:
            f.write(block)
    except OSError as e:
        log_trace("news_failure_log_error", error=str(e))

    log_trace("news_failure_alert", reason=reason, detail_chars=len(detail))

    # 2. Best-effort Telegram. If chat_id is unset (interactive run), skip.
    if not ctx.deliver_to:
        return
    try:
        msg = (
            f"⚠️ 新聞無法送出 — {ts}\n"
            f"原因：{reason}\n"
            f"細節：{detail[:500]}\n"
            f"job_id: {ctx.job_id}"
        )
        deliver_or_fail(
            ctx.deliver_to,
            msg,
            account=ctx.account,
            fail_on_delivery_error=False,  # we are already in a failure path
        )
    except Exception as e:
        log_trace("news_failure_alert_telegram_error", error=str(e))


def load_topics() -> dict[str, list[str]]:
    """Load per-account topic preferences from JSON file."""
    try:
        with open(TOPICS_FILE, encoding="utf-8") as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def save_topics(data: dict[str, list[str]]) -> None:
    """Atomically write topic preferences (write-to-temp-then-rename)."""
    dir_path = os.path.dirname(TOPICS_FILE)
    fd, tmp_path = tempfile.mkstemp(dir=dir_path, suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
            f.write("\n")
        os.rename(tmp_path, TOPICS_FILE)
    except Exception:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass
        raise


def manage_list(account: str) -> str:
    """List subscribed topics for an account."""
    data = load_topics()
    topics = data.get(account)
    if not topics:
        if account == "main":
            return f"📰 {account} 的新聞訂閱：AI、科技半導體、一般新聞（預設）"
        return f"📰 {account} 尚未設定新聞主題"
    return f"📰 {account} 的新聞訂閱：\n" + "\n".join(f"  • {t}" for t in topics)


def manage_add(account: str, topic: str) -> str:
    """Add a topic to an account's subscription (idempotent)."""
    topic = topic.strip()
    if not topic:
        return "請提供要新增的主題名稱"
    data = load_topics()
    topics = data.get(account, [])
    if topic in topics:
        return f"✅ 主題「{topic}」已在訂閱中"
    topics.append(topic)
    data[account] = topics
    save_topics(data)
    return f"✅ 已新增主題「{topic}」\n目前訂閱：{'、'.join(topics)}"


def manage_remove(account: str, topic: str) -> str:
    """Remove a topic from an account's subscription."""
    topic = topic.strip()
    if not topic:
        return "請提供要移除的主題名稱"
    data = load_topics()
    topics = data.get(account, [])
    if topic not in topics:
        return f"⚠️ 主題「{topic}」不在訂閱中"
    topics.remove(topic)
    data[account] = topics
    save_topics(data)
    if topics:
        return f"✅ 已移除主題「{topic}」\n目前訂閱：{'、'.join(topics)}"
    return f"✅ 已移除主題「{topic}」\n目前無訂閱主題（將使用預設新聞）"


def load_env():
    env_path = os.environ.get("CLAW_ENV") or os.path.expanduser("~/.nullclaw/.env")
    if os.path.exists(env_path):
        with open(env_path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                key, _, val = line.partition("=")
                key = key.strip()
                val = val.strip().strip('"').strip("'")
                if key not in os.environ:
                    os.environ[key] = val


FEEDS = {
    # AI — broad US coverage (research, policy, industry)
    "ai_us": "https://news.google.com/rss/search?q=artificial+intelligence+AI+breakthrough+OR+regulation+OR+research+when:1d&hl=en-US&gl=US&ceid=US:en",
    # AI — major labs and products
    "ai_labs": "https://news.google.com/rss/search?q=OpenAI+OR+Anthropic+OR+Google+DeepMind+OR+Meta+AI+OR+xAI+when:1d&hl=en-US&gl=US&ceid=US:en",
    # AI — China (English coverage of China AI)
    "ai_cn": "https://news.google.com/rss/search?q=China+AI+OR+Baidu+AI+OR+DeepSeek+OR+Alibaba+AI+OR+ByteDance+AI+when:1d&hl=en-US&gl=US&ceid=US:en",
    # AI — Taiwan local
    "ai_tw": "https://news.google.com/rss/search?q=AI+when:1d&hl=zh-TW&gl=TW&ceid=TW:zh-Hant",
    # Tech & semiconductor
    "tech": "https://news.google.com/rss/search?q=%E7%A7%91%E6%8A%80+%E5%8D%8A%E5%B0%8E%E9%AB%94+%E6%99%B6%E7%89%87+when:1d&hl=zh-TW&gl=TW&ceid=TW:zh-Hant",
    # General Taiwan
    "general": "https://news.google.com/rss?hl=zh-TW&gl=TW&ceid=TW:zh-Hant",
}


def fetch_feed(url: str, max_items: int = 15, timeout: float = 15.0) -> list[dict]:
    """Fetch RSS feed and return list of {title, link, pub_date, source_name}."""
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "nullclaw-news/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = resp.read()
    except Exception as e:
        print(f"[WARN] fetch failed: {url[:60]}... {e}", file=sys.stderr)
        return []

    items = []
    try:
        root = ET.fromstring(data)
        for item in root.findall(".//item")[:max_items]:
            title = item.findtext("title", "").strip()
            link = item.findtext("link", "").strip()
            pub = item.findtext("pubDate", "").strip()
            if title:
                items.append({
                    "title": title,
                    "link": link,
                    "pub_date": pub,
                    "source_name": _extract_source_name(title),
                })
    except ET.ParseError:
        pass
    return items


def _extract_source_name(title: str) -> str:
    if " - " in title:
        return title.rsplit(" - ", 1)[-1].strip()
    return ""


def dedup(items: list[dict]) -> list[dict]:
    """Remove duplicate titles (case-insensitive)."""
    seen = set()
    result = []
    for item in items:
        key = item["title"].lower()
        if key not in seen:
            seen.add(key)
            result.append(item)
    return result


def _title_without_source(title: str) -> str:
    if " - " in title:
        return title.rsplit(" - ", 1)[0]
    return title


def _topic_words(title: str) -> set[str]:
    """Significant headline tokens used for deterministic topic clustering."""
    text = _title_without_source(title).lower()
    words = {
        word
        for word in re.findall(r"[a-z0-9.]+", text)
        if len(word) > 2 and word not in _TOPIC_STOPWORDS
    }
    for run in re.findall(r"[\u3400-\u9fff\uf900-\ufaff]+", text):
        for i in range(len(run) - 1):
            pair = run[i:i + 2]
            if pair[0] in _CJK_STOP_CHARS or pair[1] in _CJK_STOP_CHARS:
                continue
            if pair in _CJK_STOP_BIGRAMS:
                continue
            words.add(pair)
    return words


def cluster(items: list[dict]) -> list[list[dict]]:
    """Group headlines that cover the same event by token overlap."""
    clusters: list[dict] = []
    for item in items:
        words = _topic_words(item.get("title", ""))
        for group in clusters:
            if len(words & group["seed_words"]) >= _CLUSTER_OVERLAP:
                group["items"].append(item)
                break
        else:
            # A seed with fewer than _CLUSTER_OVERLAP tokens cannot grow; that
            # is intentional because one-token headlines are too weak to anchor
            # a deterministic event cluster.
            clusters.append({"seed_words": set(words), "items": [item]})
    clusters.sort(key=lambda group: len(group["items"]), reverse=True)
    return [group["items"] for group in clusters]


def pick_representatives(clusters: list[list[dict]], *, per_cluster: int = 1) -> list[dict]:
    """Choose representatives from already-ranked clusters."""
    ranked: list[dict] = []
    for group in clusters:
        ranked.extend(group[:per_cluster])
    return ranked


def _build_link_map(all_items: dict[str, list[dict]]) -> dict[str, str]:
    """Build title→link mapping for post-processing."""
    link_map = {}
    for items in all_items.values():
        for it in items:
            title = it["title"].strip()
            link = it.get("link", "")
            if title and link:
                link_map[title] = link
    return link_map


def _attach_links(summary: str, link_map: dict[str, str]) -> str:
    """Attach links to news lines in the summary via fuzzy title match."""
    lines = summary.split("\n")
    result = []
    for line in lines:
        if line.startswith("- ") and "[🔗]" not in line and "http" not in line:
            title_text = line[2:].strip()
            # Try exact match first, then substring match
            link = link_map.get(title_text)
            if not link:
                for orig_title, orig_link in link_map.items():
                    if orig_title in title_text or title_text in orig_title:
                        link = orig_link
                        break
            safe_title = _neutralize_markdown_specials(title_text)
            if link:
                result.append(f"- {safe_title} [🔗]({link})")
            else:
                result.append(f"- {safe_title}")
        else:
            result.append(line)
    return "\n".join(result)


def _neutralize_markdown_specials(text: str) -> str:
    """Replace Markdown special chars with full-width visual equivalents.

    Telegram's legacy Markdown parser rejects messages with unmatched *, _,
    backtick, or [. Headlines occasionally contain these literally (e.g.
    Taiwanese stock notation "長科*成關鍵受惠股"). Substituting full-width
    forms keeps the headline visually identical while removing the
    Markdown-control meaning.

    Only call on headline body text, not on scaffolding (section headers,
    link markup, chunk prefixes).
    """
    return text.replace("*", "＊").replace("_", "＿")


def _news_bullet_lines(summary: str) -> list[str]:
    """Return news item bullets that should carry source markers."""
    import re

    lines = []
    for line in summary.splitlines():
        stripped = line.strip()
        if stripped in ("---", "--", "***"):
            continue
        if not stripped.startswith("-") and not re.match(r"^#\d+\b(?!,)", stripped):
            continue
        body = stripped[1:].strip() if stripped.startswith("-") else stripped
        if not body or body.startswith("...") or "今日無相關新聞" in body:
            continue
        lines.append(line)
    return lines


def _marker_validation_stats(summary: str, numbered: dict[int, dict]) -> tuple[int, int]:
    """Return (marked_bullets, total_news_bullets) for valid leading #N markers."""
    import re

    marked = 0
    bullet_lines = _news_bullet_lines(summary)
    for line in bullet_lines:
        match = re.match(r"^\s*(?:-\s*)?#(\d+)\b(?!,)", line)
        if match and int(match.group(1)) in numbered:
            marked += 1
    return marked, len(bullet_lines)


def _extract_leading_marker_ids(summary: str, numbered: dict[int, dict]) -> list[int]:
    import re

    seen = set()
    ids = []
    for line in _news_bullet_lines(summary):
        match = re.match(r"^\s*(?:-\s*)?#(\d+)\b(?!,)", line)
        if not match:
            continue
        num = int(match.group(1))
        if num in numbered and num not in seen:
            ids.append(num)
            seen.add(num)
    return ids


def _count_cjk(text: str) -> int:
    return sum(1 for ch in text if "\u4e00" <= ch <= "\u9fff")


def _strip_marker_prefix(line: str) -> str:
    import re

    return re.sub(r"^\s*(?:-\s*)?#\d+\b(?!,)\s*", "", line).strip()


FORBIDDEN_NON_PROPER_ENGLISH = frozenset({
    "increasingly", "significantly", "rapidly", "notably", "effectively",
    "essentially", "generally", "particularly", "specifically", "primarily",
    "ultimately", "eventually", "additionally", "furthermore", "moreover",
    "however", "therefore", "consequently", "meanwhile", "subsequently",
    "previously", "currently", "recently", "approximately", "potentially",
})


def _language_validation_stats(summary: str) -> tuple[int, int]:
    """Return (Chinese-looking bullets, total news bullets)."""
    chinese = 0
    bullet_lines = _news_bullet_lines(summary)
    for line in bullet_lines:
        body = _strip_marker_prefix(line)
        first_cjk = next((idx for idx, ch in enumerate(body) if "\u4e00" <= ch <= "\u9fff"), -1)
        if _count_cjk(body) >= 2 and first_cjk >= 0 and first_cjk <= 18:
            chinese += 1
    return chinese, len(bullet_lines)


def _language_validation_passed(summary: str) -> bool:
    chinese, total = _language_validation_stats(summary)
    if total == 0:
        return False
    if not (chinese * 5 >= total * 4):
        return False
    # Reject any bullet containing common English adverbs that are never proper nouns
    import re
    import string
    bullet_lines = _news_bullet_lines(summary)
    for line in bullet_lines:
        body = _strip_marker_prefix(line)
        # Split on whitespace, strip punctuation, lowercase
        for token in body.split():
            # Strip leading/trailing punctuation
            cleaned = token.strip(string.punctuation)
            if cleaned.lower() in FORBIDDEN_NON_PROPER_ENGLISH:
                return False
    return True


def _number_items_for_prompt(
    all_items: dict[str, list[dict]],
    labels: list[str] | None = None,
    limits: dict[str, int] | None = None,
) -> tuple[dict[int, dict], str]:
    """Build the numbered LLM prompt input, capped to keep summarization responsive."""
    numbered = {}
    sections = []
    idx = 1
    label_iter = labels if labels is not None else list(all_items.keys())
    for label in label_iter:
        items = all_items.get(label, [])
        if not items:
            continue
        limit = (limits or {}).get(label, len(items))
        lines = []
        for it in items[:limit]:
            numbered[idx] = {
                "title": it["title"],
                "link": it.get("link", ""),
                "source_name": it.get("source_name", ""),
            }
            lines.append(f"  #{idx} {it['title']}")
            idx += 1
        sections.append(f"[{label}]\n" + "\n".join(lines))
    return numbered, "\n".join(sections)


def _clip_subprocess_text(value, limit: int = 500) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        value = value.decode("utf-8", errors="replace")
    return str(value).strip()[:limit]


def _sample_nonempty_lines(value: str, limit: int = 8) -> list[str]:
    lines = []
    for line in value.splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        lines.append(stripped[:240])
        if len(lines) >= limit:
            break
    return lines


def _log_llm_validation_failed(
    variant: str,
    result,
    summary: str,
    marked_bullets: int,
    total_bullets: int,
    numbered: dict[int, dict],
    reason: str,
    extra: dict | None = None,
) -> None:
    fields = {
        "variant": variant,
        "reason": reason,
        "returncode": getattr(result, "returncode", None),
        "stdout_len": len(getattr(result, "stdout", "") or ""),
        "stderr_len": len(getattr(result, "stderr", "") or ""),
        "items_numbered": len(numbered),
        "marked_bullets": marked_bullets,
        "total_bullets": total_bullets,
        "stdout_sample": _clip_subprocess_text(summary, 1200),
        "line_sample": _sample_nonempty_lines(summary, 8),
        "bullet_sample": [line.strip()[:240] for line in _news_bullet_lines(summary)[:8]],
    }
    if extra:
        fields.update(extra)
    log_trace("llm_validation_failed", **fields)


def _llm_source_item_counts(all_items: dict[str, list[dict]]) -> dict[str, int]:
    return {label: len(items) for label, items in all_items.items()}


def _llm_retry_budget_secs() -> float | None:
    """Remaining wall-clock budget for an LLM retry, from the cron env vars.

    Mirrors lib/delivery._resolve_delivery_deadline. The scheduler sets:
      NULLCLAW_SKILL_TIMEOUT  — the skill's overall timeout, seconds
      NULLCLAW_SKILL_STARTED  — monotonic time the skill started, seconds (optional)

    Returns None when no budget is configured (manual runs / no cron env) — the
    caller then treats the retry as always permitted. Reserves 2s headroom so the
    retry cannot itself overrun the skill's hard kill.
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
            return max(0.0, timeout - elapsed - 2.0)
        except ValueError:
            pass
    return max(0.0, timeout - 2.0)


def _run_nullclaw_agent(prompt: str, timeout_secs: int, variant: str, all_items: dict[str, list[dict]], numbered: dict[int, dict]):
    """Run one agent call; on a synthetic timeout (rc=124) retry ONCE with a
    shorter budget. Only rc=124 is retried — validation failures, empty stdout,
    and other nonzero exits are deterministic and not worth a re-run. The retry
    is skipped when the remaining cron wall-clock budget is too small to fit it,
    so a wedged provider cannot push the whole multi-topic run past its kill window.
    """
    result = _run_nullclaw_agent_once(prompt, timeout_secs, variant, all_items, numbered)
    if result.returncode != 124:
        return result

    retry_timeout = min(LLM_RETRY_TIMEOUT_SECS, timeout_secs)
    budget = _llm_retry_budget_secs()
    if budget is not None and budget < retry_timeout:
        log_trace(
            "llm_agent_retry_skipped_budget",
            variant=variant,
            budget_secs=round(budget, 1),
            retry_timeout=retry_timeout,
        )
        return result

    log_trace(
        "llm_agent_retry",
        variant=variant,
        attempt=2,
        first_timeout=timeout_secs,
        retry_timeout=retry_timeout,
    )
    return _run_nullclaw_agent_once(prompt, retry_timeout, variant, all_items, numbered)


def _run_nullclaw_agent_once(prompt: str, timeout_secs: int, variant: str, all_items: dict[str, list[dict]], numbered: dict[int, dict]):
    import subprocess

    argv = [os.path.expanduser("~/nullclaw/zig-out/bin/nullclaw"), "agent", "--isolated", "-m", prompt]
    env = os.environ.copy()
    env["NULLCLAW_AGENT_TIMING_TRACE"] = "1"
    started = time.monotonic()
    log_trace(
        "llm_agent_start",
        variant=variant,
        timeout_secs=timeout_secs,
        source_item_counts=_llm_source_item_counts(all_items),
        items_numbered=len(numbered),
        prompt_chars=len(prompt),
    )
    try:
        result = subprocess.run(
            argv,
            capture_output=True, text=True, timeout=timeout_secs, env=env,
        )
    except subprocess.TimeoutExpired as e:
        elapsed_ms = int((time.monotonic() - started) * 1000)
        log_trace(
            "llm_agent_timeout",
            variant=variant,
            timeout_secs=timeout_secs,
            elapsed_ms=elapsed_ms,
            source_item_counts=_llm_source_item_counts(all_items),
            items_numbered=len(numbered),
            prompt_chars=len(prompt),
            stdout_len=len(e.stdout or ""),
            stderr_len=len(e.stderr or ""),
            stdout_tail=_clip_subprocess_text(e.stdout, 4000),
            stderr_tail=_clip_subprocess_text(e.stderr, 4000),
        )
        return subprocess.CompletedProcess(
            argv,
            124,
            stdout=_clip_subprocess_text(e.stdout, 10000),
            stderr=_clip_subprocess_text(e.stderr, 10000),
        )
    elapsed_ms = int((time.monotonic() - started) * 1000)
    log_trace(
        "llm_agent_exit",
        variant=variant,
        elapsed_ms=elapsed_ms,
        returncode=result.returncode,
        stdout_len=len(result.stdout or ""),
        stderr_len=len(result.stderr or ""),
        stderr_tail=_clip_subprocess_text(result.stderr, 4000),
    )
    return result


def _fallback_section_lines(key: str, items: list[dict], limit: int, link_map: dict[str, str]) -> list[str]:
    if not items:
        return ["- 今日無相關新聞"]
    lines = []
    for idx, item in enumerate(items[:limit], start=1):
        title = item["title"]
        link = link_map.get(title, item.get("link", ""))
        if key == "ai" and _count_cjk(title) < 2:
            title = f"AI 新聞來源 {idx}（摘要翻譯暫時失敗）"
        title = _neutralize_markdown_specials(title)
        if link:
            lines.append(f"- {title} [🔗]({link})")
        else:
            lines.append(f"- {title}")
    return lines


# Per-run memo shared across all precheck calls so AI Level-3 re-subdivision
# (which re-covers the same items) does not re-decode/re-fetch. Keyed by link.
_PRECHECK_FETCH_CACHE: dict = {}

_MARKER_RE = re.compile(r"^\s*(?:-\s*)?#(\d+)\b(?!,)")


def _tier1_filter_items(items: list[dict]) -> list[dict]:
    """Deterministic, no-network pre-LLM filter: drop deny-listed SOURCES only.

    Promo/listicle judgement is left to the LLM prompt + Tier 2's title-only promo
    check; we do not gate titles here (and never match promo against body text —
    bare substrings like 限時 over-match real news such as 限時降息). Defensive."""
    if not PRECHECK_ENABLED or not items:
        return items
    try:
        deny = news_quality.active_config()["deny"]
        kept = [it for it in items if str(it.get("source_name") or "") not in deny]
        log_trace("quality_tier1", before=len(items), after=len(kept),
                  dropped=len(items) - len(kept))
        return kept
    except Exception as e:
        print(f"[WARN: tier1 filter failed: {e}]", file=sys.stderr)
        return items


def _precheck_apply(
    summary: str,
    numbered: dict[int, dict],
    section: str,
) -> tuple[str, dict[int, dict]]:
    """Tier 2: body-precheck the LLM-picked items BEFORE link attach (while #N
    identity still exists). For each selected #N:
      - drop       → remove the bullet line (deny source/domain, or promo/PR/listicle).
      - title_only → KEEP the LLM's bullet verbatim (paywalled but reputable; the
                     LLM's Chinese headline-summary stays, we just don't drop it).
                     Recorded in the returned paywall map so the render stage can
                     add a free-replacement bullet + a 付費牆 note.
      - keep       → leave the bullet as the LLM wrote it.

    Returns ``(summary, paywall)`` where ``paywall`` maps each surviving
    ``title_only`` marker id to ``{"decoded_url", "reason", "title",
    "source_name"}``. The map is empty when nothing is paywalled.

    The digest is Traditional-Chinese (a downstream language gate enforces it), so
    we never rewrite a bullet to the raw RSS headline — that would inject English/
    Japanese past the gate. Result collection is bounded by PRECHECK_TOTAL_DEADLINE;
    a worker already inside urlopen still runs to its socket timeout
    (PRECHECK_FETCH_TIMEOUT) since cancel_futures cannot interrupt it — keep that
    timeout small so the worst-case straggler is the real wall-clock bound.
    Defensive: on any error returns (summary, {}) unchanged."""
    if not PRECHECK_ENABLED or not summary.strip():
        return summary, {}
    try:
        import concurrent.futures

        selected = _extract_leading_marker_ids(summary, numbered)
        if not selected:
            return summary, {}

        def _check(num: int) -> tuple[int, dict]:
            item = numbered.get(num) or {}
            verdict = news_quality.precheck_action(
                {"title": item.get("title", ""), "link": item.get("link", ""),
                 "source_name": item.get("source_name", "")},
                decode_timeout=PRECHECK_DECODE_TIMEOUT,
                fetch_timeout=PRECHECK_FETCH_TIMEOUT,
                fetch_cache=_PRECHECK_FETCH_CACHE,
            )
            return num, verdict

        actions: dict[int, str] = {}
        verdicts: dict[int, dict] = {}
        pool = concurrent.futures.ThreadPoolExecutor(
            max_workers=min(PRECHECK_MAX_WORKERS, len(selected))
        )
        try:
            futs = {pool.submit(_check, n): n for n in selected}
            try:
                for fut in concurrent.futures.as_completed(futs, timeout=PRECHECK_TOTAL_DEADLINE):
                    num, verdict = fut.result()
                    actions[num] = verdict.get("action") or "keep"
                    verdicts[num] = verdict
            except concurrent.futures.TimeoutError:
                # Undecided items default to keep (fail-open on timeout).
                print(f"[WARN: tier2 precheck deadline hit: section={section}]", file=sys.stderr)
        finally:
            # Do not wait for stragglers — the deadline must bound wall-clock.
            pool.shutdown(wait=False, cancel_futures=True)

        n_drop = sum(1 for a in actions.values() if a == "drop")
        n_title = sum(1 for a in actions.values() if a == "title_only")
        log_trace("quality_tier2", section=section, checked=len(selected),
                  dropped=n_drop, paywalled_kept=n_title)

        # Build the paywall map for every surviving (non-dropped) title_only item.
        paywall: dict[int, dict] = {}
        for num, action in actions.items():
            if action != "title_only":
                continue
            item = numbered.get(num) or {}
            v = verdicts.get(num) or {}
            paywall[num] = {
                "decoded_url": v.get("decoded_url"),
                "reason": v.get("reason"),
                "title": item.get("title", ""),
                "source_name": item.get("source_name", ""),
            }

        if not n_drop:
            # title_only items keep the LLM's already-Chinese bullet verbatim — we
            # only DROP here (deny/promo). Rewriting to the raw RSS headline would
            # inject English/Japanese past the Chinese-only language gate.
            return summary, paywall

        out_lines = []
        for line in summary.splitlines():
            m = _MARKER_RE.match(line)
            if m and actions.get(int(m.group(1))) == "drop":
                continue
            out_lines.append(line)
        return "\n".join(out_lines), paywall
    except Exception as e:
        print(f"[WARN: tier2 precheck failed: section={section} {e}]", file=sys.stderr)
        return summary, {}


# A paywall pair renders as two adjacent lines: the free-replacement bullet on
# top, then a continuation line (this prefix, NOT a "- " bullet) carrying the
# original paywalled headline + a 付費牆 note. The prefix lets the trim/split
# helpers keep the pair atomic and keeps _news_bullet_lines from counting the
# continuation as a separate news bullet.
PAYWALL_CONT_PREFIX = "　↳ "
PAYWALL_NOTE = "⚠️ 付費牆（原文需訂閱）"


def _attach_numbered_links(
    summary: str,
    numbered: dict[int, dict],
    paywall: dict[int, dict] | None = None,
) -> tuple[str, int]:
    import re

    paywall = paywall or {}
    attached = {"count": 0}

    marker_line = re.compile(r"^\s*(?:-\s*)?#(\d+)\b(?!,)\s*")

    def _linked(body: str, link: str) -> str:
        if link:
            attached["count"] += 1
            return f"- {body} [🔗]({link})"
        return f"- {body}" if body else "-"

    def replace_line(line: str) -> str:
        match = marker_line.match(line)
        if not match:
            return line
        num = int(match.group(1))
        item = numbered.get(num)
        body = _neutralize_markdown_specials(line[match.end():].lstrip())
        link = item["link"] if item else ""

        pw = paywall.get(num)
        if pw:
            replacement = pw.get("replacement") or {}
            rep_title = _neutralize_markdown_specials(str(replacement.get("title_zh") or "").strip())
            rep_link = str(replacement.get("link") or "").strip()
            if rep_title and rep_link:
                # Free replacement on top, original paywalled headline continues below.
                # _linked(body, link) is evaluated exactly once per rendered line so
                # the attach counter counts each link at most once.
                orig_line = f"{PAYWALL_CONT_PREFIX}原文：{_linked(body, link)[2:]}  {PAYWALL_NOTE}"
                return f"{_linked(rep_title, rep_link)}\n{orig_line}"
            # No replacement found — single bullet + note (degraded form).
            return f"{_linked(body, link)}  {PAYWALL_NOTE}"

        return _linked(body, link)

    return "\n".join(replace_line(line) for line in summary.splitlines()), attached["count"]


def _translate_single_title(title: str, date_str: str) -> str | None:
    """Translate ONE headline to Traditional Chinese, reusing the section
    translator via a throwaway single-entry numbered dict. Returns the Chinese
    title (no marker/link) or None on any validation failure. Defensive: the
    caller treats None as 'no usable replacement'."""
    try:
        # A NON-empty placeholder link is required: _translate_selected_section
        # only reports success when _attach_numbered_links attaches >=1 link
        # (links_attached>0). We strip the placeholder back out below.
        placeholder = "https://paywall-rep.invalid/x"
        temp_num = {1: {"title": title, "link": placeholder, "source_name": ""}}
        lines = _translate_selected_section("paywall_rep", [1], temp_num, date_str)
        if not lines:
            return None
        # Strip the leading "- " and the placeholder link from the single bullet.
        body = _strip_links_keep_spacing(lines[0]).lstrip()
        if body.startswith("-"):
            body = body[1:].strip()
        return body or None
    except Exception:
        return None


def _same_registered_domain(host_a: str, host_b: str) -> bool:
    """True when two hosts share the same registrable domain (last two labels),
    so www.nytimes.com / cn.nytimes.com / nytimes.com all match. A deliberately
    simple heuristic — good enough to reject same-publisher 'free' candidates
    without pulling in a public-suffix list."""
    def reg(host: str) -> str:
        # Drop any :port and userinfo so nytimes.com:443 == nytimes.com.
        h = str(host or "").lower().strip().rsplit("@", 1)[-1].split(":", 1)[0].strip(".")
        parts = h.split(".")
        return ".".join(parts[-2:]) if len(parts) >= 2 else ".".join(parts)
    a, b = reg(host_a), reg(host_b)
    return bool(a) and a == b


def _resolve_paywall_replacements(paywall: dict[int, dict], date_str: str) -> None:
    """For each paywalled entry, try to find a FREE same-story article from a
    different host and translate its title. Mutates each entry in place, adding
    ``replacement={"title_zh", "link"}`` when one is found. Absent replacement =
    the render stage degrades to a single bullet + 付費牆 note.

    Bounded by PAYWALL_REPLACE_DEADLINE and PAYWALL_REPLACE_MAX. Wrapped so ANY
    exception leaves the entry without a replacement — never raises, because an
    escape would hit main()'s alert-and-re-raise and break the exit-0 contract."""
    if not paywall or not PAYWALL_REPLACE_ENABLED or not PRECHECK_ENABLED:
        return
    from urllib.parse import urlparse

    # Isolated cache: the main precheck cache (_PRECHECK_FETCH_CACHE) is keyed by
    # link only, but a candidate can share a link with a main-precheck item while
    # carrying different title/source metadata. A private cache avoids inheriting
    # a stale verdict, while still deduping repeated candidate links within this
    # resolver pass.
    rep_cache: dict = {}

    deadline = None
    try:
        deadline = time.monotonic() + PAYWALL_REPLACE_DEADLINE
    except Exception:
        deadline = None

    processed = 0
    for num, entry in paywall.items():
        if processed >= PAYWALL_REPLACE_MAX:
            break
        if deadline is not None and time.monotonic() >= deadline:
            log_trace("paywall_replace_deadline", resolved=processed)
            break
        processed += 1
        try:
            orig_title = str(entry.get("title") or "")
            if not orig_title.strip():
                continue
            orig_host = ""
            decoded = entry.get("decoded_url")
            if decoded:
                orig_host = urlparse(str(decoded)).netloc.lower()

            query = _title_without_source(orig_title).strip()
            if not query:
                continue
            if deadline is not None and time.monotonic() >= deadline:
                break

            def _fetch_timeout() -> float | None:
                if deadline is None:
                    return 15.0
                if time.monotonic() >= deadline:
                    return None
                return min(15.0, max(0.5, deadline - time.monotonic()))

            candidates: list[dict] = []
            if "google" in PAYWALL_REPLACE_SOURCES:
                timeout = _fetch_timeout()
                if timeout is not None:
                    candidates.extend(
                        fetch_feed(_topic_feed_url(query), max_items=8, timeout=timeout)
                    )
            if "bing" in PAYWALL_REPLACE_SOURCES:
                timeout = _fetch_timeout()
                if timeout is not None:
                    for item in fetch_feed(
                        _bing_news_feed_url(query), max_items=8, timeout=timeout
                    ):
                        candidates.append(_normalize_replacement_candidate(item))
            candidates = dedup(candidates)
            orig_words = _topic_words(orig_title)

            for cand in candidates:
                # The deadline is checked before EACH network/LLM step (not just
                # between entries) so one slow entry cannot blow the wall-clock.
                if deadline is not None and time.monotonic() >= deadline:
                    log_trace("paywall_replace_deadline", resolved=processed)
                    break
                cand_title = str(cand.get("title") or "")
                cand_link = str(cand.get("link") or "")
                if not cand_title or not cand_link:
                    continue
                # Same-story check: meaningful token overlap with the original.
                overlap = orig_words & _topic_words(cand_title)
                if len(overlap) < 2:
                    continue
                precheck_item = {
                    "title": cand_title,
                    "link": cand_link,
                    "source_name": cand.get("source_name", ""),
                }
                if cand.get("decoded_url"):
                    precheck_item["decoded_url"] = cand["decoded_url"]
                verdict = news_quality.precheck_action(
                    precheck_item,
                    decode_timeout=PRECHECK_DECODE_TIMEOUT,
                    fetch_timeout=PRECHECK_FETCH_TIMEOUT,
                    fetch_cache=rep_cache,
                )
                if verdict.get("action") in ("title_only", "drop"):
                    continue  # candidate is itself paywalled or junk
                cand_decoded = verdict.get("decoded_url")
                cand_host = urlparse(str(cand_decoded)).netloc.lower() if cand_decoded else ""
                # Require a resolved, DIFFERENT publisher. An unresolved host
                # (empty) cannot be confirmed distinct from the paywalled source,
                # so it is skipped rather than risk a same-publisher "free" link.
                if not cand_host:
                    continue
                if orig_host and _same_registered_domain(cand_host, orig_host):
                    continue  # same publisher — not a free alternative
                if deadline is not None and time.monotonic() >= deadline:
                    log_trace("paywall_replace_deadline", resolved=processed)
                    break
                title_zh = _translate_single_title(cand_title, date_str)
                if not title_zh:
                    continue
                entry["replacement"] = {
                    "title_zh": title_zh,
                    "link": verdict.get("decoded_url") or cand_link,
                }
                log_trace("paywall_replacement_found", marker=num,
                          source=cand.get("source_name", ""))
                break
        except Exception as e:
            print(f"[WARN: paywall replacement lookup failed: #{num} {e}]", file=sys.stderr)
            continue


def _strip_links_keep_spacing(value: str) -> str:
    import re

    value = re.sub(r"\s*\[🔗\]\([^)]+\)\s*", " ", value)
    value = re.sub(r"^-\s*", "- ", value, flags=re.MULTILINE)
    return "\n".join(line.rstrip() for line in value.splitlines())


def _trim_links_to_limit(text: str, limit: int = 4000) -> str:
    if len(text) <= limit:
        return text

    lines = text.splitlines()
    for idx in range(len(lines) - 1, -1, -1):
        if "[🔗](" not in lines[idx]:
            continue
        lines[idx] = _strip_links_keep_spacing(lines[idx])
        candidate = "\n".join(lines)
        if len(candidate) <= limit:
            return candidate
    return _trim_lines_to_limit(_strip_links_keep_spacing(text), limit)


def _markdown_visible_text(text: str) -> str:
    import re

    return re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", text)


def _trim_lines_to_limit(text: str, limit: int = 4000) -> str:
    if len(text) <= limit:
        return text

    lines = text.splitlines()
    for idx in range(len(lines) - 1, -1, -1):
        if not lines[idx].lstrip().startswith("-"):
            continue
        # Drop the bullet AND any paywall continuation line that follows it, so a
        # paywall pair is never left as an orphaned 原文 note with no headline.
        end = idx + 1
        while end < len(lines) and lines[end].startswith(PAYWALL_CONT_PREFIX):
            end += 1
        del lines[idx:end]
        candidate = "\n".join(lines)
        if len(candidate) <= limit:
            return candidate

    if limit <= 20:
        return text[:limit]
    return text[: limit - 20].rstrip() + "\n…（已截短）"


def _translate_selected_section(
    key: str,
    selected_ids: list[int],
    numbered: dict[int, dict],
    date_str: str,
    paywall: dict[int, dict] | None = None,
) -> list[str] | None:
    if not selected_ids:
        return None

    selected_numbered = {idx: numbered[idx] for idx in selected_ids if idx in numbered}
    raw = "\n".join(f"#{idx} {item['title']}" for idx, item in selected_numbered.items())
    prompt = (
        f"你是新聞標題翻譯編輯。以下是今天({date_str})已選出的新聞標題，每則有編號 #N。\n\n"
        f"{raw}\n\n"
        f"{TRANSLATION_RULES_STRICT}\n"
        f"輸出格式必須只有 dash bullets：\n"
        f"- #N 繁體中文標題\n"
        f"- #N ...\n\n"
        f"每行必須以繁體中文新聞句子開始；不要先輸出英文原標題，也不要用「英文（中文）」格式。\n"
        f"不要輸出開場白、區塊標題、解釋或英文原標題。"
    )
    result = _run_nullclaw_agent(
        prompt,
        LLM_TRANSLATION_TIMEOUT_SECS,
        f"default_{key}_translate",
        {key: [numbered[idx] for idx in selected_ids if idx in numbered]},
        selected_numbered,
    )
    summary = result.stdout.strip()
    marked_bullets, total_bullets = _marker_validation_stats(summary, selected_numbered)
    chinese_bullets, language_total = _language_validation_stats(summary)
    if (
        total_bullets > 0 and
        marked_bullets == total_bullets and
        _language_validation_passed(summary)
    ):
        with_links, links_attached = _attach_numbered_links(summary, selected_numbered, paywall)
        if links_attached > 0:
            return with_links.splitlines()

    _log_llm_validation_failed(
        f"default_{key}_translate",
        result,
        summary,
        marked_bullets,
        total_bullets,
        selected_numbered,
        "translation_retry_validation",
        {
            "chinese_bullets": chinese_bullets,
            "language_total": language_total,
        },
    )
    return None


def _trim_digest_links(text: str) -> str:
    # Telegram's documented message limit is based on text after entity
    # parsing, not raw Markdown URL bytes. Keep source links when the visible
    # digest is short enough; delivery splits raw Markdown into safe chunks.
    if len(_markdown_visible_text(text)) <= 4000:
        return text
    lines = text.split("\n")
    in_ai = False
    trimmed = []
    for line in lines:
        if "AI 人工智慧" in line:
            in_ai = True
        elif line.startswith("**"):
            in_ai = False
        if not in_ai and "[🔗](" in line:
            line = _strip_links_keep_spacing(line)
        trimmed.append(line)
    result = "\n".join(trimmed)
    if len(result) <= 4000:
        return result
    return _trim_links_to_limit(text)


def _split_message_preserving_lines(body: str, limit: int = TELEGRAM_RAW_CHUNK_LIMIT) -> list[str]:
    if len(body) <= limit:
        return [body]

    chunks: list[str] = []
    current: list[str] = []
    current_len = 0

    def flush_current() -> None:
        nonlocal current, current_len
        if current:
            chunks.append("".join(current).rstrip())
            current = []
            current_len = 0

    raw_lines = body.splitlines(keepends=True)
    for idx, line in enumerate(raw_lines):
        this_is_cont = line.lstrip("\n").startswith(PAYWALL_CONT_PREFIX)
        # A paywall continuation line must never start a new chunk without its
        # parent bullet: only flush BEFORE the parent, never between the pair.
        next_is_cont = (
            idx + 1 < len(raw_lines)
            and raw_lines[idx + 1].lstrip("\n").startswith(PAYWALL_CONT_PREFIX)
        )
        if len(line) > limit:
            # A single physical line longer than the limit must be hard-split. If
            # it is a paywall PARENT (its next line is a continuation), keep the
            # continuation glued to the final piece so it is never orphaned; the
            # combined tail may exceed `limit` but a truncated pair is worse.
            if this_is_cont and current:
                # Over-limit continuation: keep it attached to the current chunk
                # (which holds its parent) rather than starting a fresh chunk.
                current.append(line)
                current_len += len(line)
                continue
            flush_current()
            pieces = [line[s:s + limit].rstrip() for s in range(0, len(line), limit)]
            if next_is_cont and pieces:
                # Resume accumulation on the last piece so the continuation joins it.
                for p in pieces[:-1]:
                    chunks.append(p)
                current = [pieces[-1] + "\n"]
                current_len = len(current[0])
                continue
            chunks.extend(pieces)
            continue
        if current and current_len + len(line) > limit and not this_is_cont:
            flush_current()
        current.append(line)
        current_len += len(line)
        # If the next line is a continuation, keep accumulating so the pair
        # stays together even if that pushes slightly toward the limit; the
        # per-line >limit guard above still bounds any single physical line.
        if next_is_cont:
            continue

    flush_current()
    return [chunk for chunk in chunks if chunk]


def _markdown_chunk_is_safe(chunk: str) -> tuple[bool, str]:
    """Heuristic safety check for Telegram legacy Markdown chunks."""
    probe_chars: list[str] = []
    i = 0
    while i < len(chunk):
        ch = chunk[i]
        if ch == "[":
            close_label = chunk.find("]", i + 1)
            if close_label == -1:
                return False, "unclosed link bracket"
            if close_label + 1 < len(chunk) and chunk[close_label + 1] == "(":
                close_url = chunk.find(")", close_label + 2)
                if close_url == -1:
                    return False, "unclosed link url"
                i = close_url + 1
                continue
        probe_chars.append(ch)
        i += 1

    if chunk.endswith("\\") and not chunk.endswith("\\\\"):
        return False, "trailing backslash"

    probe = "".join(probe_chars)
    for marker, name in (("*", "asterisk"), ("_", "underscore"), ("`", "backtick")):
        count = 0
        escaped = False
        for ch in probe:
            if escaped:
                escaped = False
                continue
            if ch == "\\":
                escaped = True
                continue
            if ch == marker:
                count += 1
        if count % 2 != 0:
            return False, f"unmatched {name}"
    return True, ""


def _deliver_news_chunks(
    chat_id: str,
    chunks: list[str],
    account: str,
    *,
    parse_mode: str | None,
) -> None:
    if len(chunks) == 1:
        deliver_or_fail(chat_id, chunks[0], account=account, parse_mode=parse_mode)
        return

    for idx, chunk in enumerate(chunks, start=1):
        deliver_or_fail(
            chat_id,
            f"({idx}/{len(chunks)})\n{chunk}",
            account=account,
            parse_mode=parse_mode,
        )


def _deliver_news_or_fail(chat_id: str | None, body: str, account: str) -> None:
    if not chat_id:
        deliver_or_fail(chat_id, body, account=account)
        return

    chunks = _split_message_preserving_lines(body)
    unsafe_chunks: list[tuple[int, str]] = []
    for idx, chunk in enumerate(chunks, start=1):
        ok, reason = _markdown_chunk_is_safe(chunk)
        if not ok:
            unsafe_chunks.append((idx, reason))

    if unsafe_chunks:
        log_trace(
            "digest_markdown_unsafe_fallback",
            total_chunks=len(chunks),
            unsafe_chunks=[idx for idx, _ in unsafe_chunks],
            reasons=[reason for _, reason in unsafe_chunks[:3]],
        )
        if len(chunks) > 1:
            log_trace(
                "digest_delivery_split",
                chunks=len(chunks),
                raw_chars=len(body),
                visible_chars=len(_markdown_visible_text(body)),
            )
        _deliver_news_chunks(chat_id, chunks, account, parse_mode=None)
        return

    if len(chunks) > 1:
        log_trace(
            "digest_delivery_split",
            chunks=len(chunks),
            raw_chars=len(body),
            visible_chars=len(_markdown_visible_text(body)),
        )
    _deliver_news_chunks(chat_id, chunks, account, parse_mode="Markdown")


def _summarize_default_section(key: str, items: list[dict], date_str: str, link_map: dict[str, str]) -> tuple[list[str], bool]:
    """Return (lines, used_fallback). used_fallback=True iff the LLM path
    failed and we returned _fallback_section_lines. Empty-input case returns
    a placeholder with used_fallback=False (no LLM was attempted).
    """
    spec = DEFAULT_SECTION_SPECS[key]
    if not items:
        return ["- 今日無相關新聞"], False

    section_items = {key: items}
    numbered, raw = _number_items_for_prompt(
        section_items,
        labels=[key],
        limits={key: spec["limit"]},
    )
    prompt = (
        f"你是新聞編輯。以下是今天({date_str})的「{spec['header']}」候選新聞標題（每則有編號 #N）。\n\n"
        f"{raw}\n\n"
        f"請挑出 {spec['pick']} 則{spec['focus']}。\n"
        f"用繁體中文輸出，格式嚴格如下（不要輸出標題、開場白或結語）：\n"
        f"- #N 新聞標題\n"
        f"- #N ...\n\n"
        f"規則：\n"
        f"- 每則新聞前面必須保留原始編號 #N\n"
        f"- {TRANSLATION_RULES_STRICT}\n"
        f"- 每行必須以繁體中文新聞句子開始，不要輸出英文原標題或「英文（中文）」格式\n"
        f"- 排除瑣碎的、純行銷推廣的、政治宣傳性質的、投資建議類新聞\n"
        f"- 同一則新聞如果有多個來源（標題講同一件事，例如「百度Q1財報」三個版本），只挑一則：\n"
        f"  優先選免費來源（cnyes、TechNews、Yahoo新聞、MoneyDJ、工商時報、Reuters、AP、ScienceDaily、TechCrunch 等）\n"
        f"  避開付費牆來源（WSJ、Bloomberg、FT、Nikkei、Barron's 等）\n"
        f"  只有付費來源報導時才保留付費來源\n"
        f"- 重複判斷以「事件本身」為準：同公司同季財報、同一政策公告、同一產品發布、同一研究突破都算重複"
    )
    try:
        result = _run_nullclaw_agent(
            prompt,
            LLM_SECTION_TIMEOUT_SECS,
            f"default_{key}",
            section_items,
            numbered,
        )
        summary = result.stdout.strip()
        if summary:
            marked_bullets, total_bullets = _marker_validation_stats(summary, numbered)
            if total_bullets > 0 and marked_bullets == total_bullets:
                chinese_bullets, language_total = _language_validation_stats(summary)
                if not _language_validation_passed(summary):
                    _log_llm_validation_failed(
                        f"default_{key}",
                        result,
                        summary,
                        marked_bullets,
                        total_bullets,
                        numbered,
                        "language_validation",
                        {
                            "chinese_bullets": chinese_bullets,
                            "language_total": language_total,
                        },
                    )
                    summary, paywall = _precheck_apply(summary, numbered, key)
                    if not _news_bullet_lines(summary):
                        # Every pick was deny/promo — a filter success, not an LLM
                        # failure: used_fallback=False so no operator alert fires.
                        log_trace("quality_all_dropped", section=key)
                        return ["- 今日無相關新聞"], False
                    _resolve_paywall_replacements(paywall, date_str)
                    translated = _translate_selected_section(
                        key,
                        _extract_leading_marker_ids(summary, numbered),
                        numbered,
                        date_str,
                        paywall,
                    )
                    if translated is not None:
                        return translated, False
                else:
                    summary, paywall = _precheck_apply(summary, numbered, key)
                    if not _news_bullet_lines(summary):
                        log_trace("quality_all_dropped", section=key)
                        return ["- 今日無相關新聞"], False
                    _resolve_paywall_replacements(paywall, date_str)
                    with_links, links_attached = _attach_numbered_links(summary, numbered, paywall)
                    if links_attached > 0:
                        return with_links.splitlines(), False
                    log_trace(
                        "llm_link_validation_failed",
                        variant=f"default_{key}",
                        section=key,
                        returncode=result.returncode,
                        stdout_len=len(result.stdout or ""),
                        items_numbered=len(numbered),
                        marked_bullets=marked_bullets,
                        total_bullets=total_bullets,
                        stdout_sample=_clip_subprocess_text(summary, 1200),
                        line_sample=_sample_nonempty_lines(summary, 8),
                    )
            else:
                _log_llm_validation_failed(
                    f"default_{key}",
                    result,
                    summary,
                    marked_bullets,
                    total_bullets,
                    numbered,
                    "marker_validation",
                )
            print(
                "[WARN] LLM section validation failed: "
                f"section={key} marked={marked_bullets}/{total_bullets}",
                file=sys.stderr,
            )
        else:
            _log_llm_validation_failed(
                f"default_{key}",
                result,
                summary,
                0,
                0,
                numbered,
                "empty_stdout",
            )
            print(
                f"[WARN] LLM section validation failed: section={key} empty stdout",
                file=sys.stderr,
            )
    except Exception as e:
        print(f"[WARN] LLM section summary failed: section={key} {e}", file=sys.stderr)

    return _fallback_section_lines(key, items, spec["fallback_limit"], link_map), True


def _run_ai_substage(
    items: list[dict],
    start: int,
    end: int,
    date_str: str,
) -> tuple[bool, list[str], str]:
    """Run one LLM call covering items[start:end] and return (ok, lines, error).

    ok=True   → lines is the validated bullet list for this range; error="".
    ok=False  → lines=[]; error is a short string describing the hard failure
                (timeout / non-zero exit / empty stdout / marker validation).

    A successful run is cached and returned from cache on the next attempt
    of the same clustered range on the same date.
    """
    variant = AI_SUBSTAGE_CACHE_VARIANT
    cached = _news_cache_get(date_str, variant, start, end)
    if cached is not None:
        return True, cached.splitlines(), ""

    sub_items = items[start:end]
    if not sub_items:
        return True, [], ""

    section_items = {"ai": sub_items}
    numbered, raw = _number_items_for_prompt(
        section_items,
        labels=["ai"],
        limits={"ai": len(sub_items)},
    )
    spec = DEFAULT_SECTION_SPECS["ai"]
    # Smaller pick count proportional to batch size; the two halves are
    # concatenated, so each half should not over-select.
    pick_count = max(2, len(sub_items) // 3)
    prompt = (
        f"你是新聞編輯。以下是今天({date_str})的「{spec['header']}」候選新聞標題（每則有編號 #N），這是分批處理的批次。\n\n"
        f"{raw}\n\n"
        f"請從這個批次挑出 {pick_count} 則{spec['focus']}。\n"
        f"用繁體中文輸出，格式嚴格如下（不要輸出標題、開場白或結語）：\n"
        f"- #N 新聞標題\n"
        f"- #N ...\n\n"
        f"規則：\n"
        f"- 每則新聞前面必須保留原始編號 #N\n"
        f"- {TRANSLATION_RULES_STRICT}\n"
        f"- 排除瑣碎的、純行銷推廣的、政治宣傳性質的、投資建議類新聞\n"
        f"- 同一則新聞如果有多個來源（標題講同一件事），只挑一則：\n"
        f"  優先選免費來源（cnyes、TechNews、Yahoo新聞、MoneyDJ、工商時報、Reuters、AP、ScienceDaily、TechCrunch 等）\n"
        f"  避開付費牆來源（WSJ、Bloomberg、FT、Nikkei、Barron's 等）\n"
        f"  只有付費來源報導時才保留付費來源"
    )

    result = _run_nullclaw_agent(
        prompt,
        AI_SUBSTAGE_TIMEOUT_SECS,
        f"{variant}_{start}_{end}",
        section_items,
        numbered,
    )
    summary = (result.stdout or "").strip()

    if result.returncode == 124:
        return False, [], f"timeout after {AI_SUBSTAGE_TIMEOUT_SECS}s"
    if result.returncode != 0:
        return False, [], f"exit_code={result.returncode}"
    if not summary:
        return False, [], "empty_stdout"

    marked_bullets, total_bullets = _marker_validation_stats(summary, numbered)
    if total_bullets == 0 or marked_bullets != total_bullets:
        return False, [], f"marker_validation marked={marked_bullets}/{total_bullets}"

    # Tier 2 body precheck, before any cache write or link attach (while #N identity exists).
    summary, paywall = _precheck_apply(summary, numbered, "ai")
    if not _news_bullet_lines(summary):
        # Every picked item was low-quality and dropped. This is a SUCCESS of the
        # filter, not an LLM failure — return ok=True with empty lines so the
        # Level-2/3 driver does NOT escalate to _AiSubstageExhausted. Do NOT cache
        # the empty result: a transient mis-drop must not stick for the whole day —
        # a later same-day run should re-evaluate from a fresh LLM call.
        log_trace("ai_substage_all_dropped", range=[start, end])
        return True, [], ""

    _resolve_paywall_replacements(paywall, date_str)

    if not _language_validation_passed(summary):
        translated = _translate_selected_section(
            "ai",
            _extract_leading_marker_ids(summary, numbered),
            numbered,
            date_str,
            paywall,
        )
        if translated is None:
            return False, [], "language_validation"
        _news_cache_put(date_str, variant, start, end, "\n".join(translated))
        return True, translated, ""

    with_links, links_attached = _attach_numbered_links(summary, numbered, paywall)
    if links_attached == 0:
        return False, [], "no_links_attached"

    body = with_links
    _news_cache_put(date_str, variant, start, end, body)
    return True, body.splitlines(), ""


class _AiSubstageExhausted(Exception):
    """Raised when Level 3 substaging fails for any required quarter.

    Caller (summarize_llm) re-raises so main() can record exit-1 without
    delivering a partial digest. The alert is sent from inside the substage
    path before this exception is raised.
    """
    pass


def _summarize_default_ai_substaged(
    items: list[dict],
    date_str: str,
    ctx: "AlertContext",
) -> list[str]:
    """Run default_ai as two Level-2 halves; on per-half failure escalate to
    Level 3 (one more half-cut on that half only). If any Level-3 quarter
    still fails, alert and raise _AiSubstageExhausted.
    """
    if not items:
        return ["- 今日無相關新聞"]

    before_cluster = len(items)
    clusters = cluster(items)
    items = pick_representatives(clusters, per_cluster=1)
    log_trace(
        "cluster_dedup",
        before=before_cluster,
        after=len(items),
        clusters_total=len(clusters),
        clusters_kept=len(items),
    )
    if not items:
        return ["- 今日無相關新聞"]

    n = len(items)
    mid = n // 2

    log_trace("ai_substage_start", total_items=n, level2_a=[0, mid], level2_b=[mid, n])

    halves = [(0, mid), (mid, n)]
    half_results: list[list[str] | None] = [None, None]
    half_errors: list[str] = ["", ""]

    # Level 2: two halves, sequential.
    for i, (s, e) in enumerate(halves):
        ok, lines, err = _run_ai_substage(items, s, e, date_str)
        if ok:
            half_results[i] = lines
        else:
            half_errors[i] = err
            log_trace("ai_substage_level2_failed", half=i, range=[s, e], error=err)

    # Level 3: only on halves that failed in Level 2.
    for i, (s, e) in enumerate(halves):
        if half_results[i] is not None:
            continue
        sub_n = e - s
        if sub_n <= 1:
            # Cannot halve further; treat the failed half as exhausted.
            detail = (
                f"default_ai Level 2 half items[{s}..{e}] failed with size {sub_n}, "
                f"cannot subdivide further. Level 2 error: {half_errors[i]}"
            )
            _alert_failure(ctx, "ai_substage_level3_failed", detail)
            raise _AiSubstageExhausted(detail)

        sub_mid = s + sub_n // 2
        quarters = [(s, sub_mid), (sub_mid, e)]
        log_trace("ai_substage_level3_start", failed_half=i, quarters=quarters)

        merged: list[str] = []
        for qs, qe in quarters:
            ok, lines, err = _run_ai_substage(items, qs, qe, date_str)
            if not ok:
                cached_ok = [
                    [qqs, qqe]
                    for qqs, qqe in quarters
                    if _news_cache_get(date_str, AI_SUBSTAGE_CACHE_VARIANT, qqs, qqe) is not None
                ]
                detail = (
                    f"default_ai Level 3 quarter items[{qs}..{qe}] failed: {err}; "
                    f"Level 2 half [{s}..{e}] error: {half_errors[i]}; "
                    f"quarters cached so far: {cached_ok}"
                )
                _alert_failure(ctx, "ai_substage_level3_failed", detail)
                raise _AiSubstageExhausted(detail)
            merged.extend(lines)

        half_results[i] = merged

    final: list[str] = []
    for lines in half_results:
        final.extend(lines or [])

    if not final:
        log_trace("ai_substage_empty_after_merge", total_items=n)
        return ["- 今日無相關新聞"]

    log_trace("ai_substage_complete", total_items=n, total_bullets=len(final))
    return final


def summarize_llm(all_items: dict[str, list[dict]], ctx: "AlertContext") -> str:
    """Ask the nullclaw agent to curate and summarize news for significance."""
    tw_now = datetime.now(timezone(timedelta(hours=8)))
    date_str = tw_now.strftime("%Y/%m/%d (%a)")

    link_map = _build_link_map(all_items)
    lines = [f"\U0001f4f0 早安新聞摘要 — {date_str}\n"]
    section_keys = ("ai", "tech", "general")
    section_results: dict[str, list[str]] = {}
    degraded_sections: list[str] = []  # sections that fell back to non-LLM

    for key in section_keys:
        try:
            if key == "ai":
                # Substaged path: Level 2 (always) → Level 3 (per failed half)
                # → escalate via _AiSubstageExhausted on terminal failure.
                section_results[key] = _summarize_default_ai_substaged(
                    all_items.get(key, []), date_str, ctx,
                )
            else:
                # Existing single-call path; tuple return tells us authoritatively
                # whether the LLM succeeded or we used the non-LLM fallback.
                lines_out, used_fallback = _summarize_default_section(
                    key, all_items.get(key, []), date_str, link_map,
                )
                section_results[key] = lines_out
                if used_fallback and all_items.get(key):
                    degraded_sections.append(key)
        except _AiSubstageExhausted:
            # Alert was already sent from inside _summarize_default_ai_substaged.
            # Re-raise so main() exits 1 and does not deliver a partial digest.
            raise
        except Exception as e:
            print(f"[WARN] LLM section worker failed: section={key} {e}", file=sys.stderr)
            spec = DEFAULT_SECTION_SPECS[key]
            section_results[key] = _fallback_section_lines(
                key, all_items.get(key, []), spec["fallback_limit"], link_map,
            )
            degraded_sections.append(key)
            _alert_failure(
                ctx,
                f"section_{key}_exception",
                f"section {key} raised {type(e).__name__}: {e}",
            )

    if degraded_sections:
        # News still goes out, but quality is degraded. Per the hard rule
        # ("whenever the skill cannot send out the news"), this counts:
        # the operator wanted LLM-curated news, not a raw bullet dump.
        _alert_failure(
            ctx,
            "section_fallback_used",
            f"sections delivered using non-LLM fallback: {degraded_sections}",
        )

    for key in section_keys:
        spec = DEFAULT_SECTION_SPECS[key]
        lines.append(spec["header"])
        lines.extend(section_results[key])
        lines.append("")

    digest = "\n".join(lines)
    # Footer: the PAYWALL_NOTE marker appears exactly once per paywalled STORY
    # (both the replacement-pair and the degraded single-bullet forms carry one),
    # so counting it counts stories, not rendered bullets. Not a failure — never
    # routes through _alert_failure.
    paywall_count = digest.count(PAYWALL_NOTE)
    if paywall_count:
        digest += f"\nℹ️ 本次含 {paywall_count} 則付費牆新聞（原文需訂閱）"
        log_trace("paywall_notice", count=paywall_count)
    return _trim_digest_links(digest)


def fallback_summary(all_items: dict[str, list[dict]], date_str: str, link_map: dict[str, str] | None = None) -> str:
    """Simple fallback when LLM is unavailable."""
    lines = [f"\U0001f4f0 早安新聞摘要 — {date_str}\n"]
    section_map = {
        "ai": ("**\U0001f916 AI 人工智慧**", 10),
        "tech": ("**\U0001f4bb 科技 & 半導體**", 8),
        "general": ("**\U0001f30f 重大新聞**", 3),
    }
    for key, (header, limit) in section_map.items():
        items = all_items.get(key, [])
        lines.append(header)
        if items:
            for item in items[:limit]:
                raw_title = item["title"]
                title = _neutralize_markdown_specials(raw_title)
                link = (link_map or {}).get(raw_title, item.get("link", ""))
                if link:
                    lines.append(f"- {title} [🔗]({link})")
                else:
                    lines.append(f"- {title}")
        else:
            lines.append("- 今日無相關新聞")
        lines.append("")
    return "\n".join(lines)


def _topic_feed_url(topic: str) -> str:
    """Build a Google News RSS search URL for a Chinese topic keyword."""
    encoded = urllib.parse.quote(topic)
    return (
        f"https://news.google.com/rss/search?q={encoded}+when:1d"
        f"&hl=zh-TW&gl=TW&ceid=TW:zh-Hant"
    )


def _bing_news_feed_url(query: str) -> str:
    encoded = urllib.parse.quote(query)
    return (
        f"https://www.bing.com/news/search?q={encoded}"
        f"&mkt={PAYWALL_REPLACE_BING_MKT}&format=rss"
    )


def _normalize_replacement_candidate(item: dict) -> dict:
    link = str(item.get("link") or "")
    parsed = urllib.parse.urlparse(link)
    if parsed.netloc.lower().endswith("bing.com") and parsed.path.endswith("/news/apiclick.aspx"):
        qs = urllib.parse.parse_qs(parsed.query)
        direct = (qs.get("url") or [""])[0]
        if direct:
            item = dict(item)
            item["link"] = direct
            item["decoded_url"] = direct
    return item


def _fetch_custom_topics(topics: list[str]) -> dict[str, list[dict]]:
    """Fetch feeds for custom topic list, keyed by topic name. Returns RAW deduped
    items — Tier-1 filtering is applied by the caller AFTER the feed-emptiness
    check, so a deny-list emptying every topic is not misread as a feed outage."""
    all_items: dict[str, list[dict]] = {}
    for topic in topics:
        items = fetch_feed(_topic_feed_url(topic), max_items=10)
        all_items[topic] = dedup(items)
    return all_items


def _run_custom_topic(
    topic: str,
    items: list[dict],
    date_str: str,
) -> tuple[bool, list[str], str]:
    """Run one LLM call covering exactly one custom topic; return (ok, lines, error).

    ok=True   → lines is the validated bullet list for this topic; error="".
    ok=False  → lines=[]; error is a short string describing the hard failure.

    Per-topic granularity is the resumability unit: each call covers up to
    LLM_CUSTOM_TOPIC_LIMIT items (~8) and finishes in ~10s on a healthy host,
    well within typical kill windows. A successful call is cached at
    ~/.nullclaw/.news-cache/<date>/custom-<variant>-<safe_topic>.txt (the variant
    is embedded so a precheck-logic bump invalidates same-day caches) and reused
    on the next attempt of the same (date, topic).
    """
    variant = "custom_topic_v2_precheck"
    safe_date = date_str.split()[0].replace("/", "-")
    safe_topic = "".join(ch if ch.isalnum() else "_" for ch in topic)[:40]
    # Embed the variant in the filename so a bump actually invalidates same-day
    # caches (otherwise the rename is a no-op and stale pre-precheck bodies serve).
    cache_path = os.path.join(NEWS_CACHE_DIR, safe_date, f"custom-{variant}-{safe_topic}.txt")
    try:
        with open(cache_path, encoding="utf-8") as f:
            cached = f.read()
        log_trace("news_cache_hit", variant=variant, topic=topic)
        return True, cached.splitlines(), ""
    except FileNotFoundError:
        pass
    except OSError as e:
        log_trace("news_cache_read_error", variant=variant, topic=topic, error=str(e))

    if not items:
        return True, ["- 今日無相關新聞"], ""

    section_items = {topic: items[:LLM_CUSTOM_TOPIC_LIMIT]}
    numbered, raw = _number_items_for_prompt(
        section_items,
        labels=[topic],
        limits={topic: LLM_CUSTOM_TOPIC_LIMIT},
    )
    prompt = (
        f"你是新聞編輯。以下是今天({date_str})關於「{topic}」的候選新聞標題（每則有編號 #N）。\n\n"
        f"{raw}\n\n"
        f"請從中挑出 2-4 則真正有影響力、有意義的新聞，排除瑣碎、純行銷推廣、政治宣傳性質的新聞。\n"
        f"用繁體中文輸出，格式嚴格如下（不要輸出標題、開場白或結語）：\n"
        f"- #N 新聞標題\n"
        f"- #N ...\n\n"
        f"規則：\n"
        f"- 每則新聞前面必須保留原始編號 #N\n"
        f"- {TRANSLATION_RULES_STRICT}\n"
        f"- 如果今日無相關新聞，輸出「- 今日無相關新聞」"
    )

    result = _run_nullclaw_agent(
        prompt,
        AI_SUBSTAGE_TIMEOUT_SECS,
        f"{variant}_{safe_topic}",
        section_items,
        numbered,
    )
    summary = (result.stdout or "").strip()

    if result.returncode == 124:
        return False, [], f"timeout after {AI_SUBSTAGE_TIMEOUT_SECS}s"
    if result.returncode != 0:
        return False, [], f"exit_code={result.returncode}"
    if not summary:
        return False, [], "empty_stdout"

    marked_bullets, total_bullets = _marker_validation_stats(summary, numbered)
    if total_bullets == 0 or marked_bullets != total_bullets:
        return False, [], f"marker_validation marked={marked_bullets}/{total_bullets}"

    # Tier 2 body precheck, before cache write / link attach (while #N identity exists).
    summary, paywall = _precheck_apply(summary, numbered, f"custom:{topic}")
    if not _news_bullet_lines(summary):
        # All picks were low-quality — a filter SUCCESS, not a failure. Return
        # ok=True with the placeholder so the caller does NOT re-dump raw RSS items
        # via _custom_topic_raw_listing. Returns BEFORE the cache write below, so
        # the empty result is not persisted (no sticky same-day suppression).
        log_trace("quality_all_dropped", section=f"custom:{topic}")
        return True, ["- 今日無相關新聞"], ""

    _resolve_paywall_replacements(paywall, date_str)
    with_links, links_attached = _attach_numbered_links(summary, numbered, paywall)
    if links_attached == 0:
        # Bullets present but no link attached. Treat as success (still readable
        # for the user) but skip caching so a re-run can try to do better.
        return True, summary.splitlines(), ""

    body = with_links
    try:
        os.makedirs(os.path.dirname(cache_path), exist_ok=True)
        with open(cache_path, "w", encoding="utf-8") as f:
            f.write(body)
        log_trace("news_cache_write", variant=variant, topic=topic, bytes=len(body))
    except OSError as e:
        log_trace("news_cache_write_error", variant=variant, topic=topic, error=str(e))
    return True, body.splitlines(), ""


def _custom_topic_raw_listing(topic: str, items: list[dict], link_map: dict[str, str]) -> list[str]:
    """Return the non-LLM raw bullet list for one topic (per-topic fallback)."""
    if not items:
        return ["- 今日無相關新聞"]
    lines: list[str] = []
    for item in items[:5]:
        raw_title = item["title"]
        title = _neutralize_markdown_specials(raw_title)
        link = link_map.get(raw_title, item.get("link", ""))
        if link:
            lines.append(f"- {title} [🔗]({link})")
        else:
            lines.append(f"- {title}")
    return lines


def summarize_llm_custom(all_items: dict[str, list[dict]], topics: list[str], ctx: "AlertContext") -> str:
    """LLM curation for custom topic feeds.

    Per-topic substaging: one LLM call per topic, sequential, cached. On a
    per-topic LLM failure, that topic's section in the digest is replaced
    by a raw bullet listing (still useful for the user) and the failure is
    alerted ('topic X fell back'). Other topics deliver normally.

    If every topic falls back, an additional 'all_custom_topics_failed' alert
    fires; the digest still ships in raw form so the user is not left with
    nothing.
    """
    tw_now = datetime.now(timezone(timedelta(hours=8)))
    date_str = tw_now.strftime("%Y/%m/%d (%a)")
    link_map = _build_link_map(all_items)

    log_trace("custom_substage_start", topic_count=len(topics), topics=topics)

    sections: dict[str, list[str]] = {}
    degraded_topics: list[str] = []

    for topic in topics:
        items = all_items.get(topic, [])
        ok, lines, err = _run_custom_topic(topic, items, date_str)
        if ok:
            sections[topic] = lines
        else:
            log_trace("custom_topic_fell_back", topic=topic, error=err)
            sections[topic] = _custom_topic_raw_listing(topic, items, link_map)
            degraded_topics.append(topic)

    if degraded_topics:
        if len(degraded_topics) == len(topics):
            _alert_failure(
                ctx,
                "all_custom_topics_failed",
                f"every custom topic LLM call failed; full digest is raw-listing only. topics={degraded_topics}",
            )
        else:
            _alert_failure(
                ctx,
                "custom_topics_fell_back",
                f"these topics delivered as raw listings (LLM failed): {degraded_topics}",
            )

    lines_out = [f"\U0001f4f0 每日新聞摘要 — {date_str}\n"]
    for topic in topics:
        lines_out.append(f"**{topic}**")
        lines_out.extend(sections.get(topic, ["- 今日無相關新聞"]))
        lines_out.append("")

    digest = "\n".join(lines_out)
    paywall_count = digest.count(PAYWALL_NOTE)
    if paywall_count:
        digest += f"\nℹ️ 本次含 {paywall_count} 則付費牆新聞（原文需訂閱）"
        log_trace("paywall_notice", count=paywall_count)

    log_trace(
        "custom_substage_complete",
        topic_count=len(topics),
        degraded_count=len(degraded_topics),
    )
    return _trim_links_to_limit(digest)


def _resolve_topics(args) -> list[str] | None:
    """Resolve topic list from args: --topics > --account-topics > None (default feeds)."""
    if getattr(args, "topics", None):
        return [t.strip() for t in args.topics.split(",") if t.strip()]
    if getattr(args, "account_topics", False):
        data = load_topics()
        topics = data.get(args.account)
        if topics:
            return topics
    return None


def main():
    parser = argparse.ArgumentParser(description="Fetch and summarize news")
    subs = parser.add_subparsers(dest="command")

    # Default: deliver news (also works with no subcommand)
    deliver_parser = subs.add_parser("deliver", help="Fetch and deliver news")
    for p in [parser, deliver_parser]:
        p.add_argument("--lang", default="zh", help="Language (zh or en)")
        p.add_argument("--deliver-to", help="Telegram chat ID for delivery")
        p.add_argument("--account", default="main", help="Telegram bot account name")
        p.add_argument("--topics", help="Comma-separated custom topics")
        p.add_argument("--account-topics", action="store_true",
                        help="Read topics from news-topics.json by account")

    # Manage subcommand
    manage_parser = subs.add_parser("manage", help="Manage topic subscriptions")
    manage_subs = manage_parser.add_subparsers(dest="action")

    list_p = manage_subs.add_parser("list", help="List subscribed topics")
    list_p.add_argument("--account", default="main")
    list_p.add_argument("--deliver-to", help="Telegram chat ID for delivery")

    add_p = manage_subs.add_parser("add", help="Add a topic")
    add_p.add_argument("--account", default="main")
    add_p.add_argument("--topic", required=True, help="Topic to add")
    add_p.add_argument("--deliver-to", help="Telegram chat ID for delivery")

    remove_p = manage_subs.add_parser("remove", help="Remove a topic")
    remove_p.add_argument("--account", default="main")
    remove_p.add_argument("--topic", required=True, help="Topic to remove")
    remove_p.add_argument("--deliver-to", help="Telegram chat ID for delivery")

    args = parser.parse_args()
    load_env()

    # Handle manage subcommand
    if args.command == "manage":
        if args.action == "list":
            output = manage_list(args.account)
        elif args.action == "add":
            output = manage_add(args.account, args.topic)
        elif args.action == "remove":
            output = manage_remove(args.account, args.topic)
        else:
            parser.print_help()
            sys.exit(1)
        deliver_or_fail(getattr(args, "deliver_to", None), output, account=args.account)
        emit_skill_status("ok")
        emit_trace()
        return

    # Opportunistic cache cleanup before any heavy work.
    _news_cache_sweep()

    # Build the alert context once so every failure path can use it.
    ctx = AlertContext(
        deliver_to=getattr(args, "deliver_to", None),
        account=args.account,
        job_id=os.environ.get("NULLCLAW_JOB_ID"),
    )

    try:
        # Deliver news (default command or explicit "deliver")
        topics = _resolve_topics(args)

        if topics:
            all_items = _fetch_custom_topics(topics)
        else:
            ai_us = fetch_feed(FEEDS["ai_us"])
            ai_labs = fetch_feed(FEEDS["ai_labs"])
            ai_cn = fetch_feed(FEEDS["ai_cn"])
            ai_tw = fetch_feed(FEEDS["ai_tw"])
            tech = fetch_feed(FEEDS["tech"])
            general = fetch_feed(FEEDS["general"])

            all_items = {
                "ai": dedup(ai_us + ai_labs + ai_cn + ai_tw),
                "tech": dedup(tech),
                "general": dedup(general),
            }

        # Decide "feed outage" on the RAW deduped feeds, BEFORE Tier-1 filtering —
        # a deny-list that empties every section is a filter outcome, not a feed
        # outage, and must not trigger the all_feeds_empty alert / exit 1.
        has_items = any(items for items in all_items.values())
        all_items = {k: _tier1_filter_items(v) for k, v in all_items.items()}
        if not has_items:
            # Every RSS feed returned 0 items. There is no news to send.
            # Per the hard rule, alert and exit non-zero.
            _alert_failure(
                ctx,
                "all_feeds_empty",
                "every RSS feed returned 0 items — likely network failure or feed outage",
            )
            emit_skill_status("failed")
            emit_trace()
            sys.exit(1)

        if topics:
            summary = summarize_llm_custom(all_items, topics, ctx)
        else:
            summary = summarize_llm(all_items, ctx)

        if ctx.job_id and ctx.job_id != "interactive":
            summary += f"\n\n`{ctx.job_id}`"

        _deliver_news_or_fail(args.deliver_to, summary, args.account)
        emit_skill_status("ok")
        emit_trace()

    except _AiSubstageExhausted:
        # Alert was already sent from inside _summarize_default_ai_substaged.
        # Exit 1 so cron records the failure; do NOT deliver a partial digest.
        emit_skill_status("failed")
        emit_trace()
        sys.exit(1)

    except SystemExit as se:
        # deliver_or_fail calls sys.exit(1) when telegram.send returns False.
        # The body was printed to stdout for cron capture but the message did
        # NOT reach Telegram. Per the hard rule, alert before propagating.
        # The on-disk failure log catches this even when Telegram is the
        # dead channel.
        if se.code not in (0, None):
            _alert_failure(
                ctx,
                "telegram_delivery_failed",
                "deliver_or_fail exited non-zero — telegram.send returned False",
            )
        raise

    except Exception as e:
        import traceback
        _alert_failure(
            ctx,
            "uncaught_exception",
            f"{type(e).__name__}: {e}\n{traceback.format_exc()[:1500]}",
        )
        emit_skill_status("failed")
        emit_trace()
        raise


if __name__ == "__main__":
    main()
