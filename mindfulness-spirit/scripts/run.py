import os
import sys
import json
import argparse
import urllib.request
import urllib.parse
import urllib.error
import xml.etree.ElementTree as ET
import subprocess
import re
from datetime import datetime, timezone
from pathlib import Path

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
from heartbeat import run_with_heartbeat  # noqa: E402

# --- Configuration ---
CONFIG_PATH = os.path.expanduser("~/.nullclaw/config.json")
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
SKILL_DIR = os.path.dirname(SCRIPT_DIR)
FAILED_DIR = os.path.join(SKILL_DIR, "failed")
STATUS_DIR = Path.home() / ".nullclaw" / "skills" / "mindfulness-spirit"
STATUS_PATH = STATUS_DIR / "status.json"
AGENT_TIMEOUT_SECS = 300
HEARTBEAT_INTERVAL_SECS = 5

QUERIES = [
    # English Mindfulness + Tech
    "mindfulness AI", "meditation technology", "AI spirituality",
    # Chinese Mindfulness + Tech
    "冥想 AI", "正念 數位", "身心靈 科技",
    # AI Philosophy/Consciousness
    "AI consciousness", "artificial intelligence philosophy"
]

PLACEHOLDER_PREFIXES = (
    "YOUR_",
    "PLACEHOLDER",
    "REPLACE_ME",
    "CHANGE_ME",
    "EXAMPLE",
    "DUMMY",
    "TEST_",
)


