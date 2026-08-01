import argparse
import json
import os
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path
SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
from skill_runner import strip_agent_artifacts

SKILL_NAME, STREAM_NAME = "mindfulness-spirit", "mindfulness"
# The column is operator state, not a constant. A finished season used to turn
# into a weekly not-found here — starting the next one should be a config edit,
# not a code change.
DEFAULT_COLUMN_SLUG = "machine-and-cushion"
HISTORY_LIMIT, TAGS = 8, ["ai", "mindfulness", "spirituality", "technology"]

CONFIG_PATH = Path.home() / ".nullclaw" / "config.json"
SKILL_DIR = Path(__file__).resolve().parent.parent
PROMPTS_DIR = SKILL_DIR / "prompts"
QUERIES = [
    "mindfulness AI", "meditation technology", "AI spirituality", "冥想 AI",
    "正念 數位", "身心靈 科技", "AI consciousness", "artificial intelligence philosophy",
]


def load_config():
    if not CONFIG_PATH.exists():
        return {}
    try:
        return json.loads(CONFIG_PATH.read_text(encoding="utf-8"))
    except Exception as exc:  # noqa: BLE001
        print(f"Warning: could not read {CONFIG_PATH}: {exc}", file=sys.stderr)
        return {}


def load_skill_settings():
    raw = load_config().get("skills", {}).get("mindfulness_spirit", {})
    raw = raw if type(raw) is dict else {}
    slug = raw.get("persona_slug")
    if type(slug) is not str or not slug.strip():
        raise ValueError("missing skills.mindfulness_spirit.persona_slug in ~/.nullclaw/config.json")
    column = raw.get("column_slug")
    column = column.strip() if type(column) is str and column.strip() else DEFAULT_COLUMN_SLUG
    return {
        "persona_slug": slug.strip(),
        "column_slug": column,
        "publish": raw.get("publish", True),
        "main_image_url": raw.get("main_image_url"),
    }


def pc(*args, echo=False):
    result = subprocess.run(["persona-core", *args], check=True, capture_output=True, text=True)
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    if echo and result.stdout:
        print(result.stdout, end="")
    return result.stdout


def run_nullclaw_agent(prompt, timeout_secs=300):
    argv = [os.path.expanduser("~/nullclaw/zig-out/bin/nullclaw"), "agent", "--isolated", "-m", prompt]
    env = {**os.environ, "NULLCLAW_AGENT_TIMING_TRACE": "1"}
    try:
        return subprocess.run(argv, capture_output=True, text=True, timeout=timeout_secs, env=env)
    except subprocess.TimeoutExpired as exc:
        return subprocess.CompletedProcess(
            argv, 124, stdout=(exc.stdout or "")[:10000], stderr=(exc.stderr or "")[:10000]
        )


def agent_failure_reason(result):
    if result.returncode == 124:
        return "timed out after 300s"
    stderr = (result.stderr or "").strip()
    return f"exit code {result.returncode}: {stderr[:500]}" if stderr else f"exit code {result.returncode}"


def get_rss_results(query):
    encoded = urllib.parse.quote(query)
    is_chinese = any("\u4e00" <= char <= "\u9fff" for char in query)
    hl, gl, ceid = ("zh-TW", "TW", "TW:zh-Hant") if is_chinese else ("en-US", "US", "US:en")
    url = f"https://news.google.com/rss/search?q={encoded}&hl={hl}&gl={gl}&ceid={ceid}"
    results = []
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
        with urllib.request.urlopen(req, timeout=10) as response:
            for item in ET.parse(response).getroot().findall("./channel/item"):
                title_node, link_node = item.find("title"), item.find("link")
                if title_node is None or link_node is None:
                    continue
                title, link = title_node.text or "", link_node.text or ""
                if title and link:
                    source_node = item.find("source")
                    source = source_node.text if source_node is not None else "Google News"
                    results.append((title, link, source))
                    if len(results) >= 5:
                        break
    except Exception as exc:  # noqa: BLE001
        print(f"Error fetching RSS for {query}: {exc}", file=sys.stderr)
    return results


def fetch_material_items():
    items, seen = [], set()
    for query in QUERIES:
        for title, url, source in get_rss_results(query):
            if url in seen:
                continue
            seen.add(url)
            items.append({"id": len(items) + 1, "title": title, "url": url, "source": source})
    return items


def prompt_items(items):
    return "\n".join(f"#{item['id']} [{item['source']}] {item['title']}" for item in items)


def material_text(items):
    return "\n".join(f"{item['id']}\t{item['source']}\t{item['url']}" for item in items) + "\n"


