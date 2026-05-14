#!/usr/bin/env python3
"""liko-finance weekly stream runner.

This script keeps the weekly cron prompt thin by delegating durable state,
validation, and delivery to persona-core. Shared agent-first plumbing
(log, run_cmd, call_agent, label parsing, tempfile, cron skill-contract
markers) lives in ``~/clawd/skills/lib/skill_runner.py``.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

# Resolve the shared skill lib. Resolution order matches the project-wide
# convention in scripts/claw_lib.py: $CLAW_SKILLS_LIB overrides, otherwise
# the openclaw default ~/clawd/skills/lib (which on this machine is a
# symlink to ~/a/claw-skills/lib).
_LIB = os.environ.get("CLAW_SKILLS_LIB") or os.path.expanduser("~/clawd/skills/lib")
if _LIB not in sys.path:
    sys.path.insert(0, _LIB)
import skill_runner as sr  # noqa: E402

REPO = Path(os.environ.get("PERSONA_CORE_REPO", "/home/yanggf/a/persona-core"))
STREAM = "weekly-intl-wealth-signals"
PERSONA = "liko-finance"
SKILL = "liko-finance-weekly"
SOURCE_DOC = REPO / "docs/superpowers/specs/2026-04-29-liko-finance-weekly-design/sources.md"

sr.init(SKILL)


def issue_status(issue_id: str) -> str | None:
    rows = sr.run_cmd(["persona-core", "streams", "issues", "list", STREAM], cwd=REPO)
    needle = f"id={issue_id} "
    for line in rows.splitlines():
        if line.startswith(needle):
            match = re.search(r"\bstatus=([^ ]+)", line)
            return match.group(1) if match else None
    return None


def load_context(issue_id: str, target_date: str) -> str:
    persona = sr.run_cmd(["persona-core", "personas", "get", PERSONA], cwd=REPO)
    stream = sr.run_cmd(["persona-core", "streams", "get", STREAM], cwd=REPO)
    try:
        history = sr.run_cmd(
            [
                "persona-core",
                "history",
                "list",
                "--skill",
                SKILL,
                "--stream",
                STREAM,
                "--limit",
                "8",
            ],
            timeout=60,
            cwd=REPO,
        )
    except Exception as exc:
        history = f"(history unavailable: {exc})"
    source_policy = SOURCE_DOC.read_text(encoding="utf-8")
    return "\n\n".join(
        [
            f"ISSUE_ID:\n{issue_id}",
            f"TARGET_DATE:\n{target_date}",
            f"PERSONA:\n{persona}",
            f"STREAM:\n{stream}",
            f"RECENT_HISTORY:\n{history}",
            f"SOURCE_POLICY_DOC:\n{source_policy}",
        ]
    )


def draft_with_agent(context: str, timeout: int) -> str:
    prompt = f"""你是 nullclaw 的 liko-finance weekly skill runner。

請根據下列 persona、stream contract、source policy 與近期歷史，產出本週 issue body。

硬性規則：
- 只寫繁體中文。
- 讀者是台灣稅務居民的高資產家庭，台灣視角優先。
- 不要寫新聞 roundup；只選 1-3 個真正值得檢視的訊號。
- 你必須自行查找並驗證本週 Tier A / Tier B 來源與日期；不要只根據上下文推測新聞。
- 來源不足或無法驗證時，不要硬寫新聞；改寫「本週無顯著訊號」並給保守檢視動作。
- 必須包含且只用這三個一級段落，順序不可變：
  本週訊號
  對應檢視面向
  本週檢視動作
- 這三個段落標題必須是純文字單獨一行；不要加 **、##、emoji、編號或其他裝飾。
- 本週檢視動作每一條必須以其中一個動詞開頭：
  確認 / 檢查 / 盤點 / 詢問 / 比較 / 更新
- 本週檢視動作不得使用編號、粗體或符號開頭；每一行第一個字必須是白名單動詞。
- 不得使用交易建議或商品推薦語氣。
- 如果來源不足，明說本週無顯著訊號，並仍給出保守的檢視動作。
- 不要輸出 dev.to frontmatter，不要輸出 Markdown code fence。

輸出格式必須精確包在以下 marker 中：
BEGIN_ISSUE_BODY
<issue body>
END_ISSUE_BODY

CONTEXT:
{context}
"""
    return sr.call_agent(
        prompt,
        timeout=timeout,
        body_marker=("BEGIN_ISSUE_BODY", "END_ISSUE_BODY"),
    )


def repair_with_agent(body: str, violations: list[str], timeout: int) -> str:
    prompt = f"""你是 liko-finance weekly issue 的驗證修復器。

下方 issue body 未通過 persona-core R1/R2/R3 驗證。請只修格式與違規句，不要新增新聞，不要改變核心內容。

修復規則：
- 三個段落標題必須是純文字單獨一行，且順序為：
  本週訊號
  對應檢視面向
  本週檢視動作
- 不要在三個段落標題加 **、##、emoji、編號或其他符號。
- 本週檢視動作每一條第一個字必須是：
  確認 / 檢查 / 盤點 / 詢問 / 比較 / 更新
