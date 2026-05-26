#!/usr/bin/env python3
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import run


class NewsDeliveryFormattingTests(unittest.TestCase):
    def test_neutralize_markdown_specials_basic(self):
        self.assertEqual(
            run._neutralize_markdown_specials("長科*成關鍵受惠股"),
            "長科＊成關鍵受惠股",
        )
        self.assertEqual(
            run._neutralize_markdown_specials("foo_bar baseline_v2"),
            "foo＿bar baseline＿v2",
        )
        self.assertEqual(run._neutralize_markdown_specials("一般新聞標題"), "一般新聞標題")

    def test_attach_numbered_links_sanitizes_headline_asterisk(self):
        summary = "- #1 長科*成關鍵受惠股"
        numbered = {
            1: {
                "title": "長科*成關鍵受惠股",
                "link": "http://example.com/1",
            },
        }

        body, attached = run._attach_numbered_links(summary, numbered)

        self.assertEqual(attached, 1)
        self.assertEqual(body, "- 長科＊成關鍵受惠股 [🔗](http://example.com/1)")

    def test_section_headers_preserved(self):
        self.assertEqual(
            run._neutralize_markdown_specials("**🤖 AI 人工智慧**"),
            "＊＊🤖 AI 人工智慧＊＊",
        )
        summary = "**🤖 AI 人工智慧**\n- #1 長科*成關鍵受惠股"
        numbered = {
            1: {
                "title": "長科*成關鍵受惠股",
                "link": "http://example.com/1",
            },
        }

        body, attached = run._attach_numbered_links(summary, numbered)

        self.assertEqual(attached, 1)
        self.assertIn("**🤖 AI 人工智慧**", body)
        self.assertIn("- 長科＊成關鍵受惠股 [🔗](http://example.com/1)", body)

    def test_attach_links_sanitizes_title_in_fallback_match(self):
        summary = "- 長科*成關鍵受惠股"
        link_map = {"長科*成關鍵受惠股": "http://example.com/1"}

        body = run._attach_links(summary, link_map)

        self.assertEqual(body, "- 長科＊成關鍵受惠股 [🔗](http://example.com/1)")

    def test_full_digest_no_unescaped_asterisks_in_bullets(self):
        items = [{
            "title": "長科*成關鍵受惠股｜產業熱話",
            "link": "http://example.com/1",
            "pub_date": "",
            "source_name": "cnyes",
        }]

        def fake_run_agent(prompt, timeout_secs, variant, all_items, numbered):
            return subprocess.CompletedProcess(
                ["nullclaw"],
                0,
                stdout="- #1 長科*成關鍵受惠股｜產業熱話",
                stderr="",
            )

        with patch.object(run, "_run_nullclaw_agent", fake_run_agent), \
             patch.object(run, "log_trace", lambda *args, **kwargs: None):
            lines, used_fallback = run._summarize_default_section(
                "tech",
                items,
                "2026/05/26 (Tue)",
                {"長科*成關鍵受惠股｜產業熱話": "http://example.com/1"},
            )

        self.assertFalse(used_fallback)
        for line in lines:
            if line.startswith("- "):
                self.assertNotIn("*", line)
        self.assertIn("- 長科＊成關鍵受惠股｜產業熱話 [🔗](http://example.com/1)", lines)

    def test_markdown_chunk_is_safe_accepts_clean_chunk(self):
        self.assertEqual(
            run._markdown_chunk_is_safe("- 標題 [🔗](http://x.com)"),
            (True, ""),
        )

    def test_markdown_chunk_is_safe_rejects_unmatched_asterisk(self):
        ok, reason = run._markdown_chunk_is_safe("- 長科*成關鍵 [🔗](http://x.com)")
        self.assertFalse(ok)
        self.assertIn("asterisk", reason)

    def test_markdown_chunk_is_safe_rejects_unmatched_underscore(self):
        ok, reason = run._markdown_chunk_is_safe("- foo_bar [🔗](http://x.com)")
        self.assertFalse(ok)
        self.assertIn("underscore", reason)

    def test_markdown_chunk_is_safe_accepts_balanced_bold(self):
        self.assertEqual(run._markdown_chunk_is_safe("**🤖 AI**"), (True, ""))

    def test_markdown_chunk_is_safe_rejects_unclosed_link_bracket(self):
        ok, reason = run._markdown_chunk_is_safe("- title [🔗](http://x.com/very-long-id")
        self.assertFalse(ok)
        self.assertIn("link url", reason)

    def test_deliver_news_or_fail_uses_plaintext_fallback_when_chunk_unsafe(self):
        body = "- " + ("安全" * 1895) + "\n- foo*bar [🔗](http://x.com)"
        calls = []
        traces = []

        def fake_deliver(chat_id, body, *, account="main", parse_mode="Markdown", **kwargs):
            calls.append((chat_id, body, account, parse_mode))
            return True

        with patch.object(run, "deliver_or_fail", fake_deliver), \
             patch.object(run, "log_trace", lambda event, **fields: traces.append((event, fields))):
            run._deliver_news_or_fail("chat", body, "main")

        self.assertGreater(len(calls), 1)
        self.assertTrue(all(call[3] is None for call in calls))
        fallback = [fields for event, fields in traces if event == "digest_markdown_unsafe_fallback"]
        self.assertEqual(fallback[0]["unsafe_chunks"], [2])

    def test_deliver_news_or_fail_uses_markdown_when_all_chunks_safe(self):
        body = "- " + ("安全" * 1895) + "\n- clean [🔗](http://x.com)"
        calls = []

        def fake_deliver(chat_id, body, *, account="main", parse_mode="Markdown", **kwargs):
            calls.append(parse_mode)
            return True

        with patch.object(run, "deliver_or_fail", fake_deliver), \
             patch.object(run, "log_trace", lambda *args, **kwargs: None):
            run._deliver_news_or_fail("chat", body, "main")

        self.assertGreater(len(calls), 1)
        self.assertEqual(calls, ["Markdown"] * len(calls))

    def test_deliver_news_or_fail_single_chunk_safe_uses_markdown(self):
        calls = []

        def fake_deliver(chat_id, body, *, account="main", parse_mode="Markdown", **kwargs):
            calls.append(parse_mode)
            return True

        with patch.object(run, "deliver_or_fail", fake_deliver):
            run._deliver_news_or_fail("chat", "- clean [🔗](http://x.com)", "main")

        self.assertEqual(calls, ["Markdown"])

    def test_deliver_news_or_fail_single_chunk_unsafe_uses_plaintext(self):
        calls = []

        def fake_deliver(chat_id, body, *, account="main", parse_mode="Markdown", **kwargs):
            calls.append(parse_mode)
            return True

        with patch.object(run, "deliver_or_fail", fake_deliver), \
             patch.object(run, "log_trace", lambda *args, **kwargs: None):
            run._deliver_news_or_fail("chat", "- foo*bar", "main")

        self.assertEqual(calls, [None])

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
        self.assertEqual(self.calls, [f"{run.AI_SUBSTAGE_CACHE_VARIANT}_0_2", "default_ai_translate"])
        body = "\n".join(lines)
        self.assertIn("參議院民主黨提出 AI 監管法案", body)
        self.assertNotIn("Senate Democrats", body)
        cached = self.cache[("2026/05/15 (Fri)", run.AI_SUBSTAGE_CACHE_VARIANT, 0, 2)]
        self.assertIn("參議院民主黨提出 AI 監管法案", cached)
        self.assertNotIn("Senate Democrats", cached)

    def test_chinese_output_passes_without_translation(self):
        self.set_agent_outputs([
            "- #1 參議院民主黨提出 AI 監管法案\n- #2 Anthropic 發布安全報告",
        ])

        ok, lines, err = run._run_ai_substage(self.items, 0, 2, "2026/05/15 (Fri)")

        self.assertTrue(ok)
        self.assertEqual(err, "")
        self.assertEqual(self.calls, [f"{run.AI_SUBSTAGE_CACHE_VARIANT}_0_2"])
        self.assertIn("參議院民主黨提出 AI 監管法案", "\n".join(lines))
        self.assertIn(("2026/05/15 (Fri)", run.AI_SUBSTAGE_CACHE_VARIANT, 0, 2), self.cache)

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
        self.cache[("2026/05/15 (Fri)", run.AI_SUBSTAGE_CACHE_VARIANT, 0, 2)] = cached
        self.set_agent_outputs([])

        ok, lines, err = run._run_ai_substage(self.items, 0, 2, "2026/05/15 (Fri)")

        self.assertTrue(ok)
        self.assertEqual(lines, [cached])
        self.assertEqual(err, "")
        self.assertEqual(self.calls, [])

    def test_clustered_cache_variant_does_not_reuse_legacy_substage_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(run, "NEWS_CACHE_DIR", tmp):
                old_path = run._news_cache_path("2026/05/15 (Fri)", "default_ai_substage", 0, 2)
                new_path = run._news_cache_path("2026/05/15 (Fri)", run.AI_SUBSTAGE_CACHE_VARIANT, 0, 2)

        self.assertNotEqual(old_path, new_path)
        self.assertIn("default_ai_substage-000-002.txt", old_path)
        self.assertIn("default_ai_clustered_v2-000-002.txt", new_path)


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
        self.assertIn("飆漲", words)
        self.assertNotIn("股價", words)
        self.assertNotIn("reuters", words)

    def test_cluster_groups_cross_language_coverage_with_shared_tokens(self):
        groups = run.cluster([
            _item("Nvidia regains China AI market access - Reuters", "Reuters"),
            _item("Nvidia 輝達重新取得 China AI 市場准入 - 自由財經", "自由財經"),
            _item("Anthropic launches Claude update - TechCrunch", "TechCrunch"),
        ])
        self.assertEqual(len(groups[0]), 2)

    def test_cluster_does_not_chain_merge_through_accumulated_words(self):
        groups = run.cluster([
            _item("Apple unveils iPhone 17 launch event - Reuters", "Reuters"),
            _item("iPhone 17 launch breaks preorder records - TechCrunch", "TechCrunch"),
            _item("Tesla breaks records with quarterly delivery launch - CNBC", "CNBC"),
        ])
        self.assertEqual([len(group) for group in groups], [2, 1])
        self.assertIn("Tesla", groups[1][0]["title"])

    def test_cluster_keeps_generic_cjk_product_phrases_separate(self):
        groups = run.cluster([
            _item("甲公司發布新產品 - cnyes", "cnyes"),
            _item("乙公司發布新產品 - TechCrunch", "TechCrunch"),
        ])
        self.assertEqual(len(groups), 2)

    def test_pick_representatives_uses_cluster_order_without_source_labels(self):
        items = [
            _item("Nvidia China AI market access restored - WSJ", "WSJ"),
            _item("Nvidia China AI market access restored - cnyes", "cnyes"),
            _item("Nvidia China AI market access restored - NVIDIA Blog", "NVIDIA Blog"),
            _item("Anthropic launches Claude update - TechCrunch", "TechCrunch"),
        ]
        clusters = run.cluster(items)
        picked = run.pick_representatives(clusters, per_cluster=1)

        self.assertEqual([item["source_name"] for item in picked], ["WSJ", "TechCrunch"])

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

        trace_events = []

        def fake_log_trace(event, **fields):
            trace_events.append((event, fields))

        with patch.object(run, "_run_nullclaw_agent", fake_run_agent), \
             patch.object(run, "_news_cache_get", lambda *args, **kwargs: None), \
             patch.object(run, "_news_cache_put", lambda *args, **kwargs: None), \
             patch.object(run, "log_trace", fake_log_trace):
            lines = run._summarize_default_ai_substaged(
                items,
                "2026/05/24 (Sun)",
                run.AlertContext(None, "main", "test"),
            )

        body = "\n".join(lines)
        self.assertEqual(body.count("DeepSeek 降低 API 價格"), 1)
        self.assertEqual(len(calls), 2)
        cluster_events = [fields for event, fields in trace_events if event == "cluster_dedup"]
        self.assertEqual(cluster_events, [{
            "before": 4,
            "after": 3,
            "clusters_total": 3,
            "clusters_kept": 3,
        }])


if __name__ == "__main__":
    unittest.main()
