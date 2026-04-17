import os
import sys
import json
import argparse
import urllib.request
import urllib.parse
import urllib.error
import xml.etree.ElementTree as ET
import re
from datetime import datetime, timezone
from pathlib import Path

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
from heartbeat import run_with_heartbeat  # noqa: E402
import persona_history  # noqa: E402
import persona_registry  # noqa: E402

SKILL_NAME = "mindfulness-spirit"
STREAM_NAME = "mindfulness"
SERIES_SLUG = "inner-algorithm"
HISTORY_LIMIT = 8

# --- Configuration ---
CONFIG_PATH = os.path.expanduser("~/.nullclaw/config.json")
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
SKILL_DIR = os.path.dirname(SCRIPT_DIR)
FAILED_DIR = os.path.join(SKILL_DIR, "failed")
STATUS_DIR = Path.home() / ".nullclaw" / "skills" / "mindfulness-spirit"
STATUS_PATH = STATUS_DIR / "status.json"
AGENT_TIMEOUT_SECS = 300
HEARTBEAT_INTERVAL_SECS = 5
STDOUT_HEARTBEAT_INTERVAL_SECS = 30
RUNS_DIR = os.path.join(SKILL_DIR, "runs")

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


DEFAULT_PERSONA_ROLE = "世界宗教博物館基金會執行室的駐站作家"

def _open_registry_connection():
    """Return (conn, err). Lenient — returns None on failure."""
    try:
        conn = persona_registry.connect_from_env()
        persona_registry.ensure_schema(conn)
        return conn, None
    except persona_registry.MissingCredentialsError as e:
        return None, f"credentials: {e}"
    except Exception as e:  # noqa: BLE001
        return None, f"connect/schema: {e}"


def resolve_persona(mindfulness_config, registry_conn=None):
    """Resolve the writer persona for the article.

    Priority order (first match wins):
      1. env MINDFULNESS_SPIRIT_PERSONA_ROLE (emergency role-only override)
      2. config `skills.mindfulness_spirit.persona_slug` → Turso persona_registry
      3. config `skills.mindfulness_spirit.persona` (literal dict, legacy)
      4. DEFAULT_PERSONA_ROLE fallback
    """
    env_role = os.environ.get("MINDFULNESS_SPIRIT_PERSONA_ROLE")
    if env_role:
        p = persona_registry.Persona(slug="env-override", role=env_role)
        return {
            "slug": p.slug,
            "role": p.role,
            "name": None,
            "voice_notes": None,
            "persona": p,
        }

    if not isinstance(mindfulness_config, dict):
        mindfulness_config = {}

    slug = mindfulness_config.get("persona_slug")
    if isinstance(slug, str) and slug and registry_conn is not None:
        try:
            p = persona_registry.get(registry_conn, slug)
            return {
                "slug": p.slug,
                "role": p.role,
                "name": p.name,
                "voice_notes": p.expression,
                "persona": p,
            }
        except persona_registry.PersonaNotFound:
            print(f"persona_registry: unknown slug '{slug}'; falling through", file=sys.stderr)
        except Exception as exc:  # noqa: BLE001 — lenient: degrade on any registry error
            print(f"persona_registry query failed ({exc}); falling through", file=sys.stderr)

    raw = mindfulness_config.get("persona")
    if isinstance(raw, dict):
        return {
            "slug": raw.get("slug") or "config-literal",
            "role": raw.get("role") or DEFAULT_PERSONA_ROLE,
            "name": raw.get("name"),
            "voice_notes": raw.get("voice_notes"),
            "persona": persona_registry.Persona(
                slug=raw.get("slug") or "config-literal",
                role=raw.get("role") or DEFAULT_PERSONA_ROLE,
            ),
        }

    p = persona_registry.Persona(slug="default", role=DEFAULT_PERSONA_ROLE)
    return {
        "slug": p.slug,
        "role": p.role,
        "name": None,
        "voice_notes": None,
        "persona": p,
    }