def render_writer_prompt(settings, items):
    slug = settings["persona_slug"]
    return (PROMPTS_DIR / "writer.md.tmpl").read_text(encoding="utf-8").format(
        persona_voice_block=pc("personas", "show", slug, "--as-prompt-block"),
        style_block=pc("style", "show", "default", "--as-prompt-block"),
        signature_block=pc("personas", "show", slug, "--as-signature"),
        history_block=pc("history", "list", "--persona", slug, "--as-prompt-block", "--limit", str(HISTORY_LIMIT)),
        topic_block=pc("plans", "next", SKILL_NAME, settings["column_slug"], "--as-prompt-block"),
        prompt_items=prompt_items(items),
    )


def write_inputs(work_dir, writer_prompt, items):
    paths = {"writer": work_dir / "writer.md", "checklist": work_dir / "checklist.md", "material": work_dir / "material.tsv"}
    paths["writer"].write_text(writer_prompt, encoding="utf-8")
    paths["checklist"].write_text((PROMPTS_DIR / "checklist.md.tmpl").read_text(encoding="utf-8"), encoding="utf-8")
    paths["material"].write_text(material_text(items), encoding="utf-8")
    return paths


def run_writer_and_checklist(writer_prompt):
    writer = run_nullclaw_agent(writer_prompt, 300)
    if writer.returncode != 0:
        print(writer.stdout or "", end="")
        print(writer.stderr or "", end="", file=sys.stderr)
        print(f"ERROR: writer agent failed: {agent_failure_reason(writer)}", file=sys.stderr)
        return None, None, 1

    writer_output = writer.stdout or ""
    writer_output = strip_agent_artifacts(writer_output, collapse_blank_lines=False)
    checklist_template = (PROMPTS_DIR / "checklist.md.tmpl").read_text(encoding="utf-8")
    checklist_prompt = checklist_template.replace("{{WRITER_OUTPUT}}", writer_output)
    checklist = run_nullclaw_agent(checklist_prompt, 300)
    if checklist.returncode != 0:
        reason = agent_failure_reason(checklist)
        print(f"[checklist] degraded: {reason}", file=sys.stderr)
        return writer_output, f"checklist phase degraded: {reason}", checklist.returncode

    return checklist.stdout or "", "checklist passed", 0


def cmd_write(args):
    settings = load_skill_settings()
    items = fetch_material_items()
    if not items:
        print("ERROR: No RSS results found.", file=sys.stderr)
        return 1

    work_dir = Path(tempfile.mkdtemp(prefix="mindfulness-spirit-"))
    paths = write_inputs(work_dir, render_writer_prompt(settings, items), items)
    print(f"Writer prompt: {paths['writer']}")
    print(f"Checklist prompt: {paths['checklist']}")
    print(f"Material file: {paths['material']}")
    print(f"RSS items: {len(items)}")
    print(f"Persona: {settings['persona_slug']}")
    print(f"Column: {settings['column_slug']}")
    print(f"Legacy publish config: {settings['publish']}; main_image_url: {settings['main_image_url'] or '(none)'}")
    if args.dry_run:
        print("[Dry-run] Skip agent, prepare, update-body, and publish.")
        return 0

    final_body, validation_summary, code = run_writer_and_checklist(paths["writer"].read_text(encoding="utf-8"))
    if code != 0:
        return code
    final_body = strip_agent_artifacts(final_body, collapse_blank_lines=False)
    paths["body"] = work_dir / "body.md"
    paths["body"].write_text(final_body, encoding="utf-8")

    raw_id = pc("columns", "installments", "prepare", settings["column_slug"], "--print-id").strip()
    installment_id = str(int(raw_id))
    print(f"Prepared installment: {installment_id}")
    pc(
        "columns", "installments", "update-body", installment_id,
        "--body", "@" + str(paths["body"]),
        "--material", "@" + str(paths["material"]),
        "--restore-source-links", "--derive-key-links",
        "--validation-ok",
        "--validation-summary", validation_summary,
        echo=True,
    )
    pc("columns", "installments", "publish", installment_id, "--skill-id", SKILL_NAME, echo=True)
    return 0


def cmd_fix_signature(args):
    settings = load_skill_settings()
    pc_args = ["columns", "installments", "repair-signature", str(args.devto_id), "--persona", settings["persona_slug"]]
    pc_args += ["--dry-run"] if args.dry_run else []
    pc(*pc_args, echo=True)
    return 0


def build_parser():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command")
    sub.add_parser("write", help="Generate and publish an article").add_argument("--dry-run", action="store_true")
    fix = sub.add_parser("fix-signature", help="Patch a published dev.to article signature")
    fix.add_argument("devto_id", type=int, help="dev.to article id")
    fix.add_argument("--dry-run", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    return parser


def main():
    args = build_parser().parse_args()
    return cmd_fix_signature(args) if args.command == "fix-signature" else cmd_write(args)

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        raise SystemExit(1)
    except subprocess.CalledProcessError as exc:
        print(exc.stdout or "", end="")
        print(exc.stderr or "", end="", file=sys.stderr)
        print(f"ERROR: persona-core command failed with exit code {exc.returncode}", file=sys.stderr)
        raise SystemExit(exc.returncode or 1)
