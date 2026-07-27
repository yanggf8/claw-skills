#!/usr/bin/env python3
"""RED-phase tests for mindfulness-spirit publish hardening — checklist failure must abort before publish; body must be markdown-safe sanitized (ncchoices stripped, paragraph blank lines preserved)."""
import subprocess
import sys
import unittest
from argparse import Namespace
from pathlib import Path
from unittest.mock import patch

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
sys.path.insert(0, str(SCRIPT_DIR.parent.parent / "lib"))  # lib on path for consistency (run.py will import skill_runner after the fix)

import run


def cp(returncode, stdout="", stderr=""):
    return subprocess.CompletedProcess(["nullclaw"], returncode, stdout=stdout, stderr=stderr)


SETTINGS = {"persona_slug": "test-persona", "publish": True, "main_image_url": None}
ITEMS = [{"id": 1, "title": "測試標題", "url": "https://example.com/1", "source": "Example"}]


class TestMindfulnessSpiritPublishHardening(unittest.TestCase):
    def test_checklist_failure_aborts_before_publish(self):
        """WHY: a failed checklist (incl. timeout 124) must abort BEFORE publishing; current code returns 0 so the raw unreviewed draft still publishes (RED)."""
        agents = [cp(0, "文章內容"), cp(1, "", "boom")]
        with patch.object(run, "run_nullclaw_agent", side_effect=agents):
            body, summary, code = run.run_writer_and_checklist("writer prompt")
        self.assertNotEqual(code, 0)

    def test_cmd_write_does_not_publish_on_checklist_failure(self):
        """WHY: degrade path must never reach the publish step (RED: currently publishes)."""
        agents = [cp(0, "文章內容"), cp(1, "", "boom")]
        calls = []

        def pc_mock(*a, **k):
            calls.append(a)
            return "42"

        with (
            patch.object(run, "load_skill_settings", return_value=SETTINGS),
            patch.object(run, "fetch_material_items", return_value=ITEMS),
            patch.object(run, "render_writer_prompt", return_value="WRITER PROMPT"),
            patch.object(run, "pc", side_effect=pc_mock),
            patch.object(run, "run_nullclaw_agent", side_effect=agents),
        ):
            run.cmd_write(Namespace(dry_run=False))

        for call_args in calls:
            self.assertNotIn("publish", call_args)

    def test_cmd_write_no_validation_ok_on_degrade(self):
        """WHY: a degraded run must not mark a raw draft as validation-ok (RED)."""
        agents = [cp(0, "文章內容"), cp(1, "", "boom")]
        calls = []

        def pc_mock(*a, **k):
            calls.append(a)
            return "42"

        with (
            patch.object(run, "load_skill_settings", return_value=SETTINGS),
            patch.object(run, "fetch_material_items", return_value=ITEMS),
            patch.object(run, "render_writer_prompt", return_value="WRITER PROMPT"),
            patch.object(run, "pc", side_effect=pc_mock),
            patch.object(run, "run_nullclaw_agent", side_effect=agents),
        ):
            run.cmd_write(Namespace(dry_run=False))

        for call_args in calls:
            self.assertNotIn("--validation-ok", call_args)

    def test_cmd_write_sanitizes_body_before_write(self):
        """WHY: body.md must be markdown-safe sanitized — harness tokens gone but paragraph blank lines PRESERVED (collapse_blank_lines=False, not chat mode); current code writes raw checklist stdout so ncchoices survives (RED)."""
        agents = [
            cp(0, "草稿"),
            cp(0, "第一段。\n\n<ncchoices>{\"v\":1}</ncchoices>\n\n第二段。"),
        ]
        calls = []

        def pc_mock(*a, **k):
            calls.append(a)
            return "42"

        with (
            patch.object(run, "load_skill_settings", return_value=SETTINGS),
            patch.object(run, "fetch_material_items", return_value=ITEMS),
            patch.object(run, "render_writer_prompt", return_value="WRITER PROMPT"),
            patch.object(run, "pc", side_effect=pc_mock),
            patch.object(run, "run_nullclaw_agent", side_effect=agents),
        ):
            rc = run.cmd_write(Namespace(dry_run=False))
        self.assertEqual(rc, 0)

        body_text = None
        for call_args in calls:
            if "update-body" in call_args:
                args_list = list(call_args)
                body_idx = args_list.index("--body")
                body_arg = args_list[body_idx + 1]
                body_path = body_arg[1:] if body_arg.startswith("@") else body_arg
                body_text = Path(body_path).read_text(encoding="utf-8")
                break
        self.assertIsNotNone(body_text, "expected an update-body pc call with --body")
        self.assertNotIn("ncchoices", body_text)
        self.assertIn("第一段。\n\n", body_text)
        self.assertIn("\n\n第二段。", body_text)

    def test_writer_output_sanitized_before_checklist(self):
        """WHY: harness tokens in writer output must not leak into the checklist prompt (RED: writer_output embedded raw)."""
        captured = {}
        call_count = {"n": 0}

        def agent_mock(prompt, timeout_secs=300):
            call_count["n"] += 1
            if call_count["n"] == 1:
                return cp(0, "好文章\n\n<ncchoices>{\"v\":1}</ncchoices>")
            captured["checklist_prompt"] = prompt
            return cp(0, "通過")

        with patch.object(run, "run_nullclaw_agent", side_effect=agent_mock):
            run.run_writer_and_checklist("writer prompt")

        # Assert the leaked artifact TAG is gone — not the bare word "ncchoices",
        # which now legitimately appears in the checklist instruction text.
        self.assertNotIn("<ncchoices>", captured["checklist_prompt"])

    def test_success_path_publishes_with_validation_ok(self):
        """WHY: guard test — normal publishing must keep working after the fix (this one passes even before the fix)."""
        agents = [cp(0, "乾淨草稿"), cp(0, "乾淨正文。")]
        calls = []

        def pc_mock(*a, **k):
            calls.append(a)
            return "42"

        with (
            patch.object(run, "load_skill_settings", return_value=SETTINGS),
            patch.object(run, "fetch_material_items", return_value=ITEMS),
            patch.object(run, "render_writer_prompt", return_value="WRITER PROMPT"),
            patch.object(run, "pc", side_effect=pc_mock),
            patch.object(run, "run_nullclaw_agent", side_effect=agents),
        ):
            rc = run.cmd_write(Namespace(dry_run=False))
        self.assertEqual(rc, 0)

        has_publish = any("publish" in call_args for call_args in calls)
        has_validation_ok = any("--validation-ok" in call_args for call_args in calls)
        self.assertTrue(has_publish, "expected a publish pc call")
        self.assertTrue(has_validation_ok, "expected a --validation-ok pc call")


if __name__ == "__main__":
    unittest.main()
