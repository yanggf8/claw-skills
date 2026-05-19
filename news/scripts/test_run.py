#!/usr/bin/env python3
import subprocess
import unittest
from unittest.mock import patch

import run


class NewsDeliveryFormattingTests(unittest.TestCase):
    def test_trim_digest_keeps_markdown_links_when_visible_text_fits(self):
        long_link = "https://news.google.com/rss/articles/" + ("a" * 500)
        lines = [
            "📰 早安新聞摘要",
            "",
            "**🤖 AI 人工智慧**",
        ]
        for idx in range(1, 10):
            lines.append(f"- 測試新聞 {idx} [🔗]({long_link}{idx})")
        body = "\n".join(lines)

        self.assertGreater(len(body), 4000)
        self.assertLessEqual(len(run._markdown_visible_text(body)), 4000)
        self.assertEqual(body, run._trim_digest_links(body))

    def test_split_message_preserves_complete_link_lines(self):
        long_link = "https://news.google.com/rss/articles/" + ("b" * 500)
        lines = [f"- 測試新聞 {idx} [🔗]({long_link}{idx})" for idx in range(1, 12)]
        body = "\n".join(lines)

        chunks = run._split_message_preserving_lines(body, limit=1200)

        self.assertGreater(len(chunks), 1)
        self.assertTrue(all(len(chunk) <= 1200 for chunk in chunks))
        self.assertEqual(body.count("[🔗]("), sum(chunk.count("[🔗](") for chunk in chunks))
        self.assertTrue(all(chunk.count("[🔗](") == chunk.count(")") for chunk in chunks))


class AiSubstageLanguageGateTests(unittest.TestCase):
    def setUp(self):
        self.items = [
            {"title": "OpenAI launches model", "link": "https://example.com/1"},
            {"title": "Anthropic publishes safety report", "link": "https://example.com/2"},
        ]
        self.calls = []
        self.cache = {}

        def fake_cache_get(date_str, variant, start, end):
            return self.cache.get((date_str, variant, start, end))

        def fake_cache_put(date_str, variant, start, end, body):
            self.cache[(date_str, variant, start, end)] = body

        # addCleanup runs even if setUp later raises or a test mutates the
        # attribute mid-flight; direct assignment + tearDown would leak
        # patches into NewsDeliveryFormattingTests on cleanup failure.
        for name, replacement in (
            ("_news_cache_get", fake_cache_get),
            ("_news_cache_put", fake_cache_put),
            ("log_trace", lambda *args, **kwargs: None),
        ):
            self._install_patch(name, replacement)

    def _install_patch(self, name, replacement):
        patcher = patch.object(run, name, replacement)
        patcher.start()
        self.addCleanup(patcher.stop)

    def set_agent_outputs(self, outputs):
        queue = list(outputs)

        def fake_run_agent(prompt, timeout_secs, variant, all_items, numbered):
            self.calls.append(variant)
            return subprocess.CompletedProcess(["nullclaw"], 0, stdout=queue.pop(0), stderr="")

        self._install_patch("_run_nullclaw_agent", fake_run_agent)

    def test_english_output_retries_translation_and_caches_chinese(self):
        self.set_agent_outputs([
            "- #1 Senate Democrats introduce AI bills\n- #2 Anthropic publishes safety report",
            "- #1 參議院民主黨提出 AI 監管法案\n- #2 Anthropic 發布安全報告",
        ])

        ok, lines, err = run._run_ai_substage(self.items, 0, 2, "2026/05/15 (Fri)")

        self.assertTrue(ok)
        self.assertEqual(err, "")
        self.assertEqual(self.calls, ["default_ai_substage_0_2", "default_ai_translate"])
        body = "\n".join(lines)
        self.assertIn("參議院民主黨提出 AI 監管法案", body)
        self.assertNotIn("Senate Democrats", body)
        cached = self.cache[("2026/05/15 (Fri)", "default_ai_substage", 0, 2)]
        self.assertIn("參議院民主黨提出 AI 監管法案", cached)
        self.assertNotIn("Senate Democrats", cached)

    def test_chinese_output_passes_without_translation(self):
        self.set_agent_outputs([
            "- #1 參議院民主黨提出 AI 監管法案\n- #2 Anthropic 發布安全報告",
        ])

        ok, lines, err = run._run_ai_substage(self.items, 0, 2, "2026/05/15 (Fri)")

        self.assertTrue(ok)
        self.assertEqual(err, "")
        self.assertEqual(self.calls, ["default_ai_substage_0_2"])
        self.assertIn("參議院民主黨提出 AI 監管法案", "\n".join(lines))
        self.assertIn(("2026/05/15 (Fri)", "default_ai_substage", 0, 2), self.cache)

    def test_translation_failure_returns_false_and_does_not_cache(self):
        self.set_agent_outputs([
            "- #1 Senate Democrats introduce AI bills\n- #2 Anthropic publishes safety report",
            "- #1 Senate Democrats introduce AI bills\n- #2 Anthropic publishes safety report",
        ])

        ok, lines, err = run._run_ai_substage(self.items, 0, 2, "2026/05/15 (Fri)")

        self.assertFalse(ok)
        self.assertEqual(lines, [])
        self.assertEqual(err, "language_validation")
        self.assertEqual(self.cache, {})

    def test_cache_hit_short_circuits_language_gate(self):
        cached = "- #1 English cache remains until operator clears it"
        self.cache[("2026/05/15 (Fri)", "default_ai_substage", 0, 2)] = cached
        self.set_agent_outputs([])

        ok, lines, err = run._run_ai_substage(self.items, 0, 2, "2026/05/15 (Fri)")

        self.assertTrue(ok)
        self.assertEqual(lines, [cached])
        self.assertEqual(err, "")
        self.assertEqual(self.calls, [])


