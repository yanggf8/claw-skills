#!/usr/bin/env python3
"""News skill: fetch Google News RSS feeds and format a daily summary."""
import argparse
import json
import os
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

TOPICS_FILE = os.path.expanduser("~/.nullclaw/news-topics.json")
TRACE_FILE = os.path.expanduser("~/.nullclaw/skill-traces.jsonl")
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


def fetch_feed(url: str, max_items: int = 15) -> list[dict]:
    """Fetch RSS feed and return list of {title, link, pub_date}."""
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "nullclaw-news/1.0"})
        with urllib.request.urlopen(req, timeout=15) as resp:
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
                items.append({"title": title, "link": link, "pub_date": pub})
    except ET.ParseError:
        pass
    return items


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
            if link:
                result.append(f"{line} [🔗]({link})")
            else:
                result.append(line)
        else:
            result.append(line)
    return "\n".join(result)


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
    return chinese * 5 >= total * 4


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
            numbered[idx] = {"title": it["title"], "link": it.get("link", "")}
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


def _run_nullclaw_agent(prompt: str, timeout_secs: int, variant: str, all_items: dict[str, list[dict]], numbered: dict[int, dict]):
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
        if link:
            lines.append(f"- {title} [🔗]({link})")
        else:
            lines.append(f"- {title}")
    return lines


def _attach_numbered_links(summary: str, numbered: dict[int, dict]) -> tuple[str, int]:
    import re

    attached = {"count": 0}

    marker_line = re.compile(r"^\s*(?:-\s*)?#(\d+)\b(?!,)\s*")

    def replace_line(line: str) -> str:
        match = marker_line.match(line)
        if not match:
            return line
        num = int(match.group(1))
        item = numbered.get(num)
        body = line[match.end():].lstrip()
        if item and item["link"]:
            attached["count"] += 1
            return f"- {body} [🔗]({item['link']})"
        return f"- {body}" if body else "-"

    return "\n".join(replace_line(line) for line in summary.splitlines()), attached["count"]


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


