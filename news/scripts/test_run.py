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


def _item(title, source="Reuters", link="https://example.com/news"):
    return {
        "title": title,
        "source_name": source,
        "link": link,
        "pub_date": "",
    }


class NewsClusteringTests(unittest.TestCase):
    def test_topic_words_latin(self):
        words = run._topic_words("The new Gemini app at Google I/O - blog.google")
        self.assertNotIn("the", words)
        self.assertNotIn("new", words)
        self.assertNotIn("blog.google", words)
        self.assertIn("gemini", words)
        self.assertIn("google", words)

    def test_topic_words_cjk_bigrams(self):
        words = run._topic_words("輝達黃仁勳發表新晶片 - 自由財經")
        self.assertIn("輝達", words)
        self.assertIn("黃仁", words)
        self.assertIn("仁勳", words)
        groups = run.cluster([
            _item("輝達黃仁勳發表新晶片 - 自由財經", "自由財經"),
            _item("黃仁勳談輝達晶片需求 - 中央社", "中央社"),
        ])
        self.assertEqual(len(groups[0]), 2)

    def test_topic_words_mixed(self):
        words = run._topic_words("Nvidia 輝達股價飆漲 - Reuters")
        self.assertIn("nvidia", words)
        self.assertIn("輝達", words)
        self.assertIn("股價", words)
        self.assertNotIn("reuters", words)

    def test_cluster_groups_cross_language_coverage_with_shared_tokens(self):
        groups = run.cluster([
            _item("Nvidia regains China AI market access - Reuters", "Reuters"),
            _item("Nvidia 輝達重新取得 China AI 市場准入 - 自由財經", "自由財經"),
            _item("Anthropic launches Claude update - TechCrunch", "TechCrunch"),
        ])
        self.assertEqual(len(groups[0]), 2)

    def test_pick_representatives_prefers_primary_then_free(self):
        items = [
            _item("Nvidia China AI market access restored - WSJ", "WSJ"),
            _item("Nvidia China AI market access restored - cnyes", "cnyes"),
            _item("Nvidia China AI market access restored - NVIDIA Blog", "NVIDIA Blog"),
        ]
        picked = run.pick_representatives(items, per_cluster=1)
        self.assertEqual(picked[0]["source_name"], "NVIDIA Blog")

        picked_without_primary = run.pick_representatives(items[:2], per_cluster=1)
        self.assertEqual(picked_without_primary[0]["source_name"], "cnyes")

    def test_summarize_default_ai_no_cross_half_duplicates(self):
        items = [
            _item("DeepSeek discount cuts API prices in China - Reuters", "Reuters", "https://example.com/1"),
            _item("DeepSeek discount cuts API prices for developers - TechCrunch", "TechCrunch", "https://example.com/2"),
            _item("OpenAI 發布新模型測試 - OpenAI", "OpenAI", "https://example.com/3"),
            _item("Anthropic 發布 Claude 安全報告 - Anthropic", "Anthropic", "https://example.com/4"),
        ]
        calls = []

        def fake_run_agent(prompt, timeout_secs, variant, all_items, numbered):
            calls.append((variant, list(numbered.values())))
            lines = []
            for num, item in numbered.items():
                title = item["title"]
                if "DeepSeek discount" in title:
                    title = "DeepSeek 降低 API 價格"
                elif "OpenAI" in title:
                    title = "OpenAI 發布新模型測試"
                elif "Anthropic" in title:
                    title = "Anthropic 發布 Claude 安全報告"
                lines.append(f"- #{num} {title}")
            stdout = "\n".join(lines)
            return subprocess.CompletedProcess(["nullclaw"], 0, stdout=stdout, stderr="")

        with patch.object(run, "_run_nullclaw_agent", fake_run_agent), \
             patch.object(run, "_news_cache_get", lambda *args, **kwargs: None), \
             patch.object(run, "_news_cache_put", lambda *args, **kwargs: None), \
             patch.object(run, "log_trace", lambda *args, **kwargs: None):
            lines = run._summarize_default_ai_substaged(
                items,
                "2026/05/24 (Sun)",
                run.AlertContext(None, "main", "test"),
            )

        body = "\n".join(lines)
        self.assertEqual(body.count("DeepSeek 降低 API 價格"), 1)
        self.assertEqual(len(calls), 2)


if __name__ == "__main__":
    unittest.main()