class CrosshalfDedupParserTests(unittest.TestCase):
    def test_parse_extracts_id_per_line(self):
        stdout = "#3\n#7\n#12\n"
        self.assertEqual(run._parse_crosshalf_keep_ids(stdout), {3, 7, 12})

    def test_parse_tolerates_whitespace_and_blank_lines(self):
        stdout = "  #1\n\n   #5   \n\n#9\n"
        self.assertEqual(run._parse_crosshalf_keep_ids(stdout), {1, 5, 9})

    def test_parse_ignores_chatty_preamble(self):
        # If the LLM ignores the "no preamble" instruction, the parser must
        # still pick out the bare #N lines and drop the prose lines silently.
        stdout = (
            "好的，以下是保留的編號：\n"
            "#2\n"
            "#4\n"
            "(共 2 則)\n"
        )
        self.assertEqual(run._parse_crosshalf_keep_ids(stdout), {2, 4})

    def test_parse_returns_empty_set_on_unparseable_reply(self):
        self.assertEqual(run._parse_crosshalf_keep_ids("no IDs here at all"), set())
        self.assertEqual(run._parse_crosshalf_keep_ids(""), set())

    def test_parse_rejects_inline_id_in_prose(self):
        # `^\s*#N\s*$` is strict on purpose: a stray "#3" inside a sentence
        # must not be mistaken for a keep decision.
        stdout = "保留 #3 與 #7\n"
        self.assertEqual(run._parse_crosshalf_keep_ids(stdout), set())

    def test_apply_filters_bullets_by_id(self):
        bullets = [
            "- #1 first bullet",
            "- #2 second bullet",
            "- #3 third bullet",
        ]
        result = run._apply_crosshalf_keep_ids(bullets, {1, 3})
        self.assertEqual(result, ["- #1 first bullet", "- #3 third bullet"])

    def test_apply_empty_keep_ids_returns_input_unchanged(self):
        # Sentinel: caller treats this as "LLM produced nothing usable, keep all."
        bullets = ["- #1 a", "- #2 b"]
        result = run._apply_crosshalf_keep_ids(bullets, set())
        self.assertIs(result, bullets)

    def test_apply_preserves_unmarked_bullets(self):
        # A formatting glitch (no leading `- #N`) must not silently drop the
        # bullet; this is the "fail open, prefer redundant over missing" rule.
        bullets = [
            "- #1 normal",
            "- glitched bullet missing marker",
            "- #2 also normal",
        ]
        result = run._apply_crosshalf_keep_ids(bullets, {1})
        self.assertEqual(result, ["- #1 normal", "- glitched bullet missing marker"])

    def test_apply_handles_id_not_in_input(self):
        # LLM hallucinates a #99 that was never in the input. The applier must
        # not insert it (we only filter, never synthesize).
        bullets = ["- #1 a", "- #2 b"]
        result = run._apply_crosshalf_keep_ids(bullets, {1, 99})
        self.assertEqual(result, ["- #1 a"])


if __name__ == "__main__":
    unittest.main()
