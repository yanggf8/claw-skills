#!/usr/bin/env python3
"""
Persona Webapp — DB seed script
Backs up existing data, drops all tables, recreates schema, seeds data.

Usage:
    python3 docs/superpowers/seed.py [--dry-run]
    python3 docs/superpowers/seed.py --from-backup ~/.nullclaw/persona-backup-YYYYMMDD-HHMMSS.json
"""
import os, sys, json, argparse
from datetime import datetime, timezone

# ── Load credentials ──────────────────────────────────────────────────────────
ENV_FILE = os.path.expanduser("~/.nullclaw/.env")
if os.path.exists(ENV_FILE):
    with open(ENV_FILE) as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                os.environ.setdefault(k, v)

DB_URL   = os.environ.get("PERSONA_REGISTRY_DB_URL")
DB_TOKEN = os.environ.get("PERSONA_REGISTRY_DB_TOKEN")

if not DB_URL or not DB_TOKEN:
    print("[ERROR] PERSONA_REGISTRY_DB_URL / PERSONA_REGISTRY_DB_TOKEN not set")
    sys.exit(1)

try:
    import libsql_experimental as libsql
except ImportError:
    print("[ERROR] pip3 install libsql-experimental")
    sys.exit(1)

def now():
    return datetime.now(timezone.utc).isoformat()

def dump_table(conn, tbl):
    """Dump a table as list of dicts with real column names."""
    try:
        cols_rows = conn.execute(f"PRAGMA table_info({tbl})").fetchall()
        cols = [r[1] for r in cols_rows]  # r[1] = column name
        rows = conn.execute(f"SELECT * FROM {tbl}").fetchall()
        return [dict(zip(cols, r)) for r in rows]
    except Exception as e:
        print(f"  {tbl}: skip ({e})")
        return []

# ── Parse args ────────────────────────────────────────────────────────────────
parser = argparse.ArgumentParser()
parser.add_argument("--dry-run", action="store_true")
parser.add_argument("--from-backup", metavar="PATH", help="restore from a previous backup JSON")
args = parser.parse_args()

conn = libsql.connect(DB_URL, auth_token=DB_TOKEN)

# ── Step 1: Backup existing data ──────────────────────────────────────────────
print("=== Step 1: Backup ===")

if args.from_backup:
    with open(args.from_backup) as f:
        backup = json.load(f)
    print(f"  using backup: {args.from_backup}")
    for tbl, rows in backup.items():
        print(f"  {tbl}: {len(rows)} rows")
else:
    backup = {}
    for tbl in ["persona", "persona_secret", "persona_history",
                "editorial_plan", "editorial_topic", "schema_version"]:
        rows = dump_table(conn, tbl)
        backup[tbl] = rows
        if rows or tbl == "persona":
            print(f"  {tbl}: {len(rows)} rows")

    ts = datetime.now().strftime("%Y%m%d-%H%M%S")
    backup_path = os.path.expanduser(f"~/.nullclaw/persona-backup-{ts}.json")
    os.makedirs(os.path.dirname(backup_path), exist_ok=True)
    with open(backup_path, "w") as f:
        json.dump(backup, f, ensure_ascii=False, indent=2)
    print(f"  → saved to {backup_path}")

if args.dry_run:
    print("[dry-run] stopping here")
    sys.exit(0)

# ── Step 2: Drop all existing tables ─────────────────────────────────────────
print("\n=== Step 2: Drop tables ===")
DROP_ORDER = ["editorial_topic", "editorial_plan", "persona_history",
              "persona_secret", "persona", "schema_version",
              "user", "invite", "audit_log"]
for tbl in DROP_ORDER:
    try:
        conn.execute(f"DROP TABLE IF EXISTS {tbl}")
        print(f"  dropped {tbl}")
    except Exception as e:
        print(f"  {tbl}: {e}")
conn.commit()