def resolve_devto_key(slug, registry_conn):
    """Try persona_registry secret, then DEV_TO_API_KEY env."""
    if registry_conn is not None:
        try:
            key = persona_registry.get_secret(registry_conn, slug, "devto_api_key")
        except Exception as exc:  # noqa: BLE001 — lenient: degrade on any registry error
            print(f"persona_registry secret lookup failed ({exc}); falling through", file=sys.stderr)
            key = None
        if key:
            return key
    env_key = os.environ.get("DEV_TO_API_KEY")
    if env_key:
        if registry_conn is not None:
            print(
                "[back-compat] using DEV_TO_API_KEY env; migrate to persona-skill set-secret",
                file=sys.stderr,
            )
        return env_key
    return None


def load_skill_settings(config, registry_conn=None):
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
        "persona": resolve_persona(mindfulness_config, registry_conn),
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
        stdout_interval_secs=STDOUT_HEARTBEAT_INTERVAL_SECS,
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


def fetch_devto_article(api_key, devto_id):
    """Fetch a single article from dev.to by id. Returns parsed JSON."""
    url = f"https://dev.to/api/articles/{devto_id}"
    headers = {"Accept": "application/json", "api-key": api_key}
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=20) as resp:
        return json.loads(resp.read().decode("utf-8"))


def update_devto_article(api_key, devto_id, body_markdown):
    """PUT updated body_markdown to an existing dev.to article. Returns parsed JSON."""
    url = f"https://dev.to/api/articles/{devto_id}"
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json",
        "api-key": api_key,
    }
    data = json.dumps({"article": {"body_markdown": body_markdown}}).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method="PUT")
    with urllib.request.urlopen(req, timeout=20) as resp:
        return json.loads(resp.read().decode("utf-8"))


def patch_signature(body, author_byline, role):
    """Replace plain-text author signature with translate="no" version.

    Returns (patched_body, changed).
    """
    old_sig = f"*—— {author_byline}（{role}）*"
    new_sig = f'*—— <span translate="no">{author_byline}</span>（{role}）*'
    if old_sig in body and new_sig not in body:
        return body.replace(old_sig, new_sig), True
    return body, False


def cmd_fix_signature(args):
    """Fetch one article from dev.to, patch its author signature, PUT it back."""
    config = load_config()
    registry_conn, _ = _open_registry_connection()
    skill_settings = load_skill_settings(config, registry_conn)
    persona = skill_settings["persona"]
    author_byline = persona.get("name") or persona["slug"]
    role = persona["role"]

    api_key = resolve_devto_key(persona["slug"], registry_conn)
    if registry_conn is not None:
        registry_conn.close()

    if not api_key:
        print("ERROR: no dev.to API key found.", file=sys.stderr)
        return 1

    try:
        article = fetch_devto_article(api_key, args.devto_id)
    except Exception as e:
        print(f"ERROR: fetch failed: {e}", file=sys.stderr)
        return 1

    body = article.get("body_markdown", "")
    patched, changed = patch_signature(body, author_byline, role)

    if not changed:
        print("Signature already has translate=no — no change needed.")
        return 0

    old_sig = f"*—— {author_byline}（{role}）*"
    new_sig = f'*—— <span translate="no">{author_byline}</span>（{role}）*'
    print(f"OLD: {old_sig}")
    print(f"NEW: {new_sig}")

    if args.dry_run:
        print("[DRY-RUN] would update article.")
        return 0

    try:
        update_devto_article(api_key, args.devto_id, patched)
        print(f"Updated article {args.devto_id}: {article.get('url', '')}")
    except Exception as e:
        print(f"ERROR: update failed: {e}", file=sys.stderr)
        return 1

    return 0


def post_to_devto(api_key, title, body, dry_run=False, published=True, main_image_url=None):
    """Return (article_url, devto_id, error). devto_id is None when the API
    response didn't include an integer id or on failure."""
    if dry_run:
        action = "publish to" if published else "create draft on"
        print(f"[Dry-run] Would {action} dev.to: {title}")
        return "https://dev.to/draft/example", None, None

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
            raw_id = res_data.get("id")
            devto_id = int(raw_id) if isinstance(raw_id, int) else None
            if article_url:
                status_label = "published" if published else "draft created"
                print(f"dev.to article {status_label}: {article_url}")
            return article_url, devto_id, None
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
        return None, None, detail
    except Exception as e:
        print(f"Error posting to dev.to: {e}", file=sys.stderr)
        return None, None, str(e)


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