- 不要讓動作行以編號、粗體符號或項目符號開頭。
- 不要使用買 / 賣 / 申購 / 贖回 / 轉倉 / 加碼 / 減碼 / 進場 / 出場 作為動作。

驗證錯誤：
{chr(10).join(f"- {v}" for v in violations)}

原文：
{body}

輸出格式必須精確包在以下 marker 中：
BEGIN_ISSUE_BODY
<修復後 issue body>
END_ISSUE_BODY
"""
    return sr.call_agent(
        prompt,
        timeout=timeout,
        body_marker=("BEGIN_ISSUE_BODY", "END_ISSUE_BODY"),
    )


def validate_body_file(path: Path) -> dict:
    stdout = sr.run_cmd(
        ["persona-core", "streams", "issues", "validate-body", f"@{path}"],
        cwd=REPO,
    )
    labels = sr.parse_labels(stdout)
    ok = labels.get("ok") == "yes"
    violations = [
        line[2:].strip()
        for line in stdout.splitlines()
        if line.startswith("- ") and line[2:].strip()
    ]
    return {"ok": ok, "violations": violations, "stdout": stdout}


def validation_result_payload(check: dict, target_date: str) -> str:
    payload = {
        "ok": check["ok"],
        "violations": check["violations"],
        "validated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "validator": "persona-core streams issues validate-body",
        "rules_checked": ["R1", "R2", "R3"],
        "stream": STREAM,
        "target_date": target_date,
        "source_policy": str(SOURCE_DOC),
    }
    return json.dumps(payload, ensure_ascii=False, indent=2)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true", help="Draft and validate, but do not write or publish")
    parser.add_argument("--check", action="store_true", help="Check persona-core/nullclaw prerequisites only")
    parser.add_argument("--agent-timeout", type=int, default=900)
    args = parser.parse_args(argv)

    try:
        sr.run_cmd(["persona-core", "doctor"], timeout=60, cwd=REPO)
        sr.run_cmd(["persona-core", "streams", "issues", "--help"], timeout=30, cwd=REPO)
        if args.check:
            sr.emit_status("ok")
            sr.emit_trace()
            return 0

        prepared = sr.parse_labels(
            sr.run_cmd(
                ["persona-core", "streams", "issues", "prepare", STREAM],
                timeout=120,
                cwd=REPO,
            )
        )
        issue_id = prepared["issue_id"]
        target_date = prepared["target_date"]
        status = issue_status(issue_id)
        sr.log(f"issue_id={issue_id} target_date={target_date} status={status}")

        if status == "delivered":
            sr.log("issue already delivered; no-op")
            sr.emit_status("ok")
            sr.emit_trace()
            return 0

        context = load_context(issue_id, target_date)
        body = draft_with_agent(context, args.agent_timeout)
        body_path = sr.write_temp_text(f"liko-{target_date}-", ".md", body)
        check = validate_body_file(body_path)
        if not check["ok"]:
            sr.log("initial validation failed; asking agent for one repair pass")
            body = repair_with_agent(body, check["violations"], args.agent_timeout)
            body_path = sr.write_temp_text(f"liko-{target_date}-repair-", ".md", body)
            check = validate_body_file(body_path)
        result_path = sr.write_temp_text(
            f"liko-{target_date}-validation-",
            ".json",
            validation_result_payload(check, target_date),
        )

        if not check["ok"]:
            sr.log("validation failed")
            sr.log(check["stdout"].strip())
            if not args.dry_run:
                sr.run_cmd(
                    [
                        "persona-core",
                        "streams",
                        "issues",
                        "update-body",
                        issue_id,
                        "--validation-result",
                        f"@{result_path}",
                        "--status",
                        "skipped",
                    ],
                    timeout=120,
                    cwd=REPO,
                )
            sr.emit_status("failed")
            sr.emit_trace()
            return 2

        if args.dry_run:
            print(f"dry_run: yes")
            print(f"issue_id: {issue_id}")
            print(f"target_date: {target_date}")
            print(f"body_path: {body_path}")
            print(f"validation_result_path: {result_path}")
            print("would_update_body: yes")
            print("would_publish: yes")
            sr.emit_status("ok")
            sr.emit_trace()
            return 0

        sr.run_cmd(
            [
                "persona-core",
                "streams",
                "issues",
                "update-body",
                issue_id,
                "--body",
                f"@{body_path}",
                "--validation-result",
                f"@{result_path}",
                "--status",
                "validated",
            ],
            timeout=120,
            cwd=REPO,
        )
        publish_out = sr.run_cmd(
            ["persona-core", "streams", "issues", "publish", issue_id, "--target", "both"],
            timeout=180,
            cwd=REPO,
        )
        print(publish_out.strip())
        sr.emit_status("ok")
        sr.emit_trace()
        return 0
    except Exception as exc:
        sr.log(f"ERROR {exc}")
        sr.emit_status("failed")
        sr.emit_trace()
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