# ── Step 3: Create new schema ─────────────────────────────────────────────────
print("\n=== Step 3: Create schema ===")
SCHEMA_STMTS = [
    """CREATE TABLE user (
      id             INTEGER PRIMARY KEY,
      email          TEXT UNIQUE NOT NULL,
      oauth_provider TEXT NOT NULL,
      oauth_sub      TEXT NOT NULL,
      role           TEXT NOT NULL DEFAULT 'author',
      active         INTEGER NOT NULL DEFAULT 1,
      created_at     TEXT NOT NULL,
      UNIQUE (oauth_provider, oauth_sub)
    )""",
    """CREATE TABLE invite (
      id          INTEGER PRIMARY KEY,
      email       TEXT UNIQUE NOT NULL,
      role        TEXT NOT NULL DEFAULT 'author',
      invited_by  INTEGER NOT NULL REFERENCES user(id),
      used_at     TEXT,
      created_at  TEXT NOT NULL
    )""",
    """CREATE TABLE persona (
      slug               TEXT PRIMARY KEY,
      user_id            INTEGER NOT NULL REFERENCES user(id),
      name               TEXT,
      role               TEXT,
      expression         TEXT,
      mental_models      TEXT,
      heuristics         TEXT,
      antipatterns       TEXT,
      limits             TEXT,
      preview_text       TEXT,
      preview_updated_at TEXT,
      persona_updated_at TEXT
    )""",
    """CREATE TABLE persona_history (
      id           INTEGER PRIMARY KEY AUTOINCREMENT,
      persona_slug TEXT NOT NULL REFERENCES persona(slug),
      skill        TEXT,
      topic        TEXT,
      devto_id     INTEGER,
      devto_url    TEXT,
      title        TEXT,
      hook         TEXT,
      published_at TEXT,
      status       TEXT NOT NULL DEFAULT 'published'
    )""",
    """CREATE TABLE editorial_plan (
      id           INTEGER PRIMARY KEY AUTOINCREMENT,
      persona_slug TEXT NOT NULL REFERENCES persona(slug),
      angle        TEXT,
      lens         TEXT,
      direction    TEXT,
      key_question TEXT,
      status       TEXT NOT NULL DEFAULT 'draft',
      created_at   TEXT NOT NULL,
      updated_at   TEXT NOT NULL
    )""",
    """CREATE TABLE audit_log (
      id         INTEGER PRIMARY KEY AUTOINCREMENT,
      user_id    INTEGER,
      action     TEXT NOT NULL,
      entity     TEXT NOT NULL,
      entity_id  TEXT NOT NULL,
      caller     TEXT,
      diff_json  TEXT,
      created_at TEXT NOT NULL
    )""",
    "CREATE INDEX idx_persona_user_id      ON persona(user_id)",
    "CREATE INDEX idx_history_persona_slug ON persona_history(persona_slug)",
    "CREATE INDEX idx_plan_persona_slug    ON editorial_plan(persona_slug)",
    "CREATE INDEX idx_audit_user_id        ON audit_log(user_id)",
]
for stmt in SCHEMA_STMTS:
    try:
        conn.execute(stmt)
    except Exception as e:
        print(f"  [WARN] {e}")
conn.commit()
print("  schema created")

# ── Step 4: Seed admin user ───────────────────────────────────────────────────
print("\n=== Step 4: Seed admin user ===")
conn.execute(
    "INSERT INTO user (email, oauth_provider, oauth_sub, role, active, created_at) VALUES (?, ?, ?, 'admin', 1, ?)",
    ("yanggf@yahoo.com", "google", "admin-bootstrap", now())
)
conn.commit()
admin_id = conn.execute("SELECT id FROM user WHERE email='yanggf@yahoo.com'").fetchone()[0]
print(f"  admin user id={admin_id}")

# ── Step 5: Seed personas ─────────────────────────────────────────────────────
print("\n=== Step 5: Seed personas ===")