def open_history_connection():
    """Return (conn, err) — lenient wrapper for persona_history.

    err is None on success, a string reason otherwise. Caller proceeds with
    history disabled when err is set.
    """
    try:
        conn = persona_history.connect_from_env()
        persona_history.ensure_schema(conn)
        return conn, None
    except persona_history.MissingCredentialsError as e:
        return None, f"credentials: {e}"
    except Exception as e:  # noqa: BLE001
        return None, f"connect/schema: {e}"


def format_history_for_prompt(rows):
    if not rows:
        return ""
    lines = [
        "=== 最近寫過的文章（嚴格禁止重複） ===",
        "下列是你過去已發表的文章。你**必須**：",
        "1. 標題不可與下列任何標題使用相同句式或相似比喻。",
        "2. 開場段落不可重複相同的提問句型或場景設定。",
        "3. 已引用過的連結不可再次作為主要論據（可提及但不可重述）。",
        "4. 已表達過的立場不可用相似措辭重述——需找到全新切入角度。",
        "",
    ]
    for r in rows:
        stance = (r.stance or "").strip()
        if len(stance) > 120:
            stance = stance[:117] + "…"
        links_str = ""
        if r.key_links:
            truncated = [u[:60] for u in r.key_links[:3]]
            links_str = " · 已用連結: " + ", ".join(truncated)
        lines.append(f"- {r.date} · 《{r.title}》 · 立場: {stance}{links_str}")
    lines.append("")
    return "\n".join(lines) + "\n"


def format_topic_for_prompt(topic):
    """Build a prompt block from the active editorial topic."""
    if topic is None:
        return ""
    lines = [
        "=== 本期主題指引（editorial plan） ===",
        f"系列週次：W{topic.week}",
        f"主題提示：{topic.title_hint}",
        f"切入角度（angle）：{topic.angle}",
        f"觀察視角（lens）：{topic.lens}",
        f"方向：{topic.direction}",
    ]
    if topic.key_question:
        lines.append(f"核心提問：{topic.key_question}")
    lines.append("")
    lines.append(
        "請以上述主題提示和切入角度為本篇文章的主軸。"
        "從素材中選取與此角度相關的新聞，圍繞核心提問展開論述。"
    )
    lines.append("")
    return "\n".join(lines) + "\n"


def extract_urls_from_markdown(md, limit=5):
    """Collect up to `limit` unique URLs from resolved markdown links."""
    urls = []
    seen = set()
    for m in re.finditer(r'\]\((https?://[^)\s]+)\)', md):
        url = m.group(1)
        if url in seen:
            continue
        seen.add(url)
        urls.append(url)
        if len(urls) >= limit:
            break
    return urls