def load_config():
    if os.path.exists(CONFIG_PATH):
        try:
            with open(CONFIG_PATH, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            pass
    return {}


def load_skill_settings(config):
    skills_config = config.get("skills", {})
    mindfulness_config = skills_config.get("mindfulness_spirit", {})
    if not isinstance(mindfulness_config, dict):
        mindfulness_config = {}

    main_image_url = os.environ.get("MINDFULNESS_SPIRIT_MAIN_IMAGE_URL")
    if not main_image_url:
        main_image_url = mindfulness_config.get("main_image_url")

    return {
        "publish": mindfulness_config.get("publish", True),
        "main_image_url": main_image_url,
    }


def get_rss_results(query):
    encoded_query = urllib.parse.quote(query)
    is_chinese = any('\u4e00' <= char <= '\u9fff' for char in query)
    hl = "zh-TW" if is_chinese else "en-US"
    gl = "TW" if is_chinese else "US"
    ceid = "TW:zh-Hant" if is_chinese else "US:en"

    url = f"https://news.google.com/rss/search?q={encoded_query}&hl={hl}&gl={gl}&ceid={ceid}"

    results = []
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
        with urllib.request.urlopen(req, timeout=10) as response:
            tree = ET.parse(response)
            root = tree.getroot()
            for item in root.findall("./channel/item"):
                title_node = item.find("title")
                link_node = item.find("link")
                if title_node is None or link_node is None:
                    continue
                title = title_node.text or ""
                link = link_node.text or ""
                if not title or not link:
                    continue
                source = item.find("source").text if item.find("source") is not None else "Google News"
                results.append((title, link, source))
                if len(results) >= 5:
                    break
    except Exception as e:
        print(f"Error fetching RSS for {query}: {e}", file=sys.stderr)
    return results


def call_nullclaw_agent(prompt, phase, run_id, provider=None, model=None):
    """Run `nullclaw agent -m PROMPT` with a wall-clock heartbeat.

    Returns (stdout_or_none, reason_str). reason_str is None on success,
    or a human-readable failure cause suitable for Telegram notification.
    """
    cmd = ["nullclaw", "agent", "-m", prompt]
    if provider:
        cmd.extend(["--provider", provider])
    if model:
        cmd.extend(["--model", model])

    result = run_with_heartbeat(
        cmd=cmd,
        status_path=STATUS_PATH,
        run_id=run_id,
        phase=phase,
        hard_timeout_secs=AGENT_TIMEOUT_SECS,
        heartbeat_interval_secs=HEARTBEAT_INTERVAL_SECS,
        extra={"prompt_chars": len(prompt)},
    )

    if result.timed_out:
        reason = (
            f"agent hard timeout after {AGENT_TIMEOUT_SECS}s "
            f"(elapsed {result.elapsed_secs:.0f}s)"
        )
        print(f"[{phase}] {reason}", file=sys.stderr)
        if result.stderr:
            print(result.stderr[-500:], file=sys.stderr)
        return None, reason

    # Spawn failure: Popen itself raised. heartbeat.py sets returncode=-1
    # AND error="spawn failed: ..." for this case. Must check before the
    # generic non-zero branch or it gets misreported as "exited code=-1".
    if result.error and result.error.startswith("spawn failed"):
        print(f"[{phase}] {result.error}", file=sys.stderr)
        return None, result.error

    if result.returncode != 0:
        reason = (
            f"agent exited code={result.returncode} "
            f"after {result.elapsed_secs:.0f}s"
        )
        print(f"[{phase}] {reason}", file=sys.stderr)
        if result.stderr:
            print(result.stderr[-500:], file=sys.stderr)
        return None, reason

    return result.stdout.strip(), None


def send_telegram(bot_token, chat_id, text):
    url = f"https://api.telegram.org/bot{bot_token}/sendMessage"
    payload = {
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "Markdown",
        "disable_web_page_preview": False,
    }
    data = urllib.parse.urlencode(payload).encode("utf-8")
    try:
        urllib.request.urlopen(url, data=data, timeout=10)
    except Exception as e:
        print(f"Error sending Telegram: {e}", file=sys.stderr)


def post_to_devto(api_key, title, body, dry_run=False, published=True, main_image_url=None):
    if dry_run:
        action = "publish to" if published else "create draft on"
        print(f"[Dry-run] Would {action} dev.to: {title}")
        return "https://dev.to/draft/example", None

    url = "https://dev.to/api/articles"
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "nullclaw-mindfulness-spirit/0.1",
        "api-key": api_key,
    }
    article = {
        "title": title,
        "body_markdown": body,
        "published": published,
        "tags": ["ai", "mindfulness", "spirituality", "technology"],
    }
    if main_image_url:
        article["main_image"] = main_image_url

    data = json.dumps({"article": article}).encode("utf-8")

    req = urllib.request.Request(url, data=data, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            res_data = json.loads(resp.read().decode("utf-8"))
            article_url = res_data.get("url")
            if article_url:
                status_label = "published" if published else "draft created"
                print(f"dev.to article {status_label}: {article_url}")
            return article_url, None
    except urllib.error.HTTPError as e:
        body_text = ""
        try:
            body_text = e.read().decode("utf-8", errors="replace")
        except Exception:
            pass
        detail = f"HTTP {e.code} {e.reason}"
        if body_text:
            detail = f"{detail}: {body_text[:300]}"
        print(f"Error posting to dev.to: {detail}", file=sys.stderr)
        return None, detail
    except Exception as e:
        print(f"Error posting to dev.to: {e}", file=sys.stderr)
        return None, str(e)


def is_placeholder_secret(value):
    if not value:
        return True
    normalized = value.strip()
    if not normalized:
        return True
    upper_value = normalized.upper()
    if upper_value.startswith(PLACEHOLDER_PREFIXES):
        return True
    placeholder_tokens = ("API_KEY_HERE", "TOKEN_HERE", "PLACEHOLDER", "REPLACE_ME", "CHANGE_ME")
    return any(token in upper_value for token in placeholder_tokens)


def restore_source_links(markdown, items):
    def replace_link(match):
        idx = int(match.group(1)) - 1
        if 0 <= idx < len(items):
            return f"[{items[idx]['source']}]({items[idx]['url']})"
        return match.group(0)

    return re.sub(r'\[來源 #(\d+)\]', replace_link, markdown)


def extract_intro_summary(markdown):
    lines = markdown.splitlines()
    blockquote = []
    collecting_blockquote = False
    for raw_line in lines:
        line = raw_line.strip()
        if not line:
            if collecting_blockquote and blockquote:
                break
            continue
        if line.startswith("#") and not blockquote:
            continue
        if line.startswith(">"):
            collecting_blockquote = True
            cleaned = line.lstrip(">").strip()
            if cleaned:
                blockquote.append(cleaned)
            continue
        if collecting_blockquote and blockquote:
            break

    if blockquote:
        summary = " ".join(blockquote)
        return summary[:100] + ("..." if len(summary) > 100 else "")

    paragraph = []
    for raw_line in lines:
        line = raw_line.strip()
        if not line or line.startswith("#"):
            if paragraph:
                break
            continue
        if line.startswith(">"):
            continue
        paragraph.append(line)
        if len(" ".join(paragraph)) >= 100:
            break

    if paragraph:
        summary = " ".join(paragraph)
        return summary[:100] + ("..." if len(summary) > 100 else "")
    return "（無法取得摘要）"


def save_failed_markdown(markdown):
    os.makedirs(FAILED_DIR, exist_ok=True)
    fail_path = os.path.join(FAILED_DIR, f"{datetime.now().strftime('%Y-%m-%d')}.md")
    with open(fail_path, "w", encoding="utf-8") as f:
        f.write(markdown)
    return fail_path


def resolve_telegram_config(config, args):
    tg_account = args.account or config.get("scheduler", {}).get("alert_account", "main")
    tg_chat_id = args.deliver_to or config.get("scheduler", {}).get("alert_to")
    tg_token = (
        config.get("channels", {})
        .get("telegram", {})
        .get("accounts", {})
        .get(tg_account, {})
        .get("bot_token")
    )
    return tg_token, tg_chat_id


def notify_failure(bot_token, chat_id, stage, reason, dry_run=False):
    message = f"⚠️ mindfulness-spirit 失敗\n\n階段：{stage}\n原因：{reason}"
    print(message, file=sys.stderr)
    if bot_token and chat_id and not dry_run:
        send_telegram(bot_token, chat_id, message)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--deliver-to", help="Telegram chat ID")
    parser.add_argument("--account", help="Telegram account name in config")
    parser.add_argument("--dry-run", action="store_true", help="Don't post to dev.to or Telegram")
    parser.add_argument("--skip-editor", action="store_true", help="Skip the editor LLM phase")
    args = parser.parse_args()

    config = load_config()
    skill_settings = load_skill_settings(config)
    tg_token, tg_chat_id = resolve_telegram_config(config, args)

    run_id = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    try:
        STATUS_DIR.mkdir(parents=True, exist_ok=True)
        if STATUS_PATH.exists():
            STATUS_PATH.unlink()
    except OSError:
        pass

    # 1. RSS 抓取
    all_rss_results = []
    seen_urls = set()
    for q in QUERIES:
        results = get_rss_results(q)
        for t, u, s in results:
            if u not in seen_urls:
                all_rss_results.append((t, u, s))
                seen_urls.add(u)

    if not all_rss_results:
        notify_failure(tg_token, tg_chat_id, "rss", "No RSS results found.", args.dry_run)
        return 1

    items = [{"id": i + 1, "title": t, "url": u, "source": s} for i, (t, u, s) in enumerate(all_rss_results)]

    prompt_items = "\n".join([f"#{x['id']} [{x['source']}] {x['title']}" for x in items])

    # 2. 作家 LLM
    writer_prompt = f"""你是世界宗教博物館基金會執行室的駐站作家，主題是「身心靈 × AI」。
你的讀者是對科技與靈性都感興趣的中文知識工作者。

寫作原則：
- 跨宗教/跨靈性傳統包容，不獨尊任一宗派
- 非營利視角，不推銷產品、不商業化
- 避免偽科學用語（保證、治癒、靈丹、量子糾纏類比等）
- 繁體中文（台灣用語）
- 2000-3500 字
- 結構：標題 (H1) → 引言 → ## 今日靈感 → ## 深度觀察 → ## 實踐角落 → ## 延伸閱讀

下面是今天的素材（編號清單）：
{prompt_items}

**重要：在文章中引用素材時，必須嚴格使用 [來源 #N] 格式（例如 [來源 #1]），不可直接寫 [1] 或 #1。**

請輸出完整 markdown 文章。"""

    print("Phase: Writer LLM...")
    writer_output, writer_reason = call_nullclaw_agent(writer_prompt, phase="writer", run_id=run_id)
    if not writer_output:
        notify_failure(tg_token, tg_chat_id, "writer", writer_reason or "unknown failure", args.dry_run)
        return 1

    # 3. 編輯 LLM
    final_output = writer_output
    editor_degraded = False
    if not args.skip_editor:
        editor_prompt = f"""你是一位資深編輯，剛收到作家的初稿。你的工作不是改錯字，
而是當第二雙眼睛審視這篇文章。請問自己：

1. 標題能不能讓人想點開？太平淡就改。
2. 開頭三句能不能勾住讀者？不能就重寫。
3. 哪一段最弱、最像 AI 寫的？砍掉或重寫那一段。
4. 結尾是否流於說教或灌雞湯？如果是，改成具體可實踐的東西。
5. 整篇有沒有一個「值得記住的句子」？沒有就加一個。

請輸出修改後的完整 markdown 文章，不要列出修改清單，
**務必保留原稿中的 [來源 #N] 引用標記，刪除其他多餘的編號標記**，保持繁體中文。

初稿：
<<<
{writer_output}
>>>"""
        print("Phase: Editor LLM...")
        editor_output, editor_reason = call_nullclaw_agent(editor_prompt, phase="editor", run_id=run_id)
        if editor_output:
            final_output = editor_output
        else:
            print(f"Editor phase failed ({editor_reason}), falling back to writer output.", file=sys.stderr)
            editor_degraded = True

    # 4. 連結還原
    final_markdown = restore_source_links(final_output, items)

    # 5. dev.to 發布
    dev_to_key = os.environ.get("DEV_TO_API_KEY")
    if not dev_to_key:
        dev_to_key = config.get("skills", {}).get("dev_to_api_key")

    title = "未命名身心靈文章"
    match = re.search(r'^#\s+(.+)$', final_markdown, re.MULTILINE)
    if match:
        title = match.group(1).strip()

    draft_url = None
    devto_error = None
    if is_placeholder_secret(dev_to_key):
        devto_error = "DEV_TO_API_KEY 缺失或仍是 placeholder。"
    else:
        draft_url, devto_error = post_to_devto(
            dev_to_key,
            title,
            final_markdown,
            args.dry_run,
            published=skill_settings["publish"],
            main_image_url=skill_settings["main_image_url"],
        )

    if devto_error:
        fail_path = save_failed_markdown(final_markdown)
        notify_failure(
            tg_token,
            tg_chat_id,
            "devto",
            f"{devto_error} 已保存 markdown：{fail_path}",
            args.dry_run,
        )

    # 6. Telegram 通知
    if tg_token and tg_chat_id and not args.dry_run and not devto_error:
        summary = extract_intro_summary(final_markdown)
        header = "📝 今日身心靈專欄已發布" if skill_settings["publish"] else "📝 今日身心靈專欄草稿已生成"
        if editor_degraded:
            header += "（⚠️ 編輯階段降級）"
        tg_text = f"{header}\n\n《{title}》\n{summary}\n\n🔗 {draft_url or '（發布失敗，請見日誌）'}"
        send_telegram(tg_token, tg_chat_id, tg_text)
    elif args.dry_run:
        print("[Dry-run] Skip Telegram notification.")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