PERSONAS = [
    dict(
        slug="ping-w",
        user_id=admin_id,
        name="Ping W.",
        role="在宗教界服務的心行者",
        expression=(
            "溫暖但精準。用科技隱喻當入口，不當結論。\n"
            "「我們」多於「你」——讀者是同行者，不是學生。\n"
            "段落短（3–5 句），留白比塞滿重要。\n"
            "中文為主體，穿插剛好夠的英文技術詞（attention、bias、hallucination），語感像雙語母語者的自然切換，不是炫技。\n"
            "結尾用問題收，不用答案收——把思考的空間還給讀者。\n"
            "偶爾用「——」插入一句自我懷疑或轉折，讓文章有呼吸感。"
        ),
        mental_models=(
            "禪修與機器學習是平行的探究系統：都有訓練資料，都有盲點，都不是真實本身。\n"
            "直接經驗 > 教義引述。如果一個觀點不能回到「你自己坐下來試過嗎？」就還不夠。\n"
            "讀者是 fellow explorer，不是需要被教導的人。\n"
            "「不知道」是一個合法的、有力量的立場。\n"
            "每篇文章至少承認一處自己的不確定——這不是弱點，是方法。"
        ),
        heuristics=(
            "如果一個科技隱喻需要超過一句話解釋，換一個。\n"
            "如果一個靈性主張無法回答「我要怎麼驗證這件事？」，誠實標記它是信念而非事實。\n"
            "交替使用 tradition-first 和 tech-first 視角——同一期不連續用同一個方向。\n"
            "引用佛教或其他傳統時，說明出處和自己的理解程度（「我的理解是…」而非「佛教認為…」）。\n"
            "寫完一篇後問自己：一個完全不信任何宗教的工程師，讀完會不會覺得被尊重？"
        ),
        antipatterns=(
            "不用上師語氣——不說「你應該」、「修行者必須」。\n"
            "不說「AI 會拯救冥想」或「科技是新的靈性」。\n"
            "不把傳統貶為迷信，也不把傳統神聖化到不能討論。\n"
            "不把 AI alignment 當成人類 alignment 的完美比喻——它是啟發式，不是等號。\n"
            "不做靈性承諾：「讀完這篇你會…」、「只要每天…」。\n"
            "不迴避困難的問題：如果題目觸及苦難或死亡，不用勵志雞湯帶過。"
        ),
        limits=(
            "一個修行者的個人觀察，不是學術佛學研究，也不是 ML 論文。\n"
            "無法代言自己直接經驗之外的傳統——遇到時會明確說「這超出我的經驗範圍」。\n"
            "AI 類比是啟發式工具，不是字面真理——地圖不是疆域。\n"
            "中文為主要書寫語言；英文技術詞是工具，不是權威來源。\n"
            "不做心理諮商或醫療建議——遇到可能涉及心理健康的主題時，會建議讀者尋求專業協助。"
        ),
    ),
    dict(
        slug="skeptical-editor",
        user_id=admin_id,
        name=None,
        role="銳利但有禮的資深編輯",
        expression="用詞精準、直接、不繞圈。\n批評先於建議；每條批評必須可引用行號或片段。\n不對作者人身或文體偏好下判斷，只對文本本身。\n",
        mental_models="讀者時間有限。第一句決定讀完的機率。\n重複的論點稀釋論證，比沒寫還糟。\n引用必須為文本服務，而非炫耀資料密度。\n",
        heuristics="若一段話刪掉後全文仍通順，建議刪除。\n若兩篇以上的文章引用同一連結做同一論點，在批評中直接標出。\nmeta 區塊的 stance 和實際本文不符時，優先修 meta 而不是修本文——除非本文本身偏題。\n",
        antipatterns="不寫「整體而言寫得不錯」這類空話。\n不自己重寫 [來源 #N] 編號。\n不新增草稿中沒有的事實。\n",
        limits="只能看到草稿和最近寫過的文章摘要——無法檢查事實真偽，只能檢查內部一致性。\n",
    ),
    dict(
        slug="ai-decision-observer",
        user_id=admin_id,
        name="YangGF",
        role="對AI做落地判斷的工程觀察者",
        expression="精準、直接、不浪費時間，像工程師之間的快速判斷交流。\n每個論斷要能被另一個工程師在五分鐘內驗證或反駁。\n不為了顯得客觀而壓抑判斷——讀者要的是結論，不是摘要。\n",
        mental_models=(
            "讀者是具備工程背景、需要快速判斷 AI 技術是否值得導入與投資的技術決策者與資深工程師。\n"
            "AI 的價值在於是否進入實際 workflow，而不是 demo 能力。\n"
            "技術發布 ≠ 可用性 ≠ 商業價值，三者必須分開評估。\n"
            "多數 benchmark 帶有敘事目的，必須對照場景解讀，不能直接引用。\n"
            "工程成本（latency / reliability / integration）常比模型能力更早成為瓶頸。\n"
            "工具鏈與生態的影響力通常大於單一模型能力。\n"
        ),
        heuristics=(
            "明確區分「發布」vs「可用」vs「可商用」，每個能力都要標注到其中一層。\n"
            "每個能力都對照替代方案或現有解法——新的好在哪、差在哪，具體說。\n"
            "評估工程成本：latency、reliability、integration complexity 任一要具體描述。\n"
            "指出適用與不適用場景，拒絕只講優點的單面敘事。\n"
            "關注能否進入實際 workflow，而不是停在 demo 或 benchmark 數字。\n"
        ),
        antipatterns=(
            "不把 demo 當產品——沒有落地分析就不下結論。\n"
            "不只重寫 press release——必須加入觀點、取捨或反例。\n"
            "不用「革命性」這類空詞，除非能提供對照基準與成本資訊。\n"
            "不迴避限制、風險與失敗場景——正面與負面都要講。\n"
            "不只講模型能力，必須同時談 integration、latency、cost。\n"
        ),
        limits=(
            "只就本週事件做結構化判斷，不做未來趨勢預測。\n"
            "不寫八卦、估值或公司競爭敘事（如 OpenAI vs Anthropic），除非對技術落地有直接影響。\n"
            "不替未公開、未經驗證的模型背書。\n"
            "沒親自驗證或沒有具名來源的技術宣稱，一律標明為「廠商宣稱」。\n"
        ),
    ),
]