def _trim_lines_to_limit(text: str, limit: int = 4000) -> str:
    if len(text) <= limit:
        return text

    lines = text.splitlines()
    for idx in range(len(lines) - 1, -1, -1):
        if not lines[idx].lstrip().startswith("-"):
            continue
        del lines[idx]
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
) -> list[str] | None:
    if not selected_ids:
        return None

    selected_numbered = {idx: numbered[idx] for idx in selected_ids if idx in numbered}
    raw = "\n".join(f"#{idx} {item['title']}" for idx, item in selected_numbered.items())
    prompt = (
        f"你是新聞標題翻譯編輯。以下是今天({date_str})已選出的新聞標題，每則有編號 #N。\n\n"
        f"{raw}\n\n"
        f"請只把每則標題翻譯成繁體中文，保留公司名、人名、產品名英文，且不要改變編號或順序。\n"
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
        with_links, links_attached = _attach_numbered_links(summary, selected_numbered)
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
    if len(text) <= 4000:
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


def _summarize_default_section(key: str, items: list[dict], date_str: str, link_map: dict[str, str]) -> list[str]:
    spec = DEFAULT_SECTION_SPECS[key]
    if not items:
        return ["- 今日無相關新聞"]

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
        f"- 英文標題翻譯成繁體中文，但保留關鍵專有名詞（公司名、人名）的英文\n"
        f"- 每行必須以繁體中文新聞句子開始，不要輸出英文原標題或「英文（中文）」格式\n"
        f"- 排除瑣碎的、純行銷推廣的、政治宣傳性質的、投資建議類新聞"
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
                    translated = _translate_selected_section(
                        key,
                        _extract_leading_marker_ids(summary, numbered),
                        numbered,
                        date_str,
                    )
                    if translated is not None:
                        return translated
                else:
                    with_links, links_attached = _attach_numbered_links(summary, numbered)
                    if links_attached > 0:
                        return with_links.splitlines()
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

    return _fallback_section_lines(key, items, spec["fallback_limit"], link_map)


def summarize_llm(all_items: dict[str, list[dict]]) -> str:
    """Ask the nullclaw agent to curate and summarize news for significance."""
    tw_now = datetime.now(timezone(timedelta(hours=8)))
    date_str = tw_now.strftime("%Y/%m/%d (%a)")

    link_map = _build_link_map(all_items)
    lines = [f"\U0001f4f0 早安新聞摘要 — {date_str}\n"]
    section_keys = ("ai", "tech", "general")
    section_results = {}
    for key in section_keys:
        try:
            section_results[key] = _summarize_default_section(
                key,
                all_items.get(key, []),
                date_str,
                link_map,
            )
        except Exception as e:
            print(f"[WARN] LLM section worker failed: section={key} {e}", file=sys.stderr)
            spec = DEFAULT_SECTION_SPECS[key]
            section_results[key] = _fallback_section_lines(key, all_items.get(key, []), spec["fallback_limit"], link_map)

    for key in section_keys:
        spec = DEFAULT_SECTION_SPECS[key]
        lines.append(spec["header"])
        lines.extend(section_results[key])
        lines.append("")
    return _trim_digest_links("\n".join(lines))


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
                title = item["title"]
                link = (link_map or {}).get(title, item.get("link", ""))
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


def _fetch_custom_topics(topics: list[str]) -> dict[str, list[dict]]:
    """Fetch feeds for custom topic list, keyed by topic name."""
    all_items: dict[str, list[dict]] = {}
    for topic in topics:
        items = fetch_feed(_topic_feed_url(topic), max_items=10)
        all_items[topic] = dedup(items)
    return all_items


def summarize_llm_custom(all_items: dict[str, list[dict]], topics: list[str]) -> str:
    """LLM curation for custom topic feeds."""
    tw_now = datetime.now(timezone(timedelta(hours=8)))
    date_str = tw_now.strftime("%Y/%m/%d (%a)")

    numbered, raw = _number_items_for_prompt(
        all_items,
        labels=topics,
        limits={topic: LLM_CUSTOM_TOPIC_LIMIT for topic in topics},
    )

    topic_list = "、".join(topics)
    topic_format = "\n".join(f"**{t}**\n- #N ...\n" for t in topics)

    prompt = (
        f"你是新聞編輯。以下是今天({date_str})從多個來源蒐集的新聞標題（每則有編號 #N），"
        f"涵蓋以下主題：{topic_list}。\n\n"
        f"{raw}\n\n"
        f"請做以下工作：\n"
        f"1. 從每個主題中，挑出真正有影響力、有意義的 2-4 則新聞。"
        f"排除瑣碎的、純行銷推廣的、政治宣傳性質的新聞。\n"
        f"2. 用繁體中文輸出，格式嚴格如下（不要加其他說明）：\n\n"
        f"📰 每日新聞摘要 — {date_str}\n\n"
        f"{topic_format}\n"
        f"規則：\n"
        f"- 每則新聞前面必須保留原始編號 #N\n"
        f"- 英文標題翻譯成繁體中文，但保留關鍵專有名詞的英文\n"
        f"- 如果某主題今日無相關新聞，該區塊寫「今日無相關新聞」\n"
        f"- 只輸出摘要本身，不要加開場白或結語"
    )
    try:
        result = _run_nullclaw_agent(prompt, LLM_CUSTOM_TIMEOUT_SECS, "custom", all_items, numbered)
        summary = result.stdout.strip()
        if summary:
            marked_bullets, total_bullets = _marker_validation_stats(summary, numbered)
            if total_bullets == 0 or marked_bullets != total_bullets:
                _log_llm_validation_failed(
                    "custom",
                    result,
                    summary,
                    marked_bullets,
                    total_bullets,
                    numbered,
                    "marker_validation",
                )
                print(
                    "[WARN] LLM summary marker validation failed: "
                    f"{marked_bullets}/{total_bullets} bullets have valid #N markers",
                    file=sys.stderr,
                )
            else:
                chinese_bullets, language_total = _language_validation_stats(summary)
                if not _language_validation_passed(summary):
                    _log_llm_validation_failed(
                        "custom",
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
                    raise ValueError("LLM custom summary failed language validation")

                with_links, links_attached = _attach_numbered_links(summary, numbered)
                if total_bullets > 0 and links_attached == 0:
                    log_trace(
                        "llm_link_validation_failed",
                        variant="custom",
                        returncode=result.returncode,
                        stdout_len=len(result.stdout or ""),
                        items_numbered=len(numbered),
                        marked_bullets=marked_bullets,
                        total_bullets=total_bullets,
                        stdout_sample=_clip_subprocess_text(summary, 1200),
                        line_sample=_sample_nonempty_lines(summary, 8),
                    )
                    print(
                        "[WARN] LLM summary link validation failed: "
                        f"0 links attached for {total_bullets} bullets",
                        file=sys.stderr,
                    )
                else:
                    return _trim_links_to_limit(with_links)
        else:
            _log_llm_validation_failed(
                "custom",
                result,
                summary,
                0,
                0,
                numbered,
                "empty_stdout",
            )
            print("[WARN] LLM summary validation failed: empty stdout", file=sys.stderr)
    except Exception as e:
        print(f"[WARN] LLM summary failed: {e}", file=sys.stderr)

    # Fallback: raw listing
    lines = [f"\U0001f4f0 每日新聞摘要 — {date_str}\n"]
    link_map = _build_link_map(all_items)
    for topic in topics:
        lines.append(f"**{topic}**")
        items = all_items.get(topic, [])
        if items:
            for item in items[:5]:
                title = item["title"]
                link = link_map.get(title, item.get("link", ""))
                if link:
                    lines.append(f"- {title} [🔗]({link})")
                else:
                    lines.append(f"- {title}")
        else:
            lines.append("- 今日無相關新聞")
        lines.append("")
    return "\n".join(lines)


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

    # Deliver news (default command or explicit "deliver")
    topics = _resolve_topics(args)

    if topics:
        all_items = _fetch_custom_topics(topics)
        summary = summarize_llm_custom(all_items, topics)
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

        summary = summarize_llm(all_items)

    has_items = any(items for items in all_items.values())

    # Append job instance ID if running under cron
    job_id = os.environ.get("NULLCLAW_JOB_ID")
    if job_id:
        summary += f"\n\n`{job_id}`"

    deliver_or_fail(args.deliver_to, summary, account=args.account)
    emit_skill_status("ok" if has_items else "failed")
    emit_trace()


if __name__ == "__main__":
    main()
