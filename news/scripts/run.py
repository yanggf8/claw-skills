#!/usr/bin/env python3
"""News skill: fetch Google News RSS feeds and format a daily summary."""
import argparse
import json
import os
import sys
import tempfile
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from datetime import datetime, timezone, timedelta

import tempfile

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
import telegram

TOPICS_FILE = os.path.expanduser("~/.nullclaw/news-topics.json")

# Default topics for accounts without a stored preference
DEFAULT_TOPICS = {
    "main": None,  # None means use the hardcoded AI/tech/general feeds
}


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


def summarize_llm(all_items: dict[str, list[dict]]) -> str:
    """Ask the nullclaw agent to curate and summarize news for significance."""
    import subprocess
    import re

    tw_now = datetime.now(timezone(timedelta(hours=8)))
    date_str = tw_now.strftime("%Y/%m/%d (%a)")

    # Number all items so LLM can reference by ID
    numbered = {}  # id -> {title, link}
    sections = []
    idx = 1
    for label, items in all_items.items():
        if items:
            lines = []
            for it in items:
                numbered[idx] = {"title": it["title"], "link": it.get("link", "")}
                lines.append(f"  #{idx} {it['title']}")
                idx += 1
            sections.append(f"[{label}]\n" + "\n".join(lines))
    raw = "\n".join(sections)

    prompt = (
        f"你是新聞編輯。以下是今天({date_str})從多個來源蒐集的新聞標題（每則有編號 #N），"
        f"分為 AI（美國、中國、台灣、實驗室動態）、科技半導體、以及一般重大新聞。\n\n"
        f"{raw}\n\n"
        f"請做以下工作：\n"
        f"1. 從所有 AI 類新聞中，只挑出真正有影響力、有意義的 5-8 則（重大研究突破、政策變化、產品發布、產業併購等）。"
        f"排除瑣碎的、純行銷推廣的、政治宣傳性質的、投資建議類的新聞。\n"
        f"2. 科技半導體挑 3-5 則重要的。\n"
        f"3. 一般新聞挑 2-3 則最重大的。\n"
        f"4. 用繁體中文輸出，格式嚴格如下（不要加其他說明）：\n\n"
        f"📰 早安新聞摘要 — {date_str}\n\n"
        f"**🤖 AI 人工智慧**\n"
        f"- #N 新聞標題\n"
        f"- ...\n\n"
        f"**💻 科技 & 半導體**\n"
        f"- #N ...\n\n"
        f"**🌏 重大新聞**\n"
        f"- #N ...\n\n"
        f"規則：\n"
        f"- 每則新聞前面必須保留原始編號 #N\n"
        f"- 英文標題翻譯成繁體中文，但保留關鍵專有名詞（公司名、人名）的英文\n"
        f"- 只輸出摘要本身，不要加開場白或結語"
    )
    try:
        result = subprocess.run(
            [os.path.expanduser("~/nullclaw/zig-out/bin/nullclaw"), "agent", "-m", prompt],
            capture_output=True, text=True, timeout=60,
        )
        summary = result.stdout.strip()
        if summary:
            # Replace #N references with links
            def replace_ref(m):
                num = int(m.group(1))
                item = numbered.get(num)
                if item and item["link"]:
                    return f"[🔗]({item['link']}) "
                return ""
            with_links = re.sub(r"#(\d+)\s*", replace_ref, summary)
            # Telegram limit is 4096 chars; if too long, strip links
            if len(with_links) <= 4000:
                return with_links
            # Keep links only for AI section (most valuable), strip rest
            lines = with_links.split("\n")
            in_ai = False
            trimmed = []
            for line in lines:
                if "AI 人工智慧" in line:
                    in_ai = True
                elif line.startswith("**"):
                    in_ai = False
                if not in_ai and "[🔗](" in line:
                    line = re.sub(r"\s*\[🔗\]\([^)]+\)\s*", "", line)
                trimmed.append(line)
            result_text = "\n".join(trimmed)
            if len(result_text) <= 4000:
                return result_text
            # Still too long — strip all links
            return re.sub(r"\s*\[🔗\]\([^)]+\)\s*", "", with_links)
    except Exception as e:
        print(f"[WARN] LLM summary failed: {e}", file=sys.stderr)

    # Fallback: raw listing with links
    link_map = _build_link_map(all_items)
    return fallback_summary(all_items, date_str, link_map)


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
    import subprocess
    import re

    tw_now = datetime.now(timezone(timedelta(hours=8)))
    date_str = tw_now.strftime("%Y/%m/%d (%a)")

    numbered = {}
    sections = []
    idx = 1
    for topic in topics:
        items = all_items.get(topic, [])
        if items:
            lines = []
            for it in items:
                numbered[idx] = {"title": it["title"], "link": it.get("link", "")}
                lines.append(f"  #{idx} {it['title']}")
                idx += 1
            sections.append(f"[{topic}]\n" + "\n".join(lines))
    raw = "\n".join(sections)

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
        result = subprocess.run(
            [os.path.expanduser("~/nullclaw/zig-out/bin/nullclaw"), "agent", "-m", prompt],
            capture_output=True, text=True, timeout=90,
        )
        summary = result.stdout.strip()
        if summary:
            def replace_ref(m):
                num = int(m.group(1))
                item = numbered.get(num)
                if item and item["link"]:
                    return f"[🔗]({item['link']}) "
                return ""
            with_links = re.sub(r"#(\d+)\s*", replace_ref, summary)
            if len(with_links) <= 4000:
                return with_links
            return re.sub(r"\s*\[🔗\]\([^)]+\)\s*", "", with_links)
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
        if getattr(args, "deliver_to", None):
            telegram.send(args.deliver_to, output, account=args.account)
        else:
            print(output)
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

    # Append job instance ID if running under cron
    job_id = os.environ.get("NULLCLAW_JOB_ID")
    if job_id:
        summary += f"\n\n`{job_id}`"

    if args.deliver_to:
        ok = telegram.send(args.deliver_to, summary, account=args.account)
        if ok:
            print(f"Delivered to Telegram chat {args.deliver_to}")
        else:
            print(summary)
            print(f"\n[ERROR] Telegram delivery to {args.deliver_to} failed", file=sys.stderr)
            sys.exit(1)
    else:
        print(summary)


if __name__ == "__main__":
    main()