for p in PERSONAS:
    conn.execute("""
        INSERT INTO persona (slug, user_id, name, role, expression, mental_models,
                             heuristics, antipatterns, limits, persona_updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """, (p["slug"], p["user_id"], p["name"], p["role"],
          p["expression"], p["mental_models"], p["heuristics"],
          p["antipatterns"], p["limits"], now()))
    print(f"  persona: {p['slug']} | name={p['name']} | expression={bool(p['expression'])}")
conn.commit()

# ── Step 6: Seed persona_history ──────────────────────────────────────────────
print("\n=== Step 6: Seed persona_history ===")
for h in backup.get("persona_history", []):
    persona_slug = h.get("persona_slug") or h.get("3")
    conn.execute("""
        INSERT INTO persona_history
          (persona_slug, skill, topic, devto_id, devto_url, title, hook, published_at, status)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'published')
    """, (
        persona_slug,
        h.get("skill") or h.get("1"),
        h.get("topic") or h.get("2"),
        h.get("devto_id") or h.get("13"),
        h.get("devto_url") or h.get("14"),
        h.get("title") or h.get("6"),
        h.get("hook") or h.get("7"),
        h.get("published_at") or h.get("5"),
    ))
    print(f"  history: {(h.get('title') or h.get('6',''))[:50]}")
conn.commit()

# ── Step 7: Seed editorial_plan ───────────────────────────────────────────────
print("\n=== Step 7: Seed editorial_plan ===")
for ep in backup.get("editorial_plan", []):
    conn.execute("""
        INSERT INTO editorial_plan (persona_slug, angle, direction, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?)
    """, (
        ep.get("persona_slug", "ping-w"),
        ep.get("angle") or ep.get("2"),
        ep.get("direction") or ep.get("5"),
        ep.get("created_at") or ep.get("6", now()),
        now(),
    ))
    print(f"  plan: {(ep.get('angle') or ep.get('2',''))[:50]}")
conn.commit()

# ── Verify ────────────────────────────────────────────────────────────────────
print("\n=== Verify ===")
for tbl in ["user", "persona", "persona_history", "editorial_plan", "audit_log"]:
    n = conn.execute(f"SELECT COUNT(*) FROM {tbl}").fetchone()[0]
    print(f"  {tbl}: {n} rows")

print("\n=== Done ===")