def derive_stance_from_markdown(md, fallback_title):
    """First blockquote line or first body paragraph line, trimmed.

    mindfulness-spirit doesn't (yet) emit an ainews-meta block. Derive a
    short stance mechanically so history rows stay useful; upgrade to a
    prompted stance later (DESIGN §8 option a).
    """
    summary = extract_intro_summary(md)
    if summary and summary != "（無法取得摘要）":
        return summary
    return fallback_title


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command")

    # default (write) subcommand
    p_write = sub.add_parser("write", help="Generate and publish an article (default)")
    p_write.add_argument("--deliver-to", help="Telegram chat ID")
    p_write.add_argument("--account", help="Telegram account name in config")
    p_write.add_argument("--dry-run", action="store_true", help="Don't post to dev.to or Telegram")

    # fix-signature subcommand
    fix = sub.add_parser("fix-signature", help="Patch author signature in a published dev.to article")
    fix.add_argument("devto_id", type=int, help="dev.to article id")
    fix.add_argument("--dry-run", action="store_true", help="Show what would change without updating")

    # top-level flags (backward compat: `run.py --dry-run` without subcommand)
    parser.add_argument("--deliver-to", help="Telegram chat ID")
    parser.add_argument("--account", help="Telegram account name in config")
    parser.add_argument("--dry-run", action="store_true", help="Don't post to dev.to or Telegram")

    args = parser.parse_args()

    if args.command == "fix-signature":
        return cmd_fix_signature(args)

    config = load_config()

    registry_conn, registry_err = _open_registry_connection()
    if registry_err:
        print(
            f"persona_registry disabled ({registry_err}); using fallback persona",
            file=sys.stderr,
        )

    skill_settings = load_skill_settings(config, registry_conn)
    tg_token, tg_chat_id = resolve_telegram_config(config, args)

    persona = skill_settings["persona"]
    history_conn, history_err = open_history_connection()
    if history_err:
        print(
            f"persona_history disabled ({history_err}); proceeding without memory",
            file=sys.stderr,
        )
    history_rows = []
    if history_conn is not None:
        try:
            history_rows = persona_history.recent(
                history_conn,
                persona_slug=persona["slug"],
                limit=HISTORY_LIMIT,
            )
        except Exception as e:  # noqa: BLE001
            print(f"persona_history.recent failed: {e}", file=sys.stderr)

    # Query editorial plan for today's topic
    active_topic = None
    if history_conn is not None:
        try:
            plan = persona_history.get_plan(
                history_conn, skill=SKILL_NAME, series_slug=SERIES_SLUG,
            )
            if plan is not None:
                active_topic = persona_history.next_topic(
                    history_conn, plan_id=plan.id,
                )
                if active_topic:
                    print(f"Editorial plan: W{active_topic.week} — {active_topic.title_hint}")
                else:
                    print("Editorial plan: all topics published or skipped.", file=sys.stderr)
        except Exception as e:  # noqa: BLE001
            print(f"editorial plan query failed: {e}", file=sys.stderr)

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
    history_block = format_history_for_prompt(history_rows)
    topic_block = format_topic_for_prompt(active_topic)
    author_byline = persona.get("name") or persona["slug"]
    signature_block = (
        f"*—— <span translate=\"no\">{author_byline}</span>（{persona['role']}）*\n\n"
        "*本文為個人觀察，立場不代表任何宗教傳統。歡迎各傳統的讀者帶著自身的智慧框架閱讀與回應。*"
    )
    persona_voice_block = ""
    if persona.get("expression"):
        persona_voice_block += f"\n## 表達風格\n{persona['expression']}\n"
    if persona.get("mental_models"):
        persona_voice_block += f"\n## 心智模型\n{persona['mental_models']}\n"
    if persona.get("heuristics"):
        persona_voice_block += f"\n## 啟發法\n{persona['heuristics']}\n"
    if persona.get("antipatterns"):
        persona_voice_block += f"\n## 反模式（避免）\n{persona['antipatterns']}\n"
    if persona.get("limits"):
        persona_voice_block += f"\n## 限制\n{persona['limits']}\n"

    writer_prompt = f"""你是{persona["role"]}，筆名「{author_byline}」，主題是「身心靈 × AI」。
你的讀者是對科技與靈性都感興趣的中文知識工作者。
{persona_voice_block}
寫作原則：
- 跨宗教/跨靈性傳統包容，不獨尊任一宗派
- 非營利視角，不推銷產品、不商業化
- 避免偽科學用語（保證、治癒、靈丹、量子糾纏類比等）
- 繁體中文（台灣用語）
- 2000-3500 字
- 結構：標題 (H1) → 引言 → ## 今日靈感 → ## 深度觀察 → ## 實踐角落 → ## 延伸閱讀

{history_block}{topic_block}下面是今天的素材（編號清單）：
{prompt_items}

**重要：在文章中引用素材時，必須嚴格使用 [來源 #N] 格式（例如 [來源 #1]），不可直接寫 [1] 或 #1。**

**反重複規則（違反任何一條就是不及格）：**
- 若上方「最近寫過的文章」清單不為空，你的標題、開場、和核心論點必須與所有已發表文章**明顯不同**。
- 不可再用已出現過的比喻（例如「工具是為了讓你放手」已經用過，不可變體重述）。
- 開場段落禁止使用「想像一下…」「你有沒有想過…」「當 X 遇上 Y」等已在前幾篇用過的句式。
- 若素材與過去文章的主題高度重疊，你必須找到**完全不同的切入角度**（例如：技術細節、使用者故事、歷史脈絡、批判視角）。

**文末署名（必填，原樣輸出，不要改寫）：**
在最後一段之後插入一條 `---` 分隔線，然後原樣加上下列兩行署名：

{signature_block}

請輸出完整 markdown 文章。"""

    print("Phase: Writer LLM...")
    writer_output, writer_reason = call_nullclaw_agent(writer_prompt, phase="writer", run_id=run_id)
    if not writer_output:
        notify_failure(tg_token, tg_chat_id, "writer", writer_reason or "unknown failure", args.dry_run)
        return 1

    try:
        os.makedirs(RUNS_DIR, exist_ok=True)
        writer_draft_path = os.path.join(RUNS_DIR, f"{run_id}-writer.md")
        with open(writer_draft_path, "w", encoding="utf-8") as f:
            f.write(writer_output)
        print(f"Writer draft saved: {writer_draft_path}")
    except OSError as e:
        print(f"Warning: could not save writer draft: {e}", file=sys.stderr)

    # 3. 檢查清單 LLM
    final_output = writer_output
    checklist_degraded = False
    checklist_prompt = f"""你剛寫完一篇初稿。現在用下面的檢查清單自己審視一次，
不是改錯字，而是確認每一條是否過關：

1. 標題能不能讓人想點開？太平淡就改。
2. 開頭三句能不能勾住讀者？不能就重寫。
3. 哪一段最弱、最像 AI 寫的？砍掉或重寫那一段。
4. 結尾是否流於說教或灌雞湯？如果是，改成具體可實踐的東西。
5. 整篇有沒有一個「值得記住的句子」？沒有就加一個。

請輸出修改後的完整 markdown 文章，不要列出修改清單，
**務必保留原稿中的 [來源 #N] 引用標記，刪除其他多餘的編號標記**，保持繁體中文。
**文末署名區塊（`---` 分隔線後的兩行斜體文字）必須原樣保留，不可改寫或刪除。**

初稿：
<<<
{writer_output}
>>>"""
    print("Phase: Checklist review...")
    checklist_output, checklist_reason = call_nullclaw_agent(checklist_prompt, phase="checklist", run_id=run_id)
    if checklist_output:
        final_output = checklist_output
    else:
        print(f"Checklist phase failed ({checklist_reason}), falling back to writer output.", file=sys.stderr)
        checklist_degraded = True

    # 4. 連結還原
    final_markdown = restore_source_links(final_output, items)

    # 5. dev.to 發布
    dev_to_key = resolve_devto_key(persona["slug"], registry_conn)

    title = "未命名身心靈文章"
    match = re.search(r'^#\s+(.+)$', final_markdown, re.MULTILINE)
    if match:
        title = match.group(1).strip()

    draft_url = None
    devto_id = None
    devto_error = None
    if is_placeholder_secret(dev_to_key):
        devto_error = "DEV_TO_API_KEY 缺失或仍是 placeholder。"
    else:
        draft_url, devto_id, devto_error = post_to_devto(
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
    elif history_conn is not None and not args.dry_run:
        try:
            history_id = persona_history.record(
                history_conn,
                skill=SKILL_NAME,
                stream=STREAM_NAME,
                persona_slug=persona["slug"],
                date=datetime.now().strftime("%Y-%m-%d"),
                title=title,
                stance=derive_stance_from_markdown(final_markdown, title),
                key_links=extract_urls_from_markdown(final_markdown),
                writer_hash=persona_registry.persona_hash(persona["persona"]),
                devto_id=devto_id,
                devto_url=draft_url,
            )
            if active_topic is not None:
                try:
                    persona_history.mark_topic_published(
                        history_conn, active_topic.id, history_id,
                    )
                    print(f"Editorial plan: W{active_topic.week} marked published.")
                except ValueError as e:
                    print(f"mark_topic_published failed: {e}", file=sys.stderr)
        except Exception as e:  # noqa: BLE001
            print(f"persona_history.record failed: {e}", file=sys.stderr)

    # 6. Telegram 通知
    if tg_token and tg_chat_id and not args.dry_run and not devto_error:
        summary = extract_intro_summary(final_markdown)
        header = "📝 今日身心靈專欄已發布" if skill_settings["publish"] else "📝 今日身心靈專欄草稿已生成"
        if checklist_degraded:
            header += "（⚠️ 檢查階段降級）"
        tg_text = f"{header}\n\n《{title}》\n{summary}\n\n🔗 {draft_url or '（發布失敗，請見日誌）'}"
        send_telegram(tg_token, tg_chat_id, tg_text)
    elif args.dry_run:
        print("[Dry-run] Skip Telegram notification.")

    if history_conn is not None:
        history_conn.close()
    if registry_conn is not None:
        registry_conn.close()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
