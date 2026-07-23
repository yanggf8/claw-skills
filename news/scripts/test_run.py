#!/usr/bin/env python3
import collections
import os
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

    def test_attach_numbered_links_drops_stray_bracket_markers(self):
        # A production digest leaked LLM-emitted instrumentation markers
        # ([hint_picked:none], [hints_used:false], [cat_dedup:0]) into the tech
        # section; their underscores broke Markdown and forced a plaintext
        # fallback that expanded every [🔗] link. Such pure-bracket noise lines
        # must be dropped (and traced), while headers and bullets survive.
        summary = "**💻 科技**\n- #1 正常標題\n[hint_picked:none]\n[hints_used:false]\n[cat_dedup:0]"
        numbered = {1: {"title": "正常標題", "link": "http://example.com/1"}}
        traces = []
        with patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            body, attached = run._attach_numbered_links(summary, numbered)
        self.assertEqual(attached, 1)
        for gone in ("hint_picked", "hints_used", "cat_dedup"):
            self.assertNotIn(gone, body, gone)
        self.assertIn("- 正常標題 [🔗](http://example.com/1)", body)
        self.assertIn("**💻 科技**", body)                    # header preserved
        dropped = [f for e, f in traces if e == "section_noise_dropped"]
        self.assertEqual(len(dropped), 1, traces)
        self.assertEqual(dropped[0].get("count"), 3)

    def test_attach_numbered_links_output_markdown_safe_after_dropping_markers(self):
        # The whole point: after dropping the noise the chunk must pass the
        # markdown-safety probe, so delivery stays Markdown (links collapse to 🔗)
        # instead of falling back to plaintext (raw URLs).
        # Exactly the production trio: 3 underscores (hint_picked, hints_used,
        # cat_dedup) = odd count = "unmatched underscore" until they are dropped.
        summary = "- #1 標題\n[hint_picked:none]\n[hints_used:false]\n[cat_dedup:0]"
        numbered = {1: {"title": "標題", "link": "http://example.com/1"}}
        with patch.object(run, "log_trace", lambda *a, **k: None):
            body, _ = run._attach_numbered_links(summary, numbered)
        safe, reason = run._markdown_chunk_is_safe(body)
        self.assertTrue(safe, f"unsafe: {reason} :: {body!r}")

    def test_section_strips_leaked_markers_end_to_end(self):
        # Composition proof for the marker-leak fix: an LLM tech-section output
        # carrying the exact production noise ([skill-result:tech], [hint_picked:none],
        # [hints_used:false], [cat_dedup:0]) must come out of the REAL
        # _summarize_default_section marker-free and Markdown-safe. Only precheck and
        # paywall resolution are stubbed; _attach_numbered_links runs for real so the
        # drop actually happens on the assembled section, not just in isolation.
        items = [
            _item("台積電宣布美國新廠投產", "src", "https://example.com/1"),
            _item("記憶體報價連三月上漲", "src", "https://example.com/2"),
            _item("聯發科發表新旗艦晶片", "src", "https://example.com/3"),
        ]

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout=(
                    "[skill-result:tech]\n"
                    "- #1 台積電宣布美國新廠投產\n"
                    "- #2 記憶體報價連三月上漲\n"
                    "- #3 聯發科發表新旗艦晶片\n"
                    "[hint_picked:none]\n"
                    "[hints_used:false]\n"
                    "[cat_dedup:0]"
                ),
                stderr="",
            )

        traces = []
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "_precheck_apply",
                          lambda summary, numbered, section: (summary, {})), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            lines, used_fallback = run._summarize_default_section(
                "tech", items, "2026/07/21 (Tue)", {},
            )

        self.assertFalse(used_fallback)
        body = "\n".join(lines)
        for gone in ("skill-result", "hint_picked", "hints_used", "cat_dedup"):
            self.assertNotIn(gone, body, gone)
        safe, reason = run._markdown_chunk_is_safe(body)
        self.assertTrue(safe, f"section body not markdown-safe: {reason} :: {body!r}")
        self.assertIn("台積電宣布美國新廠投產", body)
        self.assertEqual(body.count("[🔗]"), 3)             # all three real bullets kept + linked
        dropped = [f for e, f in traces if e == "section_noise_dropped"]
        self.assertEqual(len(dropped), 1, traces)
        self.assertEqual(dropped[0].get("count"), 4)         # 4 stray markers dropped

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
        # The legacy substage name is frozen on purpose: a cache built under it
        # must never be reused after a variant bump, so this literal stays hardcoded.
        self.assertIn("default_ai_substage-000-002.txt", old_path)
        # The new name tracks AI_SUBSTAGE_CACHE_VARIANT — derive it from the constant
        # so a future variant bump (e.g. v3_precheck -> v4) does not re-stale this test.
        self.assertIn(f"{run.AI_SUBSTAGE_CACHE_VARIANT}-000-002.txt", new_path)


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

        # Pin the cross-dedup ensemble to a single sample so this test keeps
        # asserting the substage call *shape* rather than tracking N.
        with patch.object(run, "_run_nullclaw_agent", fake_run_agent), \
             patch.object(run, "_news_cache_get", lambda *args, **kwargs: None), \
             patch.object(run, "_news_cache_put", lambda *args, **kwargs: None), \
             patch.dict(os.environ, {"NEWS_CROSS_DEDUP_N": "1"}, clear=False), \
             patch.object(run, "log_trace", fake_log_trace):
            lines = run._summarize_default_ai_substaged(
                items,
                "2026/05/24 (Sun)",
                run.AlertContext(None, "main", "test"),
            )

        body = "\n".join(lines)
        self.assertEqual(body.count("DeepSeek 降低 API 價格"), 1)
        # 2 Level-2 half substages + 1 cross-half LLM same-event dedup pass
        self.assertEqual(len(calls), 3)
        self.assertEqual(calls[-1][0], "cross_dedup")
        cluster_events = [fields for event, fields in trace_events if event == "cluster_dedup"]
        self.assertEqual(cluster_events, [{
            "before": 4,
            "after": 3,
            "clusters_total": 3,
            "clusters_kept": 3,
        }])


# Real-world tech rewrite fixture (Bloomberg US chip-crisis story + related theme).
# Truth: {1,5} same event; 2/3/4 distinct subjects under the same sector theme.
_TECH_DEDUP_FIXTURE_TITLES = [
    "川普「晶片回流」恐夢碎？專家揭美半導體業最大危機：台積電、美光、三星都難逃",
    "美光、三星領跌 記憶體晶片股集體陷技術性熊市",
    "半導體股震盪何時完結？",
    "晶片股慘遭拋售 輝達選擇權卻爆量押多 甦醒時刻到了？",
    "台積電也難逃！彭博爆美國晶片業「拉警報」 最大危機曝光",
]


class LlmDedupHintsTests(unittest.TestCase):
    """P0 DEDUP_RULES + P1 soft hints + P2 post-select hard dedup (overlap >= 4)."""

    def test_fixture_topic_word_overlaps(self):
        words = [run._topic_words(t) for t in _TECH_DEDUP_FIXTURE_TITLES]
        self.assertEqual(len(words[0] & words[4]), 7)  # 1 vs 5 same event
        self.assertEqual(len(words[0] & words[1]), 3)  # 1 vs 2 theme only
        self.assertEqual(len(words[0] & words[2]), 2)
        self.assertEqual(len(words[1] & words[3]), 2)

    def test_dedup_pair_hints_fixture_only_one_five(self):
        numbered = {
            i + 1: {"title": t, "link": f"http://x/{i + 1}", "source_name": ""}
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES)
        }
        pairs = run._dedup_pair_hints(numbered)
        self.assertEqual(pairs, [(1, 5, 7)])
        # Must not false-hint same-theme pairs at threshold 4.
        pair_set = {(a, b) for a, b, _ in pairs}
        self.assertNotIn((1, 2), pair_set)
        self.assertNotIn((1, 3), pair_set)
        self.assertNotIn((2, 4), pair_set)

    def test_dedup_pair_hints_independent_not_transitive(self):
        # Fully pairwise-high fixture: emits every unordered pair as independent
        # 3-tuples (not a single multi-id cluster object). Exact set locked.
        numbered = {
            1: {"title": "台積電先進製程擴產計畫曝光 美國廠進度 - A"},
            2: {"title": "台積電先進製程擴產計畫再更新 美國廠 - B"},
            3: {"title": "台積電先進製程擴產計畫與美國廠供應鏈 - C"},
        }
        pairs = run._dedup_pair_hints(numbered, min_overlap=4)
        self.assertEqual(pairs, [(1, 2, 12), (1, 3, 12), (2, 3, 12)])
        self.assertTrue(all(len(p) == 3 for p in pairs))
        self.assertTrue(all(a < b for a, b, _ in pairs))
        self.assertEqual(pairs, sorted(pairs, key=lambda x: (x[0], x[1])))
    def test_format_dedup_hint_block(self):
        self.assertEqual(run._format_dedup_hint_block([]), "")
        block = run._format_dedup_hint_block([(1, 5, 7)])
        self.assertIn("#1+#5", block)
        self.assertIn("僅供複核", block)
        self.assertTrue(block.endswith("\n\n"))

    def test_tech_prompt_includes_dedup_rules_and_fixture_hint(self):
        items = [
            _item(t, "src", f"https://example.com/{i}")
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES, start=1)
        ]
        prompts = []

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            prompts.append(prompt)
            # Valid Chinese marked bullets so the section path can finish.
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout="- #1 美半導體危機影響台積電\n- #2 記憶體股技術性熊市\n- #4 輝達選擇權爆量",
                stderr="",
            )

        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "LLM_DEDUP_HINTS_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None), \
             patch.object(run, "_precheck_apply",
                          lambda summary, numbered, section: (summary, {})), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "_attach_numbered_links",
                          lambda summary, numbered, paywall=None: (summary, 1)):
            lines, used_fallback = run._summarize_default_section(
                "tech", items, "2026/07/13 (Mon)", {},
            )

        self.assertFalse(used_fallback)
        self.assertEqual(len(prompts), 1)
        prompt = prompts[0]
        self.assertIn(run.DEDUP_RULES, prompt)
        self.assertIn("#1+#5", prompt)
        self.assertNotIn("#1+#2", prompt)
        self.assertIn("可能同事件候選", prompt)

    def test_ai_substage_prompt_includes_dedup_rules_and_hints(self):
        # Codex S5: AI injects same P1 soft hints when pair overlap >= 4.
        items = [
            _item(_TECH_DEDUP_FIXTURE_TITLES[0], "A", "https://example.com/1"),
            _item(_TECH_DEDUP_FIXTURE_TITLES[4], "B", "https://example.com/5"),
        ]
        prompts = []

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            prompts.append(prompt)
            body = "\n".join(
                f"- #{n} 這是繁體中文標題內容測試用" for n in numbered
            )
            return subprocess.CompletedProcess(
                ["nullclaw"], 0, stdout=body, stderr="",
            )

        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "_news_cache_get", lambda *a, **k: None), \
             patch.object(run, "_news_cache_put", lambda *a, **k: None), \
             patch.object(run, "LLM_DEDUP_HINTS_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None), \
             patch.object(run, "_precheck_apply",
                          lambda summary, numbered, section: (summary, {})), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "_attach_numbered_links",
                          lambda summary, numbered, paywall=None: (summary, 1)):
            ok, lines, err = run._run_ai_substage(items, 0, 2, "2026/07/13 (Mon)")

        self.assertTrue(ok, err)
        self.assertEqual(len(prompts), 1)
        self.assertIn(run.DEDUP_RULES, prompts[0])
        self.assertIn("可能同事件候選", prompts[0])
        self.assertIn("#1+#2", prompts[0])

    def test_kill_switch_disables_hints_and_traces(self):
        items = [
            _item(t, "src", f"https://example.com/{i}")
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES, start=1)
        ]
        prompts = []
        traces = []

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            prompts.append(prompt)
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout="- #1 美半導體危機影響台積電\n- #2 記憶體股技術性熊市",
                stderr="",
            )

        def fake_trace(event, **fields):
            traces.append((event, fields))

        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "LLM_DEDUP_HINTS_ENABLED", False), \
             patch.object(run, "log_trace", fake_trace), \
             patch.object(run, "_precheck_apply",
                          lambda summary, numbered, section: (summary, {})), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "_attach_numbered_links",
                          lambda summary, numbered, paywall=None: (summary, 1)):
            run._summarize_default_section("tech", items, "2026/07/13 (Mon)", {})

        self.assertEqual(len(prompts), 1)
        self.assertNotIn("可能同事件候選", prompts[0])
        self.assertNotIn("#1+#5", prompts[0])
        # Shared rules still present when hints are off.
        self.assertIn(run.DEDUP_RULES, prompts[0])
        hint_traces = [f for e, f in traces if e == "llm_dedup_hints"]
        self.assertEqual(hint_traces, [{
            "section": "tech",
            "enabled": False,
            "pairs": [],
            "min_overlap": 4,
        }])

    def test_hint_trace_when_enabled(self):
        items = [
            _item(t, "src", f"https://example.com/{i}")
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES, start=1)
        ]
        traces = []

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout="- #1 美半導體危機影響台積電",
                stderr="",
            )

        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "LLM_DEDUP_HINTS_ENABLED", True), \
             patch.object(run, "LLM_POST_DEDUP_ENABLED", False), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))), \
             patch.object(run, "_precheck_apply",
                          lambda summary, numbered, section: (summary, {})), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "_attach_numbered_links",
                          lambda summary, numbered, paywall=None: (summary, 1)):
            run._summarize_default_section("tech", items, "2026/07/13 (Mon)", {})

        hint_traces = [f for e, f in traces if e == "llm_dedup_hints"]
        self.assertEqual(len(hint_traces), 1)
        self.assertEqual(hint_traces[0]["section"], "tech")
        self.assertTrue(hint_traces[0]["enabled"])
        self.assertEqual(hint_traces[0]["pairs"], [{"a": 1, "b": 5, "overlap": 7}])

    def test_post_dedup_collapses_one_and_five_keeps_theme_peers(self):
        numbered = {
            i + 1: {"title": t, "link": f"http://x/{i + 1}", "source_name": ""}
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES)
        }
        # LLM still picked both rewrites plus distinct-theme peers.
        summary = (
            "- #1 川普晶片回流危機台積電難逃\n"
            "- #2 記憶體股技術性熊市\n"
            "- #4 輝達選擇權爆量\n"
            "- #5 彭博美晶片業拉警報台積電難逃"
        )
        traces = []
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech",
            )
        ids = run._extract_leading_marker_ids(out, numbered)
        self.assertEqual(ids, [1, 2, 4])  # 5 dropped; 2 and 4 kept
        self.assertNotIn("#5", out)
        post = [f for e, f in traces if e == "llm_post_dedup"]
        self.assertEqual(len(post), 1)
        self.assertEqual(post[0]["before"], [1, 2, 4, 5])
        self.assertEqual(post[0]["after"], [1, 2, 4])
        self.assertEqual(post[0]["dropped"], [5])
        self.assertEqual(post[0]["pairs"], [{"a": 1, "b": 5, "overlap": 7}])
        self.assertEqual(post[0]["min_overlap"], 4)

    def test_post_dedup_does_not_drop_overlap_three_theme_pair(self):
        numbered = {
            i + 1: {"title": t, "link": f"http://x/{i + 1}", "source_name": ""}
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES)
        }
        # Only #1 and #2 selected — overlap 3, below threshold 4.
        summary = "- #1 危機A\n- #2 記憶體熊市"
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech",
            )
        self.assertEqual(run._extract_leading_marker_ids(out, numbered), [1, 2])

    def test_post_dedup_kill_switch(self):
        numbered = {
            i + 1: {"title": t, "link": f"http://x/{i + 1}", "source_name": ""}
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES)
        }
        summary = "- #1 危機A\n- #5 危機B"
        traces = []
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", False), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech",
            )
        self.assertEqual(run._extract_leading_marker_ids(out, numbered), [1, 5])
        post = [f for e, f in traces if e == "llm_post_dedup"]
        self.assertEqual(len(post), 1)
        self.assertFalse(post[0]["enabled"])

    def test_tech_section_post_dedup_runs_before_precheck(self):
        items = [
            _item(t, "src", f"https://example.com/{i}")
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES, start=1)
        ]
        precheck_ids = []

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout=(
                    "- #1 美半導體危機影響台積電美光三星\n"
                    "- #2 記憶體股技術性熊市\n"
                    "- #5 彭博爆美國晶片業拉警報"
                ),
                stderr="",
            )

        def fake_precheck(summary, numbered, section):
            precheck_ids.append(run._extract_leading_marker_ids(summary, numbered))
            return summary, {}

        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "LLM_DEDUP_HINTS_ENABLED", True), \
             patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None), \
             patch.object(run, "_precheck_apply", fake_precheck), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "_attach_numbered_links",
                          lambda summary, numbered, paywall=None: (summary, 1)):
            lines, used_fallback = run._summarize_default_section(
                "tech", items, "2026/07/13 (Mon)", {},
            )

        self.assertFalse(used_fallback)
        # Precheck: #5 collapsed; Codex S3 refill may add never-selected #3
        # to meet pick_min=3 — still no #5.
        self.assertEqual(precheck_ids, [[1, 2, 3]])
        body = "\n".join(lines)
        self.assertIn("#1", body)
        self.assertIn("#2", body)
        self.assertIn("#3", body)
        self.assertNotIn("#5", body)

    def test_ai_substage_post_dedup_before_precheck(self):
        items = [
            _item(_TECH_DEDUP_FIXTURE_TITLES[0], "A", "https://example.com/1"),
            _item(_TECH_DEDUP_FIXTURE_TITLES[4], "B", "https://example.com/5"),
            _item(_TECH_DEDUP_FIXTURE_TITLES[1], "C", "https://example.com/2"),
        ]
        precheck_ids = []

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout=(
                    "- #1 美半導體危機影響台積電\n"
                    "- #2 彭博爆美國晶片業拉警報\n"
                    "- #3 記憶體股技術性熊市"
                ),
                stderr="",
            )

        def fake_precheck(summary, numbered, section):
            precheck_ids.append(run._extract_leading_marker_ids(summary, numbered))
            return summary, {}

        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "_news_cache_get", lambda *a, **k: None), \
             patch.object(run, "_news_cache_put", lambda *a, **k: None), \
             patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None), \
             patch.object(run, "_precheck_apply", fake_precheck), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "_attach_numbered_links",
                          lambda summary, numbered, paywall=None: (summary, 1)):
            ok, lines, err = run._run_ai_substage(items, 0, 3, "2026/07/13 (Mon)")

        self.assertTrue(ok, err)
        self.assertEqual(precheck_ids, [[1, 3]])  # #2 collapsed into #1
        body = "\n".join(lines)
        self.assertIn("#1", body)
        self.assertIn("#3", body)
        self.assertNotIn("#2", body)

    def test_custom_topic_has_dedup_rules_hints_and_post_dedup(self):
        items = [
            _item(t, "src", f"https://example.com/{i}")
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES, start=1)
        ]
        prompts = []
        precheck_ids = []

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            prompts.append(prompt)
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout=(
                    "- #1 美半導體危機影響台積電\n"
                    "- #2 記憶體股技術性熊市\n"
                    "- #5 彭博爆美國晶片業拉警報"
                ),
                stderr="",
            )

        def fake_precheck(summary, numbered, section):
            precheck_ids.append(run._extract_leading_marker_ids(summary, numbered))
            return summary, {}

        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "LLM_DEDUP_HINTS_ENABLED", True), \
             patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None), \
             patch.object(run, "_precheck_apply", fake_precheck), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "_attach_numbered_links",
                          lambda summary, numbered, paywall=None: (summary, 1)), \
             tempfile.TemporaryDirectory() as tmp, \
             patch.object(run, "NEWS_CACHE_DIR", tmp):
            ok, lines, err = run._run_custom_topic(
                "半導體", items, "2026/07/13 (Mon)",
            )
            cache_names = []
            for root, _dirs, files in os.walk(tmp):
                cache_names.extend(files)

        self.assertTrue(ok, err)
        self.assertEqual(len(prompts), 1)
        self.assertIn(run.DEDUP_RULES, prompts[0])
        self.assertIn("#1+#5", prompts[0])
        self.assertEqual(precheck_ids, [[1, 2]])
        body = "\n".join(lines)
        self.assertIn("#1", body)
        self.assertIn("#2", body)
        self.assertNotIn("#5", body)
        self.assertTrue(
            any("custom_topic_v3_dedup" in name for name in cache_names),
            f"cache files={cache_names}",
        )

    def test_general_section_gets_hints_and_dedup_rules(self):
        items = [
            _item(t, "src", f"https://example.com/{i}")
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES, start=1)
        ]
        prompts = []

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            prompts.append(prompt)
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout="- #1 美半導體危機影響台積電\n- #3 半導體股震盪何時完結",
                stderr="",
            )

        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "LLM_DEDUP_HINTS_ENABLED", True), \
             patch.object(run, "LLM_POST_DEDUP_ENABLED", False), \
             patch.object(run, "log_trace", lambda *a, **k: None), \
             patch.object(run, "_precheck_apply",
                          lambda summary, numbered, section: (summary, {})), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "_attach_numbered_links",
                          lambda summary, numbered, paywall=None: (summary, 1)):
            lines, used_fallback = run._summarize_default_section(
                "general", items, "2026/07/13 (Mon)", {},
            )

        self.assertFalse(used_fallback)
        self.assertEqual(len(prompts), 1)
        self.assertIn(run.DEDUP_RULES, prompts[0])
        self.assertIn("#1+#5", prompts[0])

    def test_post_dedup_keeps_first_llm_order_not_lower_id(self):
        # When LLM lists #5 before #1, keep #5 (first-in-output), not min id.
        numbered = {
            i + 1: {"title": t, "link": f"http://x/{i + 1}", "source_name": ""}
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES)
        }
        summary = "- #5 彭博版本先出現\n- #1 川普版本後出現\n- #2 記憶體"
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech",
            )
        self.assertEqual(run._extract_leading_marker_ids(out, numbered), [5, 2])
        self.assertIn("#5", out)
        self.assertNotIn("#1", out)

    def test_post_dedup_noop_single_or_empty(self):
        numbered = {1: {"title": _TECH_DEDUP_FIXTURE_TITLES[0], "link": "http://x/1"}}
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            one = run._post_dedup_selected_summary(
                "- #1 只有一則", numbered, section="tech",
            )
            empty = run._post_dedup_selected_summary(
                "", numbered, section="tech",
            )
        self.assertEqual(run._extract_leading_marker_ids(one, numbered), [1])
        self.assertEqual(empty.strip(), "")

    def test_post_dedup_preserves_non_bullet_lines(self):
        numbered = {
            i + 1: {"title": t, "link": f"http://x/{i + 1}", "source_name": ""}
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES)
        }
        summary = (
            "前言保留\n"
            "- #1 危機A\n"
            "- #5 危機B\n"
            "結語保留"
        )
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech",
            )
        self.assertIn("前言保留", out)
        self.assertIn("結語保留", out)
        self.assertEqual(run._extract_leading_marker_ids(out, numbered), [1])

    def test_cache_variants_bumped_for_dedup(self):
        self.assertEqual(
            run.AI_SUBSTAGE_CACHE_VARIANT,
            "default_ai_clustered_v5_post_dedup",
        )
        self.assertEqual(run.LLM_DEDUP_HINT_OVERLAP, 4)
        self.assertEqual(run.LLM_POST_DEDUP_OVERLAP, 4)
        self.assertEqual(run.LLM_DEDUP_HINT_OVERLAP, run.LLM_POST_DEDUP_OVERLAP)
        # Defaults on when env unset (module load values under normal test env).
        self.assertTrue(run.LLM_DEDUP_HINTS_ENABLED)
        self.assertTrue(run.LLM_POST_DEDUP_ENABLED)
        import inspect
        src = inspect.getsource(run._run_custom_topic)
        self.assertIn("custom_topic_v3_dedup", src)

    def test_post_dedup_no_transitive_bridge_drop(self):
        """A–B and B–C edges must not drop C when A∩C is low (no union-find)."""
        numbered = {
            1: {"title": "蘋果發布新款 iPhone 十七 台灣開賣", "link": "http://x/1"},
            2: {
                "title": "蘋果發布新款 iPhone 十七 與半導體供應鏈震盪綜述",
                "link": "http://x/2",
            },
            3: {"title": "半導體供應鏈震盪綜述 記憶體報價走勢", "link": "http://x/3"},
        }
        # Sanity: bridge B shares with both; A∩C should stay below threshold.
        w = {i: run._topic_words(numbered[i]["title"]) for i in numbered}
        self.assertGreaterEqual(len(w[1] & w[2]), 4)
        self.assertGreaterEqual(len(w[2] & w[3]), 4)
        self.assertLess(len(w[1] & w[3]), 4)
        summary = "- #1 蘋果iphone\n- #2 橋接綜述\n- #3 記憶體報價"
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech",
            )
        # Keep A and C; drop only bridge B (or keep A,C regardless of B fate).
        ids = run._extract_leading_marker_ids(out, numbered)
        self.assertIn(1, ids)
        self.assertIn(3, ids)
        self.assertNotIn(2, ids)

    def test_post_dedup_three_same_event_keeps_first_only(self):
        numbered = {
            1: {"title": "台積電美國廠量產進度延後 專家示警供應鏈", "link": "http://x/1"},
            2: {"title": "台積電美國廠量產進度再延 專家示警供應鏈壓力", "link": "http://x/2"},
            3: {"title": "專家示警：台積電美國廠量產進度延後衝擊供應鏈", "link": "http://x/3"},
        }
        w = {i: run._topic_words(numbered[i]["title"]) for i in numbered}
        self.assertGreaterEqual(len(w[1] & w[2]), 4)
        self.assertGreaterEqual(len(w[1] & w[3]), 4)
        self.assertGreaterEqual(len(w[2] & w[3]), 4)
        summary = "- #2 第二則先出\n- #1 第一則\n- #3 第三則"
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech",
            )
        self.assertEqual(run._extract_leading_marker_ids(out, numbered), [2])

    def test_dedup_pair_hints_skips_empty_or_missing_title(self):
        numbered = {
            1: {"title": "", "link": "http://x/1"},
            2: {"link": "http://x/2"},
            3: {
                "title": "台積電美國廠量產進度延後 專家示警供應鏈",
                "link": "http://x/3",
            },
            4: {
                "title": "台積電美國廠量產進度再延 專家示警供應鏈壓力",
                "link": "http://x/4",
            },
        }
        pairs = run._dedup_pair_hints(numbered, min_overlap=4)
        self.assertEqual([(a, b) for a, b, _ in pairs], [(3, 4)])

    def test_post_dedup_comma_marker_not_treated_as_id(self):
        numbered = {
            i + 1: {"title": t, "link": f"http://x/{i + 1}", "source_name": ""}
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES)
        }
        summary = (
            "- #1, 這不是合法 marker\n"
            "- #1 合法危機A\n"
            "- #5 合法危機B"
        )
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech",
            )
        ids = run._extract_leading_marker_ids(out, numbered)
        self.assertEqual(ids, [1])
        self.assertIn("#1,", out)

    def test_post_dedup_underfill_trace_and_refill(self):
        # Codex S3: underfill then deterministic refill from never-selected ids.
        numbered = {
            i + 1: {"title": t, "link": f"http://x/{i + 1}", "source_name": ""}
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES)
        }
        summary = "- #1 危機A\n- #2 記憶體\n- #5 危機B"
        traces = []
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech", pick_min=3,
            )
        ids = run._extract_leading_marker_ids(out, numbered)
        # #5 dropped; #3 (never selected) refilled to reach pick_min=3.
        self.assertEqual(ids[0], 1)
        self.assertEqual(ids[1], 2)
        self.assertIn(3, ids)
        self.assertNotIn(5, ids)
        self.assertEqual(len(ids), 3)
        under = [f for e, f in traces if e == "post_dedup_underfill"]
        self.assertEqual(len(under), 1)
        self.assertEqual(under[0]["before"], 3)
        self.assertEqual(under[0]["after"], 2)
        self.assertEqual(under[0]["pick_min"], 3)
        refill = [f for e, f in traces if e == "post_dedup_refill"]
        self.assertEqual(len(refill), 1)
        self.assertEqual(refill[0]["added"], [3])
        self.assertEqual(refill[0]["final_count"], 3)
        self.assertFalse(refill[0]["still_underfill"])

    def test_refill_rejects_high_overlap_unselected(self):
        numbered = {
            1: {"title": "台積電美國廠量產進度延後 專家示警供應鏈", "link": "http://x/1"},
            2: {"title": "記憶體股技術性熊市 美光三星領跌", "link": "http://x/2"},
            3: {"title": "台積電美國廠量產進度再延 專家示警供應鏈壓力", "link": "http://x/3"},
        }
        # LLM picked 1+3 (same event) + only those two; pick_min=2 already met after
        # collapse to [1] — underfill needs selected>=pick_min before cut.
        summary = "- #1 版本A\n- #3 版本B"
        traces = []
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech", pick_min=2,
            )
        ids = run._extract_leading_marker_ids(out, numbered)
        # Collapse to #1; underfill; #2 is free theme and should refill.
        self.assertEqual(ids, [1, 2])
        refill = [f for e, f in traces if e == "post_dedup_refill"]
        self.assertEqual(refill[0]["added"], [2])

    def test_parse_pick_min(self):
        self.assertEqual(run._parse_pick_min("3-5"), 3)
        self.assertEqual(run._parse_pick_min("2-3"), 2)
        self.assertEqual(run._parse_pick_min(None), None)
        self.assertEqual(run._parse_pick_min(""), None)

    def test_custom_topic_language_gate_after_refill_english(self):
        """Claude M1: custom path must language-gate refilled raw English titles."""
        items = [
            _item("台積電美國廠量產進度延後 專家示警供應鏈", "A", "https://a/1"),
            _item("台積電美國廠量產進度再延 專家示警供應鏈壓力", "B", "https://a/2"),
            _item("OpenAI launches significantly better model increasingly", "C", "https://a/3"),
        ]
        # LLM picks two same-event Chinese rewrites → P2 keeps one → underfill
        # pick_min=2 → refill never-selected English #3 → language gate must run.

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout="- #1 台積電美國廠延後\n- #2 台積電美國廠再延",
                stderr="",
            )

        translate_called = []

        def fake_translate(key, ids, numbered, date_str, paywall):
            translate_called.append(list(ids))
            return [f"- #{i} 已翻譯標題" for i in ids]

        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "LLM_DEDUP_HINTS_ENABLED", False), \
             patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None), \
             patch.object(run, "_precheck_apply",
                          lambda summary, numbered, section: (summary, {})), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "_translate_selected_section", fake_translate), \
             tempfile.TemporaryDirectory() as tmp, \
             patch.object(run, "NEWS_CACHE_DIR", tmp):
            ok, lines, err = run._run_custom_topic(
                "半導體", items, "2026/07/13 (Mon)",
            )

        self.assertTrue(ok, err)
        self.assertTrue(translate_called, "language gate must invoke translate")
        self.assertTrue(any("已翻譯" in ln for ln in lines))

    def test_duplicate_markers_in_summary_deduped_by_extract(self):
        numbered = {
            i + 1: {"title": t, "link": f"http://x/{i + 1}", "source_name": ""}
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES)
        }
        summary = "- #1 危機A\n- #1 重複同一編號\n- #5 危機B"
        self.assertEqual(
            run._extract_leading_marker_ids(summary, numbered), [1, 5],
        )
        with patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._post_dedup_selected_summary(
                summary, numbered, section="tech",
            )
        self.assertEqual(run._extract_leading_marker_ids(out, numbered), [1])

    def test_tech_language_fail_branch_still_post_dedups_before_precheck(self):
        """Ordering: marker ok → post_dedup → language fail path → precheck."""
        items = [
            _item(t, "src", f"https://example.com/{i}")
            for i, t in enumerate(_TECH_DEDUP_FIXTURE_TITLES, start=1)
        ]
        precheck_ids = []

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            # English-heavy bullets fail language gate; still must post-dedup.
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout=(
                    "- #1 semiconductor crisis hits TSMC significantly\n"
                    "- #5 semiconductor crisis hits TSMC increasingly\n"
                    "- #2 memory stocks technical bear market"
                ),
                stderr="",
            )

        def fake_precheck(summary, numbered, section):
            precheck_ids.append(run._extract_leading_marker_ids(summary, numbered))
            return summary, {}

        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "LLM_DEDUP_HINTS_ENABLED", False), \
             patch.object(run, "LLM_POST_DEDUP_ENABLED", True), \
             patch.object(run, "log_trace", lambda *a, **k: None), \
             patch.object(run, "_precheck_apply", fake_precheck), \
             patch.object(run, "_resolve_paywall_replacements", lambda *a, **k: None), \
             patch.object(run, "_translate_selected_section",
                          lambda *a, **k: ["- 已翻譯"]), \
             patch.object(run, "_attach_numbered_links",
                          lambda summary, numbered, paywall=None: (summary, 1)):
            lines, used_fallback = run._summarize_default_section(
                "tech", items, "2026/07/13 (Mon)", {},
            )

        self.assertFalse(used_fallback)
        # #1 and #5 collapsed before precheck even on language-fail branch;
        # refill may add never-selected #3 for pick_min.
        self.assertEqual(precheck_ids, [[1, 2, 3]])
        self.assertNotIn(5, precheck_ids[0])


class ForbiddenEnglishAdverbTests(unittest.TestCase):
    """Tests for the FORBIDDEN_NON_PROPER_ENGLISH check in language validation."""

    def test_forbidden_adverb_in_bullet_fails_validation(self):
        """A bullet with 'increasingly' (not a proper noun) must fail validation."""
        summary = "- 中國 increasingly 將最優秀的AI人才留在國內"
        self.assertFalse(run._language_validation_passed(summary))

    def test_clean_bullet_with_proper_nouns_passes_validation(self):
        """A clean bullet with only proper nouns (OpenAI, GPT-5) passes validation."""
        summary = "- OpenAI 發布新版 GPT-5"
        self.assertTrue(run._language_validation_passed(summary))

    def test_existing_80_percent_cjk_rule_still_enforced(self):
        """The original 80% CJK-start + ≥2 CJK char rule still applies."""
        # This summary has bullets that don't start with CJK within first 18 chars
        # and would fail the 80% threshold
        summary = "- abcdefghijklmnop English only title without Chinese\n- 這是第二則中文新聞標題"
        # With 2 bullets, need at least 2 to pass 80% (chinese * 5 >= total * 4)
        # Only 1 has CJK starting early enough, so this should fail
        self.assertFalse(run._language_validation_passed(summary))


class SkillWallclockTests(unittest.TestCase):
    """Read-only scheduler wall-clock view for timing traces (Limitation 3).
    Must NOT share code with _llm_retry_budget_secs, whose 2s-reserve retry gate
    stays byte-for-byte."""

    def _clear(self):
        for v in ("NULLCLAW_SKILL_TIMEOUT", "NULLCLAW_SKILL_STARTED"):
            os.environ.pop(v, None)

    def test_wallclock_elapsed_and_remaining_from_started_and_timeout(self):
        with patch.dict(os.environ, {}, clear=False), \
             patch.object(run.time, "monotonic", lambda: 130.0):
            os.environ["NULLCLAW_SKILL_STARTED"] = "100"   # elapsed = 130-100 = 30
            os.environ["NULLCLAW_SKILL_TIMEOUT"] = "90"     # remaining = 90-30 = 60 (NO 2s reserve)
            elapsed, remaining = run._skill_wallclock()
        self.assertAlmostEqual(elapsed, 30.0)
        self.assertAlmostEqual(remaining, 60.0)

    def test_wallclock_none_when_no_env(self):
        with patch.dict(os.environ, {}, clear=False):
            self._clear()
            self.assertEqual(run._skill_wallclock(), (None, None))

    def test_wallclock_remaining_none_without_started(self):
        # timeout alone can't give proximity-to-kill: elapsed is unknown -> don't guess
        with patch.dict(os.environ, {}, clear=False):
            self._clear()
            os.environ["NULLCLAW_SKILL_TIMEOUT"] = "90"
            self.assertEqual(run._skill_wallclock(), (None, None))

    def test_wallclock_elapsed_only_without_timeout(self):
        with patch.dict(os.environ, {}, clear=False), \
             patch.object(run.time, "monotonic", lambda: 150.0):
            self._clear()
            os.environ["NULLCLAW_SKILL_STARTED"] = "100"
            elapsed, remaining = run._skill_wallclock()
        self.assertAlmostEqual(elapsed, 50.0)
        self.assertIsNone(remaining)

    def test_llm_retry_budget_secs_unchanged_golden(self):
        # Regression guard: the retry gate keeps its 2s reserve and its
        # started-absent / no-timeout semantics. If a refactor ever touched it,
        # these goldens break.
        with patch.dict(os.environ, {}, clear=False):
            self._clear()
            self.assertIsNone(run._llm_retry_budget_secs())          # no timeout -> None
            os.environ["NULLCLAW_SKILL_TIMEOUT"] = "5"
            self.assertAlmostEqual(run._llm_retry_budget_secs(), 3.0)  # 5 - 2 reserve, no started
        with patch.dict(os.environ, {}, clear=False), \
             patch.object(run.time, "monotonic", lambda: 130.0):
            self._clear()
            os.environ["NULLCLAW_SKILL_TIMEOUT"] = "90"
            os.environ["NULLCLAW_SKILL_STARTED"] = "100"             # elapsed 30
            self.assertAlmostEqual(run._llm_retry_budget_secs(), 58.0)  # 90 - 30 - 2


class NewsTimingTests(unittest.TestCase):
    """Limitation 3: section + whole-run wall-clock traces. The whole-run trace
    MUST fire even on the exit-1 paths (that is where the kill-window pressure is),
    so it lives in a finally with an outcome field."""

    def _ctx(self):
        return run.AlertContext(deliver_to=None, account="main", job_id="interactive")

    def test_summarize_llm_emits_section_timing_each_section(self):
        traces = []
        with patch.object(run, "_summarize_default_ai_substaged",
                          lambda items, date_str, ctx: ["- 標題A"]), \
             patch.object(run, "_summarize_default_section",
                          lambda key, items, date_str, link_map: (["- 標題" + key], False)), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            run.summarize_llm(
                {"ai": [{"title": "a", "link": "http://a"}],
                 "tech": [{"title": "b", "link": "http://b"}],
                 "general": [{"title": "c", "link": "http://c"}]}, self._ctx())
        st = [f for e, f in traces if e == "section_timing"]
        self.assertEqual(sorted(f["section"] for f in st), ["ai", "general", "tech"])
        for f in st:
            self.assertIsInstance(f["elapsed_ms"], int)

    def test_summarize_llm_section_timing_fires_on_ai_exhausted(self):
        def boom(items, date_str, ctx):
            raise run._AiSubstageExhausted("x")
        traces = []
        with patch.object(run, "_summarize_default_ai_substaged", boom), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            with self.assertRaises(run._AiSubstageExhausted):
                run.summarize_llm({"ai": [{"title": "a", "link": "http://a"}]}, self._ctx())
        st = [f for e, f in traces if e == "section_timing" and f["section"] == "ai"]
        self.assertEqual(len(st), 1)   # finally fired even though the section raised

    def _main_patches(self, traces, summarize):
        return [
            patch("sys.argv", ["run.py", "--deliver-to", "chat"]),
            patch.object(run, "load_env", lambda *a, **k: None),
            patch.object(run, "_news_cache_sweep", lambda *a, **k: None),
            patch.object(run, "_resolve_topics", lambda args: []),
            patch.object(run, "fetch_feed", lambda *a, **k: [{"title": "t", "link": "http://x"}]),
            patch.object(run, "summarize_llm", summarize),
            patch.object(run, "_deliver_news_or_fail", lambda *a, **k: None),
            patch.object(run, "_alert_failure", lambda *a, **k: None),
            patch.object(run, "emit_skill_status", lambda *a, **k: None),
            patch.object(run, "emit_trace", lambda *a, **k: None),
            patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))),
        ]

    def test_main_emits_run_timing_on_success(self):
        import contextlib
        traces = []
        with contextlib.ExitStack() as es:
            for p in self._main_patches(traces, lambda all_items, ctx: "digest"):
                es.enter_context(p)
            run.main()
        rt = [f for e, f in traces if e == "news_run_timing"]
        self.assertEqual(len(rt), 1, traces)
        self.assertEqual(rt[0]["outcome"], "ok")
        for field in ("total_ms", "feeds_ms", "summarize_ms", "deliver_ms"):
            self.assertIsInstance(rt[0][field], int, field)

    def test_main_emits_run_timing_on_ai_exhausted_exit1(self):
        import contextlib
        traces = []

        def boom(all_items, ctx):
            raise run._AiSubstageExhausted("x")

        with contextlib.ExitStack() as es:
            for p in self._main_patches(traces, boom):
                es.enter_context(p)
            with self.assertRaises(SystemExit) as cm:
                run.main()
        self.assertEqual(cm.exception.code, 1)          # exit-1 contract preserved
        rt = [f for e, f in traces if e == "news_run_timing"]
        self.assertEqual(len(rt), 1, traces)            # finally still fired on the exit path
        self.assertEqual(rt[0]["outcome"], "ai_exhausted")

    def test_main_run_timing_feeds_empty_keeps_outcome(self):
        # feeds_empty sets outcome then sys.exit(1) -> caught by except SystemExit,
        # whose `if outcome == "ok"` guard must NOT relabel it delivery_exit.
        import contextlib
        traces = []
        with contextlib.ExitStack() as es:
            for p in self._main_patches(traces, lambda all_items, ctx: "digest"):
                es.enter_context(p)
            es.enter_context(patch.object(run, "fetch_feed", lambda *a, **k: []))  # empty -> outage
            with self.assertRaises(SystemExit) as cm:
                run.main()
        self.assertEqual(cm.exception.code, 1)
        rt = [f for e, f in traces if e == "news_run_timing"]
        self.assertEqual(len(rt), 1, traces)
        self.assertEqual(rt[0]["outcome"], "feeds_empty")   # guard kept it, not delivery_exit

    def test_main_feeds_empty_no_false_telegram_alert(self):
        # feeds_empty already fires "all_feeds_empty"; its sys.exit(1) is caught by
        # `except SystemExit`, which must NOT also fire the misleading
        # "telegram_delivery_failed" alert — that path never touched delivery.
        import contextlib
        alerts = []
        with contextlib.ExitStack() as es:
            es.enter_context(patch("sys.argv", ["run.py", "--deliver-to", "chat"]))
            es.enter_context(patch.object(run, "load_env", lambda *a, **k: None))
            es.enter_context(patch.object(run, "_news_cache_sweep", lambda *a, **k: None))
            es.enter_context(patch.object(run, "_resolve_topics", lambda args: []))
            es.enter_context(patch.object(run, "fetch_feed", lambda *a, **k: []))  # every feed empty
            es.enter_context(patch.object(run, "_alert_failure",
                                          lambda ctx, key, msg: alerts.append(key)))
            es.enter_context(patch.object(run, "emit_skill_status", lambda *a, **k: None))
            es.enter_context(patch.object(run, "emit_trace", lambda *a, **k: None))
            es.enter_context(patch.object(run, "log_trace", lambda *a, **k: None))
            with self.assertRaises(SystemExit) as cm:
                run.main()
        self.assertEqual(cm.exception.code, 1)
        self.assertIn("all_feeds_empty", alerts)               # the correct alert fired
        self.assertNotIn("telegram_delivery_failed", alerts)   # the misleading one did NOT


class LLMRetryOnTimeoutTests(unittest.TestCase):
    """Change 1: _run_nullclaw_agent retries ONCE on rc=124, with a budget guard."""

    def _cp(self, rc, stdout=""):
        return subprocess.CompletedProcess(["nullclaw"], rc, stdout=stdout, stderr="")

    def test_timeout_then_success_retries_and_returns_ok(self):
        calls = []

        def fake_once(prompt, timeout_secs, variant, all_items, numbered):
            calls.append(timeout_secs)
            return self._cp(124) if len(calls) == 1 else self._cp(0, stdout="- #1 ok")

        with patch.object(run, "_run_nullclaw_agent_once", fake_once), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            # ensure no cron budget set so retry is always permitted
            for var in ("NULLCLAW_SKILL_TIMEOUT", "NULLCLAW_SKILL_STARTED"):
                run.os.environ.pop(var, None)
            res = run._run_nullclaw_agent("p", 60, "v", {}, {})

        self.assertEqual(res.returncode, 0)
        self.assertEqual(len(calls), 2, "should attempt exactly twice")
        self.assertEqual(calls[0], 60, "first attempt uses full timeout")
        self.assertEqual(calls[1], run.LLM_RETRY_TIMEOUT_SECS, "retry uses shorter timeout")

    def test_nonzero_nontimeout_is_not_retried(self):
        calls = []

        def fake_once(prompt, timeout_secs, variant, all_items, numbered):
            calls.append(timeout_secs)
            return self._cp(1)  # generic failure, deterministic — no retry

        with patch.object(run, "_run_nullclaw_agent_once", fake_once), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            res = run._run_nullclaw_agent("p", 60, "v", {}, {})

        self.assertEqual(res.returncode, 1)
        self.assertEqual(len(calls), 1, "non-124 exit must not retry")

    def test_success_first_try_does_not_retry(self):
        calls = []

        def fake_once(prompt, timeout_secs, variant, all_items, numbered):
            calls.append(timeout_secs)
            return self._cp(0, stdout="- #1 ok")

        with patch.object(run, "_run_nullclaw_agent_once", fake_once), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            res = run._run_nullclaw_agent("p", 60, "v", {}, {})

        self.assertEqual(res.returncode, 0)
        self.assertEqual(len(calls), 1)

    def test_retry_skipped_when_budget_too_small(self):
        calls = []

        def fake_once(prompt, timeout_secs, variant, all_items, numbered):
            calls.append(timeout_secs)
            return self._cp(124)

        traces = []
        with patch.object(run, "_run_nullclaw_agent_once", fake_once), \
             patch.object(run, "log_trace", lambda ev, **k: traces.append(ev)):
            # only 5s of wall-clock left, retry wants 30s -> skip
            run.os.environ["NULLCLAW_SKILL_TIMEOUT"] = "5"
            run.os.environ.pop("NULLCLAW_SKILL_STARTED", None)
            try:
                res = run._run_nullclaw_agent("p", 60, "v", {}, {})
            finally:
                run.os.environ.pop("NULLCLAW_SKILL_TIMEOUT", None)

        self.assertEqual(res.returncode, 124)
        self.assertEqual(len(calls), 1, "retry must be skipped under tight budget")
        self.assertIn("llm_agent_retry_skipped_budget", traces)


class RecentFailureCountTests(unittest.TestCase):
    """Change 2: _recent_failure_count surfaces chronic-alert trends."""

    def _write_log(self, blocks):
        f = tempfile.NamedTemporaryFile("w", suffix=".log", delete=False, encoding="utf-8")
        f.write("".join(blocks))
        f.close()
        return f.name

    def _block(self, ts, reason, account):
        return (
            f"=== {ts} CST ===\n"
            f"job_id: x\ndeliver_to: y\naccount: {account}\n"
            f"reason: {reason}\ndetail: whatever\n\n"
        )

    def test_counts_matching_reason_and_account_in_window(self):
        from datetime import datetime, timezone, timedelta
        now = datetime.now(timezone(timedelta(hours=8)))
        recent = (now - timedelta(days=3)).strftime("%Y-%m-%d %H:%M:%S")
        old = (now - timedelta(days=40)).strftime("%Y-%m-%d %H:%M:%S")
        log = self._write_log([
            self._block(recent, "custom_topics_fell_back", "nunu"),
            self._block(recent, "custom_topics_fell_back", "nunu"),
            self._block(old, "custom_topics_fell_back", "nunu"),          # outside 30d
            self._block(recent, "custom_topics_fell_back", "main"),        # other account
            self._block(recent, "telegram_delivery_failed", "nunu"),       # other reason
        ])
        with patch.object(run, "NEWS_FAILURE_LOG", log):
            n = run._recent_failure_count("custom_topics_fell_back", "nunu", days=30)
        self.assertEqual(n, 2)

    def test_missing_log_returns_zero(self):
        with patch.object(run, "NEWS_FAILURE_LOG", "/nonexistent/path.log"):
            self.assertEqual(run._recent_failure_count("r", "a", days=30), 0)


class PaywallReplacementTests(unittest.TestCase):
    def test_attach_numbered_links_no_paywall_unchanged(self):
        # Default path must be byte-identical to before this feature.
        summary = "- #1 一般新聞標題"
        numbered = {1: {"title": "一般新聞標題", "link": "http://x/1"}}
        body, attached = run._attach_numbered_links(summary, numbered)
        self.assertEqual(attached, 1)
        self.assertEqual(body, "- 一般新聞標題 [🔗](http://x/1)")
        self.assertNotIn(run.PAYWALL_NOTE, body)

    def test_attach_numbered_links_paywall_no_replacement_single_bullet_note(self):
        summary = "- #1 付費牆新聞"
        numbered = {1: {"title": "付費牆新聞", "link": "http://nyt/1", "source_name": "NYT"}}
        paywall = {1: {"decoded_url": "http://nyt/1", "reason": "paywalled",
                       "title": "付費牆新聞", "source_name": "NYT"}}
        body, attached = run._attach_numbered_links(summary, numbered, paywall)
        # One physical bullet line, plus the note; no continuation line.
        self.assertEqual(attached, 1)
        self.assertIn(run.PAYWALL_NOTE, body)
        self.assertNotIn(run.PAYWALL_CONT_PREFIX, body)
        self.assertEqual(len(body.splitlines()), 1)

    def test_attach_numbered_links_renders_replacement_double_bullet(self):
        summary = "- #1 付費牆原標題"
        numbered = {1: {"title": "付費牆原標題", "link": "http://nyt/1", "source_name": "NYT"}}
        paywall = {1: {"decoded_url": "http://nyt/1", "reason": "paywalled",
                       "title": "付費牆原標題", "source_name": "NYT",
                       "replacement": {"title_zh": "免費替代標題", "link": "http://free/1"}}}
        body, attached = run._attach_numbered_links(summary, numbered, paywall)
        first, second = body.splitlines()
        # Replacement on top with its own link; original continues below with note.
        self.assertEqual(first, "- 免費替代標題 [🔗](http://free/1)")
        self.assertTrue(second.startswith(run.PAYWALL_CONT_PREFIX))
        self.assertIn("付費牆原標題", second)
        self.assertIn("http://nyt/1", second)
        self.assertIn(run.PAYWALL_NOTE, second)
        # Two links attached (replacement + original).
        self.assertEqual(attached, 2)

    def test_precheck_apply_returns_paywall_map(self):
        summary = "- #1 付費牆標題"
        numbered = {1: {"title": "付費牆標題", "link": "http://nyt/1", "source_name": "NYT"}}
        fake = {"action": "title_only", "reason": "paywalled", "decoded_url": "http://nyt/1"}
        with patch.object(run.news_quality, "precheck_action", return_value=fake):
            out_summary, paywall = run._precheck_apply(summary, numbered, "ai")
        # title_only is never dropped — the bullet survives.
        self.assertIn("#1", out_summary)
        self.assertIn(1, paywall)
        self.assertEqual(paywall[1]["decoded_url"], "http://nyt/1")
        self.assertEqual(paywall[1]["title"], "付費牆標題")

    def test_resolve_paywall_replacements_swallows_exception(self):
        # A raising lookup must NOT propagate (exit-0 contract) and must leave
        # the entry without a replacement so render degrades to note-only.
        paywall = {1: {"title": "x", "decoded_url": "http://nyt/1"}}
        with patch.object(run, "fetch_feed", side_effect=RuntimeError("boom")):
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertNotIn("replacement", paywall[1])

    def test_resolve_paywall_replacements_disabled_is_noop(self):
        paywall = {1: {"title": "x", "decoded_url": "http://nyt/1"}}
        with patch.object(run, "PAYWALL_REPLACE_ENABLED", False), \
             patch.object(run, "fetch_feed") as ff:
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        ff.assert_not_called()
        self.assertNotIn("replacement", paywall[1])

    def test_resolve_paywall_replacements_finds_free_alternative(self):
        paywall = {1: {"title": "全球最強 AI 面臨謎團 - The New York Times",
                       "decoded_url": "https://www.nytimes.com/x"}}
        cands = [{"title": "全球最強 AI 面臨謎團 - TechNews", "link": "http://free/1",
                  "source_name": "TechNews"}]
        free_verdict = {"action": "keep", "reason": None, "decoded_url": "https://technews.tw/x"}
        with patch.object(run, "fetch_feed", return_value=cands), \
             patch.object(run.news_quality, "precheck_action", return_value=free_verdict), \
             patch.object(run, "_translate_single_title", return_value="全球最強 AI 面臨謎團"):
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertIn("replacement", paywall[1])
        self.assertEqual(paywall[1]["replacement"]["link"], "https://technews.tw/x")

    def test_resolve_skips_same_host_candidate(self):
        # A free-looking candidate on a same-publisher SUBDOMAIN is not a real
        # alternative (suffix-aware: cn.nytimes.com == www.nytimes.com publisher).
        paywall = {1: {"title": "AI 謎團 續報 謎團 - NYT",
                       "decoded_url": "https://www.nytimes.com/x"}}
        cands = [{"title": "AI 謎團 續報 謎團 - NYT", "link": "http://nyt/2", "source_name": "NYT"}]
        same_host = {"action": "keep", "reason": None, "decoded_url": "https://cn.nytimes.com/y"}
        with patch.object(run, "fetch_feed", return_value=cands), \
             patch.object(run.news_quality, "precheck_action", return_value=same_host), \
             patch.object(run, "_translate_single_title", return_value="AI 謎團 續報"):
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertNotIn("replacement", paywall[1])

    def test_resolve_skips_candidate_with_unresolved_host(self):
        # A candidate whose publisher host can't be resolved is skipped (can't
        # confirm it's a DIFFERENT publisher than the paywalled source).
        paywall = {1: {"title": "AI 謎團 全球 最強 - NYT",
                       "decoded_url": "https://www.nytimes.com/x"}}
        cands = [{"title": "AI 謎團 全球 最強 - Blog", "link": "http://free/1", "source_name": "Blog"}]
        unresolved = {"action": "keep", "reason": None, "decoded_url": None}
        with patch.object(run, "fetch_feed", return_value=cands), \
             patch.object(run.news_quality, "precheck_action", return_value=unresolved), \
             patch.object(run, "_translate_single_title", return_value="AI 謎團 全球 最強"):
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertNotIn("replacement", paywall[1])

    def test_translate_single_title_uses_placeholder_link(self):
        # Regression: the temp item must carry a non-empty link so
        # _translate_selected_section reports success; the placeholder is
        # stripped so the returned title has no link/marker.
        with patch.object(run, "_run_nullclaw_agent") as agent:
            agent.return_value = __import__("subprocess").CompletedProcess(
                ["nullclaw"], 0, stdout="- #1 免費中文標題", stderr="")
            out = run._translate_single_title("Free English Headline - TechNews",
                                              "2026/07/02 (Wed)")
        self.assertEqual(out, "免費中文標題")
        self.assertNotIn("🔗", out or "")
        self.assertFalse((out or "").startswith("-"))

    def test_resolve_end_to_end_produces_double_bullet_replacement(self):
        # Full resolver with a real (mocked-agent) translate — proves the
        # replacement path is NOT dead: a free different-host candidate yields a
        # replacement dict that the renderer turns into a double bullet.
        paywall = {1: {"title": "全球 最強 AI 面臨 謎團 - The New York Times",
                       "decoded_url": "https://www.nytimes.com/x",
                       "source_name": "The New York Times"}}
        cands = [{"title": "全球 最強 AI 面臨 謎團 - TechNews", "link": "http://free/1",
                  "source_name": "TechNews"}]
        free = {"action": "keep", "reason": None, "decoded_url": "https://technews.tw/x"}
        with patch.object(run, "fetch_feed", return_value=cands), \
             patch.object(run.news_quality, "precheck_action", return_value=free), \
             patch.object(run, "_run_nullclaw_agent") as agent:
            agent.return_value = __import__("subprocess").CompletedProcess(
                ["nullclaw"], 0, stdout="- #1 全球最強AI面臨謎團", stderr="")
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertIn("replacement", paywall[1])
        self.assertEqual(paywall[1]["replacement"]["link"], "https://technews.tw/x")
        # And the renderer turns it into a double bullet.
        summary = "- #1 原付費標題"
        numbered = {1: {"title": "原付費標題", "link": "http://nyt/1"}}
        body, _ = run._attach_numbered_links(summary, numbered, paywall)
        self.assertEqual(len(body.splitlines()), 2)

    def test_over_limit_replacement_bullet_keeps_continuation(self):
        # If the replacement bullet alone exceeds the chunk limit, its
        # continuation must still land in the SAME chunk (not orphaned).
        long_title = "免" * 500
        pair = (
            f"- {long_title} [🔗](http://free/1)\n"
            f"{run.PAYWALL_CONT_PREFIX}原文：付費原標題 [🔗](http://nyt/1)  {run.PAYWALL_NOTE}"
        )
        chunks = run._split_message_preserving_lines(pair, limit=200)
        # The continuation must ride in the chunk that still holds the tail of the
        # replacement bullet — never a standalone chunk with only the note.
        cont_chunks = [c for c in chunks if run.PAYWALL_CONT_PREFIX in c]
        self.assertEqual(len(cont_chunks), 1)
        before_note = cont_chunks[0].split(run.PAYWALL_CONT_PREFIX)[0]
        self.assertIn("免", before_note,
                      "continuation was orphaned from its replacement bullet's tail")

    def test_paywall_footer_counts_stories_not_bullets(self):
        # Two paywalled stories (one with replacement double-bullet, one degraded)
        # → footer says 2, not 3 (the replacement double-bullet must not inflate).
        digest = (
            "📰 摘要\n"
            "- 免費替代 [🔗](http://free/1)\n"
            f"{run.PAYWALL_CONT_PREFIX}原文：付費A [🔗](http://nyt/1)  {run.PAYWALL_NOTE}\n"
            f"- 付費B [🔗](http://wsj/1)  {run.PAYWALL_NOTE}\n"
        )
        self.assertEqual(digest.count(run.PAYWALL_NOTE), 2)

    def test_paywall_pair_not_orphaned_by_trim(self):
        # _trim_lines_to_limit must drop the replacement bullet AND its
        # continuation together — never leave a headless 原文 note.
        head = "📰 摘要\n" + "\n".join(f"- 填充新聞{i} [🔗](http://x/{i})" for i in range(60))
        pair = (
            "- 免費替代標題 [🔗](http://free/1)\n"
            f"{run.PAYWALL_CONT_PREFIX}原文：付費原標題 [🔗](http://nyt/1)  {run.PAYWALL_NOTE}"
        )
        text = head + "\n" + pair
        trimmed = run._trim_lines_to_limit(text, limit=400)
        # If the replacement bullet was trimmed, its continuation must be gone too.
        if "免費替代標題" not in trimmed:
            self.assertNotIn(run.PAYWALL_CONT_PREFIX, trimmed)

    def test_paywall_pair_not_split_across_chunk(self):
        # _split_message_preserving_lines must keep a pair in one chunk.
        filler = "\n".join(f"- 新聞{i} [🔗](http://x/{i})" for i in range(40))
        pair = (
            "- 免費替代標題 [🔗](http://free/1)\n"
            f"{run.PAYWALL_CONT_PREFIX}原文：付費原標題 [🔗](http://nyt/1)  {run.PAYWALL_NOTE}"
        )
        body = filler + "\n" + pair
        chunks = run._split_message_preserving_lines(body, limit=300)
        for ch in chunks:
            if run.PAYWALL_CONT_PREFIX in ch:
                self.assertIn("免費替代標題", ch,
                              "continuation line was split from its replacement bullet")

    def test_paywall_digest_end_to_end_degrades_to_note(self):
        # Full section path: LLM picks a paywalled item, precheck flags it
        # title_only, replacement lookup returns nothing → digest keeps the
        # original bullet + 付費牆 note, and NO failure alert fires.
        items = [{"title": "全球最強 AI 面臨謎團 - The New York Times",
                  "link": "http://nyt/rss/1", "pub_date": "", "source_name": "The New York Times"}]

        def fake_run_agent(prompt, timeout_secs, variant, all_items, numbered):
            return subprocess.CompletedProcess(
                ["nullclaw"], 0,
                stdout="- #1 全球最強 AI 面臨謎團", stderr="")

        title_only = {"action": "title_only", "reason": "paywalled",
                      "decoded_url": "https://www.nytimes.com/x"}
        with patch.object(run, "_run_nullclaw_agent", fake_run_agent), \
             patch.object(run.news_quality, "precheck_action", return_value=title_only), \
             patch.object(run, "fetch_feed", return_value=[]), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            lines, used_fallback = run._summarize_default_section(
                "ai", items, "2026/07/02 (Wed)", {items[0]["title"]: "http://nyt/rss/1"})

        joined = "\n".join(lines)
        self.assertFalse(used_fallback)  # not a failure — news went out
        self.assertIn(run.PAYWALL_NOTE, joined)
        self.assertNotIn(run.PAYWALL_CONT_PREFIX, joined)  # degraded: single bullet
        self.assertNotIn("新聞無法送出", joined)

    def test_normalize_replacement_candidate_unwraps_bing_redirect(self):
        # Bing apiclick links must resolve to the publisher URL before precheck.
        item = {"title": "AI story - Reuters",
                "link": "https://www.bing.com/news/apiclick.aspx?ref=FexRss&url=https%3A%2F%2Fwww.reuters.com%2Ftechnology%2Fai-story%2F",
                "source_name": "Reuters"}
        out = run._normalize_replacement_candidate(item)
        self.assertEqual(out["link"], "https://www.reuters.com/technology/ai-story/")
        self.assertEqual(out["decoded_url"], "https://www.reuters.com/technology/ai-story/")

    def test_normalize_replacement_candidate_missing_url_param_left_unresolved(self):
        # Malformed Bing apiclick links are left untouched and do not raise.
        item = {"title": "AI story - Reuters",
                "link": "https://www.bing.com/news/apiclick.aspx?ref=FexRss&aid=1",
                "source_name": "Reuters"}
        out = run._normalize_replacement_candidate(item)
        self.assertEqual(out, item)
        self.assertNotIn("decoded_url", out)

    def test_resolve_paywall_replacements_merges_google_and_bing_sources(self):
        # Resolver consults both Google and Bing replacement feeds.
        paywall = {1: {"title": "Global AI policy breakthrough shifts markets - NYT",
                       "decoded_url": "https://www.nytimes.com/ai-policy"}}
        google_url = "https://google.invalid/rss"
        bing_url = "https://bing.invalid/rss"
        calls = []

        def fake_fetch(url, max_items=15, timeout=15):
            calls.append(url)
            if url == google_url:
                return [{"title": "Local sports championship result - Example",
                         "link": "https://example.com/sports",
                         "source_name": "Example"}]
            if url == bing_url:
                return [{"title": "Global AI policy breakthrough shifts markets - Reuters",
                         "link": "https://reuters.com/ai-policy",
                         "source_name": "Reuters"}]
            self.fail(f"unexpected feed URL: {url}")

        free = {"action": "keep", "reason": None,
                "decoded_url": "https://www.reuters.com/ai-policy"}
        with patch.object(run, "_topic_feed_url", return_value=google_url), \
             patch.object(run, "_bing_news_feed_url", return_value=bing_url), \
             patch.object(run, "fetch_feed", side_effect=fake_fetch), \
             patch.object(run.news_quality, "precheck_action", return_value=free), \
             patch.object(run, "_translate_single_title", return_value="AI 政策突破"):
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertCountEqual(calls, [google_url, bing_url])
        self.assertEqual(paywall[1]["replacement"]["link"], "https://www.reuters.com/ai-policy")

    def test_resolve_paywall_replacements_dedupes_exact_title_across_sources(self):
        # Exact duplicate titles from Google and Bing are prechecked once.
        paywall = {1: {"title": "Global AI policy breakthrough shifts markets - NYT",
                       "decoded_url": "https://www.nytimes.com/ai-policy"}}
        google_url = "https://google.invalid/rss"
        bing_url = "https://bing.invalid/rss"
        title = "Global AI policy breakthrough shifts markets - Reuters"

        def fake_fetch(url, max_items=15, timeout=15):
            return [{"title": title, "link": f"https://{url.split('//', 1)[1]}/story",
                     "source_name": "Reuters"}]

        unresolved = {"action": "keep", "reason": "unresolved", "decoded_url": None}
        with patch.object(run, "_topic_feed_url", return_value=google_url), \
             patch.object(run, "_bing_news_feed_url", return_value=bing_url), \
             patch.object(run, "fetch_feed", side_effect=fake_fetch), \
             patch.object(run.news_quality, "precheck_action", return_value=unresolved) as pc:
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertEqual(pc.call_count, 1)
        self.assertEqual(pc.call_args.args[0]["title"], title)
        self.assertNotIn("replacement", paywall[1])

    def test_resolve_paywall_replacements_both_sources_empty_degrades_cleanly(self):
        # Empty Google and Bing feeds leave the paywall entry note-only.
        paywall = {1: {"title": "Global AI policy breakthrough shifts markets - NYT",
                       "decoded_url": "https://www.nytimes.com/ai-policy"}}
        with patch.object(run, "_topic_feed_url", return_value="https://google.invalid/rss"), \
             patch.object(run, "_bing_news_feed_url", return_value="https://bing.invalid/rss"), \
             patch.object(run, "fetch_feed", return_value=[]) as ff:
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertEqual(ff.call_count, 2)
        self.assertNotIn("replacement", paywall[1])

    def test_resolve_paywall_replacements_near_duplicate_titles_both_survive(self):
        # Near-duplicate but non-exact titles both reach the existing overlap check.
        paywall = {1: {"title": "Global AI policy breakthrough shifts markets - NYT",
                       "decoded_url": "https://www.nytimes.com/ai-policy"}}
        google_title = "Global AI policy breakthrough shifts markets - Reuters"
        bing_title = "Global AI policy breakthrough shift markets - Reuters"

        def fake_fetch(url, max_items=15, timeout=15):
            if "google" in url:
                return [{"title": google_title, "link": "https://reuters.com/a",
                         "source_name": "Reuters"}]
            return [{"title": bing_title, "link": "https://reuters.com/b",
                     "source_name": "Reuters"}]

        unresolved = {"action": "keep", "reason": "unresolved", "decoded_url": None}
        with patch.object(run, "_topic_feed_url", return_value="https://google.invalid/rss"), \
             patch.object(run, "_bing_news_feed_url", return_value="https://bing.invalid/rss"), \
             patch.object(run, "fetch_feed", side_effect=fake_fetch), \
             patch.object(run.news_quality, "precheck_action", return_value=unresolved) as pc:
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertEqual([c.args[0]["title"] for c in pc.call_args_list],
                         [google_title, bing_title])
        self.assertNotIn("replacement", paywall[1])

    def test_fetch_feed_receives_shrinking_timeout_across_two_calls(self):
        # Bing fetch timeout is bounded by remaining resolver deadline budget.
        paywall = {1: {"title": "Global AI policy breakthrough shifts markets - NYT",
                       "decoded_url": "https://www.nytimes.com/ai-policy"}}
        now = {"value": 100.0}
        timeouts = []

        def fake_fetch(url, max_items=15, timeout=15):
            timeouts.append(timeout)
            if len(timeouts) == 1:
                now["value"] = 104.0
            return []

        with patch.object(run, "_topic_feed_url", return_value="https://google.invalid/rss"), \
             patch.object(run, "_bing_news_feed_url", return_value="https://bing.invalid/rss"), \
             patch.object(run, "PAYWALL_REPLACE_DEADLINE", 10.0), \
             patch.object(run.time, "monotonic", side_effect=lambda: now["value"]), \
             patch.object(run, "fetch_feed", side_effect=fake_fetch):
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertEqual(len(timeouts), 2)
        self.assertLess(timeouts[1], timeouts[0])
        self.assertLessEqual(timeouts[1], 6.0)
        self.assertNotEqual(timeouts[1], 15)

    def test_replacement_link_prefers_decoded_url_over_raw_candidate_link(self):
        # Replacement link uses verdict decoded_url, not the raw RSS candidate link.
        paywall = {1: {"title": "Global AI policy breakthrough shifts markets - NYT",
                       "decoded_url": "https://www.nytimes.com/ai-policy"}}
        raw_link = "https://www.bing.com/news/apiclick.aspx?url=https%3A%2F%2Fbing.invalid%2Fraw"
        cands = [{"title": "Global AI policy breakthrough shifts markets - Reuters",
                  "link": raw_link, "source_name": "Reuters"}]
        free = {"action": "keep", "reason": None,
                "decoded_url": "https://www.reuters.com/ai-policy"}
        with patch.object(run, "fetch_feed", return_value=cands), \
             patch.object(run.news_quality, "precheck_action", return_value=free), \
             patch.object(run, "_translate_single_title", return_value="AI 政策突破"):
            run._resolve_paywall_replacements(paywall, "2026/07/02 (Wed)")
        self.assertEqual(paywall[1]["replacement"]["link"], "https://www.reuters.com/ai-policy")

    def test_parse_ai_blocks_normal_paywall_and_replacement(self):
        import subprocess  # noqa
        lines = [
            "- 全新 AI 模型以影像思考 [🔗](https://news.google.com/rss/articles/AAA?oc=5)",
            "- 祖克柏豪賭 AI：單座資料中心上看 2,500 億美元 [🔗](https://news.google.com/rss/articles/BBB?oc=5)  ⚠️ 付費牆（原文需訂閱）",
            "- Meta 路易斯安那資料中心 [🔗](https://thenextweb.com/x)",
            "　↳ 原文：Meta 路易斯安那 Hyperion 資料中心 [🔗](https://news.google.com/rss/articles/CCC?oc=5)  ⚠️ 付費牆（原文需訂閱）",
        ]
        blocks = run._parse_ai_blocks(lines)
        self.assertEqual(len(blocks), 3)
        self.assertEqual(blocks[0]["headline"], "全新 AI 模型以影像思考")
        self.assertEqual(blocks[0]["access"], "normal")
        self.assertEqual(blocks[1]["access"], "paywalled")
        self.assertNotIn("付費牆", blocks[1]["headline"])
        self.assertNotIn("🔗", blocks[1]["headline"])
        # replacement block spans 2 lines, carries the original headline
        self.assertEqual(blocks[2]["access"], "free_replacement")
        self.assertEqual(blocks[2]["start"], 2)
        self.assertEqual(blocks[2]["end"], 4)
        self.assertIn("Hyperion", blocks[2]["original_headline"])

    def test_parse_ai_blocks_fails_closed_on_orphan_continuation(self):
        lines = ["　↳ 原文：孤兒續行 [🔗](https://x)"]  # continuation with no parent
        self.assertIsNone(run._parse_ai_blocks(lines))

    def test_parse_ai_blocks_fails_closed_on_unexpected_line(self):
        lines = ["- 正常標題 [🔗](https://x)", "以下是結果："]  # stray non-bullet
        self.assertIsNone(run._parse_ai_blocks(lines))


class CrossDedupGroupingTests(unittest.TestCase):
    def test_parse_cross_dedup_response_valid(self):
        out = '{"groups":[{"members":[2,3],"keep":2}]}'
        groups = run._parse_cross_dedup_response(out, 4)
        self.assertEqual(groups, [{"members": [2, 3], "keep": 2}])

    def test_parse_cross_dedup_response_empty_groups_ok(self):
        self.assertEqual(run._parse_cross_dedup_response('{"groups":[]}', 4), [])

    def test_parse_cross_dedup_response_rejects_invalid(self):
        bad = [
            "not json at all",
            '{"no_groups_key":1}',
            '{"groups":[{"members":[2,9],"keep":2}]}',   # 9 out of range (block_count=4)
            '{"groups":[{"members":[2,2],"keep":2}]}',   # duplicate member
            '{"groups":[{"members":[2],"keep":2}]}',      # <2 members
            '{"groups":[{"members":[2,3],"keep":4}]}',    # keep not in group
            '{"groups":[{"members":[1,2],"keep":1},{"members":[2,3],"keep":2}]}',  # overlap on 2
        ]
        for b in bad:
            self.assertIsNone(run._parse_cross_dedup_response(b, 4), b)

    def test_cross_dedup_prompt_contains_rules_and_blocks(self):
        blocks = run._parse_ai_blocks([
            "- 標題甲 [🔗](https://x)",
            "- 標題乙 [🔗](https://y)",
        ])
        p = run._cross_dedup_prompt(blocks, "2026/07/14 (Tue)")
        self.assertIn("#1", p)
        self.assertIn("標題甲", p)
        self.assertIn("groups", p)          # asks for JSON groups
        self.assertIn("同一", p)            # same-event language present

    def test_apply_cross_dedup_drops_non_kept_atomically(self):
        lines = [
            "- 甲 [🔗](https://a)",
            "- 乙（重複）[🔗](https://b)",
            "　↳ 原文：乙原始 [🔗](https://b2)  ⚠️ 付費牆（原文需訂閱）",
            "- 丙 [🔗](https://c)",
            "- 丁 [🔗](https://d)",
            "- 戊 [🔗](https://e)",
            "- 己 [🔗](https://f)",
        ]
        blocks = run._parse_ai_blocks(lines)      # 6 blocks (block #2 spans 2 lines)
        groups = [{"members": [1, 2], "keep": 1}]  # keep #1, drop #2 (+continuation)
        out = run._apply_cross_dedup(lines, blocks, groups)
        self.assertIsNotNone(out)
        self.assertNotIn("乙（重複）", "\n".join(out))
        self.assertNotIn("乙原始", "\n".join(out))          # continuation gone too
        self.assertIn("甲", "\n".join(out))
        self.assertEqual(len(out), 5)                         # 7 lines - 2 dropped

    def test_apply_cross_dedup_circuit_breaker_blocks_excessive(self):
        lines = [f"- 標題{i} [🔗](https://{i})" for i in range(8)]
        blocks = run._parse_ai_blocks(lines)   # 8 blocks
        groups = [{"members": [1, 2, 3, 4, 5, 6, 7], "keep": 1}]  # 8 -> 2 (>50%, < min(8,5))
        self.assertIsNone(run._apply_cross_dedup(lines, blocks, groups))

    def test_cross_dedup_ai_merges_same_event(self):
        lines = [
            "- 祖克柏豪賭 AI：單座資料中心上看 2,500 億美元 [🔗](https://a)",
            "- 全新 AI 模型以影像思考 [🔗](https://b)",
            "- 美國對中國 AI 的恐慌並未切中要點 [🔗](https://c)",
            "- 台積電營收創新高 [🔗](https://d)",
            "- SK 海力士獲利預警 [🔗](https://e)",
            "- Meta 路易斯安那資料中心 500 億美元 [🔗](https://f)",
        ]
        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            import subprocess
            return subprocess.CompletedProcess(["nullclaw"], 0,
                stdout='{"groups":[{"members":[1,6],"keep":1}]}', stderr="")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.dict(os.environ, {}, clear=False):
            os.environ.pop("NEWS_CROSS_DEDUP", None)
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        joined = "\n".join(out)
        self.assertIn("2,500 億", joined)          # kept (#1)
        self.assertNotIn("路易斯安那", joined)      # dropped (#6, same event)
        self.assertEqual(len(out), 5)

    def test_cross_dedup_ai_false_merge_kept(self):
        lines = [
            "- Google 投資 100 億美元興建日本資料中心 [🔗](https://a)",
            "- Microsoft 投資 50 億美元擴建德國資料中心 [🔗](https://b)",
            "- 台積電營收創新高 [🔗](https://c)",
            "- SK 海力士獲利預警 [🔗](https://d)",
            "- 全新 AI 模型以影像思考 [🔗](https://e)",
        ]
        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            import subprocess
            return subprocess.CompletedProcess(["nullclaw"], 0, stdout='{"groups":[]}', stderr="")
        with patch.object(run, "_run_nullclaw_agent", fake_agent):
            os.environ.pop("NEWS_CROSS_DEDUP", None)
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        self.assertEqual(len(out), 5)   # both kept

    def test_cross_dedup_ai_kill_switch_env(self):
        lines = ["- 甲 [🔗](https://a)", "- 乙 [🔗](https://b)"]
        called = {"n": 0}
        def fake_agent(*a, **k):
            called["n"] += 1
            import subprocess
            return subprocess.CompletedProcess(["nullclaw"], 0, stdout='{"groups":[]}', stderr="")
        with patch.object(run, "_run_nullclaw_agent", fake_agent):
            os.environ["NEWS_CROSS_DEDUP"] = "0"
            try:
                out = run._cross_dedup_ai(list(lines), "d", {}, {})
            finally:
                os.environ.pop("NEWS_CROSS_DEDUP", None)
        self.assertEqual(out, lines)
        self.assertEqual(called["n"], 0)   # no LLM call when disabled

    def test_cross_dedup_ai_llm_failure_safe(self):
        lines = ["- 甲 [🔗](https://a)", "- 乙 [🔗](https://b)", "- 丙 [🔗](https://c)"]
        def fake_agent(*a, **k):
            import subprocess
            return subprocess.CompletedProcess(["nullclaw"], 124, stdout="", stderr="timeout")
        with patch.object(run, "_run_nullclaw_agent", fake_agent):
            os.environ.pop("NEWS_CROSS_DEDUP", None)
            out = run._cross_dedup_ai(list(lines), "d", {}, {})
        self.assertEqual(out, lines)   # unchanged on failure


class CrossDedupVoteEnsembleTests(unittest.TestCase):
    """P4: N concurrent samples of the same prompt + K-of-N pair voting.

    Contract these tests pin:
      - module constants CROSS_DEDUP_SAMPLES (N) and CROSS_DEDUP_VOTE_K (K),
        overridable at call time by NEWS_CROSS_DEDUP_N / NEWS_CROSS_DEDUP_K
      - each sample parsed independently; a bad sample = zero votes, never an abort
      - groups = connected components over pairs with >= K votes (never raw model groups)
      - survivor chosen deterministically (free_replacement/normal > paywalled,
        then lowest block index) -- the model's own "keep" is ignored
      - traces: cross_dedup_skipped{reason=...} and cross_dedup_llm{n,k,samples,votes,kept}
    """

    # Sentinels for _queued_agent: a canned sample that fails instead of returning JSON.
    RC_FAIL = ("rc_fail",)
    RAISES = ("raises",)

    def _env(self, **overrides):
        # patch.dict restores on cleanup; NEWS_CROSS_DEDUP is popped so an ambient
        # kill switch in the shell cannot silently mask these tests.
        patcher = patch.dict(os.environ, overrides, clear=False)
        patcher.start()
        self.addCleanup(patcher.stop)
        os.environ.pop("NEWS_CROSS_DEDUP", None)

    def _queued_agent(self, outputs):
        """Thread-safe queue fake: each sample pops one canned stdout.

        Samples run concurrently so pop order is nondeterministic -- only the
        multiset of canned outputs is meaningful. deque.popleft() is atomic under
        CPython, unlike the list.pop(0) used by AiSubstageLanguageGateTests.
        """
        queue = collections.deque(outputs)
        calls = []

        def fake_agent(prompt, timeout_secs, variant, all_items, numbered):
            calls.append(variant)
            try:
                out = queue.popleft()
            except IndexError:
                raise AssertionError("more agent calls than canned sample outputs")
            if out is self.RC_FAIL:
                return subprocess.CompletedProcess(["nullclaw"], 124, stdout="", stderr="timeout")
            if out is self.RAISES:
                raise RuntimeError("sample blew up")
            return subprocess.CompletedProcess(["nullclaw"], 0, stdout=out, stderr="")

        return fake_agent, calls

    def _fixed_agent(self, stdout):
        return self._queued_agent(collections.deque([stdout] * 64))

    @staticmethod
    def _filler(n):
        # block #N <-> 標題N, so assertions can name blocks by their 1-based number
        return [f"- 標題{i} [🔗](https://{i})" for i in range(1, n + 1)]

    # a) pair-vote threshold
    def test_cross_dedup_ai_pair_needs_k_votes(self):
        lines = self._filler(6)
        # (2,3) gets K-1 votes and must NOT merge; (1,6) gets K votes and must merge.
        outputs = ['{"groups":[{"members":[2,3],"keep":2}]}'] * 2 + \
                  ['{"groups":[{"members":[1,6],"keep":1}]}'] * 3
        fake_agent, calls = self._queued_agent(outputs)
        self._env(NEWS_CROSS_DEDUP_N="5", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        joined = "\n".join(out)
        self.assertEqual(len(calls), 5)                 # N independent samples
        self.assertNotIn("標題6", joined)               # 3 votes >= K -> dropped
        self.assertIn("標題2", joined)                  # 2 votes < K -> both kept
        self.assertIn("標題3", joined)
        self.assertEqual(len(out), 5)

    # b) a bad sample contributes zero votes and must not abort the run
    def test_cross_dedup_ai_bad_samples_do_not_abort_run(self):
        lines = self._filler(6)
        # raise / rc=124 / unparseable JSON: any one aborts the old all-or-nothing
        # implementation before the good samples are ever counted. N=10 with 7 good
        # keeps ok_count (7) at min_ok(10,3)=7 so the merge is applied under the
        # abstain policy -- the point here is that the 3 bad samples don't abort it.
        outputs = [self.RAISES, self.RC_FAIL, "not json at all"] + \
                  ['{"groups":[{"members":[1,6],"keep":1}]}'] * 7
        fake_agent, calls = self._queued_agent(outputs)
        self._env(NEWS_CROSS_DEDUP_N="10", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        joined = "\n".join(out)
        self.assertEqual(len(calls), 10)                # bad samples never short-circuit the rest
        self.assertNotIn("標題6", joined)               # 7 good votes still reach K
        self.assertIn("標題1", joined)
        self.assertEqual(len(out), 5)

    # c) components are built from VOTED pairs, not from raw model groups
    def test_cross_dedup_ai_unvoted_bridge_does_not_merge_components(self):
        lines = self._filler(8)
        # (2,4) is the bridge and appears in only K-1 samples -> {1,2} and {4,5}
        # must stay two separate components rather than collapsing into {1,2,4,5}.
        outputs = ['{"groups":[{"members":[2,4],"keep":2}]}'] * 2 + \
                  ['{"groups":[{"members":[1,2],"keep":1},{"members":[4,5],"keep":4}]}'] * 3
        fake_agent, calls = self._queued_agent(outputs)
        self._env(NEWS_CROSS_DEDUP_N="5", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        joined = "\n".join(out)
        self.assertEqual(len(calls), 5)
        self.assertNotIn("標題2", joined)               # component {1,2} -> keep #1
        self.assertNotIn("標題5", joined)               # component {4,5} -> keep #4
        self.assertIn("標題4", joined)                  # bridge unvoted: #4 survives
        self.assertIn("標題1", joined)
        self.assertEqual(len(out), 6)

    def test_cross_dedup_ai_voted_bridge_merges_components(self):
        lines = self._filler(8)
        # same shape as above but the bridge now clears K, so union-find over voted
        # pairs must transitively merge {1,2} + {2,4} + {4,5} into one component.
        outputs = ['{"groups":[{"members":[2,4],"keep":2}]}'] * 3 + \
                  ['{"groups":[{"members":[1,2],"keep":1},{"members":[4,5],"keep":4}]}'] * 3
        fake_agent, calls = self._queued_agent(outputs)
        self._env(NEWS_CROSS_DEDUP_N="6", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        joined = "\n".join(out)
        self.assertEqual(len(calls), 6)
        self.assertIn("標題1", joined)                  # lowest index in the component
        for gone in ("標題2", "標題4", "標題5"):
            self.assertNotIn(gone, joined, gone)
        self.assertEqual(len(out), 5)

    # d) deterministic keep policy -- the model's "keep" is ignored
    def test_cross_dedup_ai_keeps_free_access_over_paywalled(self):
        lines = [
            f"- 甲 [🔗](https://a)  {run.PAYWALL_NOTE}",                                  # #1 paywalled
            "- 乙 [🔗](https://b)",                                                        # #2 ...
            f"{run.PAYWALL_CONT_PREFIX}原文：乙原始 [🔗](https://b2)  {run.PAYWALL_NOTE}",  # ... free_replacement
            f"- 丙 [🔗](https://c)  {run.PAYWALL_NOTE}",                                  # #3 paywalled
            "- 丁 [🔗](https://d)",                                                        # #4 normal
        ] + [f"- 標題{i} [🔗](https://{i})" for i in range(5, 9)]                          # #5..#8
        blocks = run._parse_ai_blocks(lines)
        self.assertEqual([b["access"] for b in blocks][:4],
                         ["paywalled", "free_replacement", "paywalled", "normal"])
        # model asks to keep the paywalled member of each pair; policy must overrule it
        fake_agent, calls = self._fixed_agent(
            '{"groups":[{"members":[1,2],"keep":1},{"members":[3,4],"keep":3}]}')
        self._env(NEWS_CROSS_DEDUP_N="3", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        joined = "\n".join(out)
        self.assertNotIn("甲", joined)                  # paywalled loses to free_replacement
        self.assertIn("乙", joined)
        self.assertIn("乙原始", joined)                 # survivor keeps its continuation line
        self.assertNotIn("丙", joined)                  # paywalled loses to normal
        self.assertIn("丁", joined)
        self.assertEqual(len(out), 7)                   # 9 lines - 2 single-line blocks

    def test_cross_dedup_ai_tie_breaks_to_lowest_block_index(self):
        lines = self._filler(6)
        # all blocks are "normal", so the tie-break alone decides; model says keep #5
        fake_agent, calls = self._fixed_agent('{"groups":[{"members":[2,5],"keep":5}]}')
        self._env(NEWS_CROSS_DEDUP_N="3", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        joined = "\n".join(out)
        self.assertIn("標題2", joined)                  # lowest index wins the tie
        self.assertNotIn("標題5", joined)
        self.assertEqual(len(out), 5)

    # e) circuit breaker: >=1 drop must be possible for any n >= 2, 50% cap still holds
    def test_apply_cross_dedup_allows_one_drop_but_still_caps_at_half(self):
        for n in range(2, 6):
            lines = self._filler(n)
            blocks = run._parse_ai_blocks(lines)
            groups = [{"members": [1, 2], "keep": 1}]      # exactly one drop
            out = run._apply_cross_dedup(lines, blocks, groups)
            self.assertIsNotNone(out, f"n={n} must permit a single drop")
            self.assertEqual(len(out), n - 1, f"n={n}")
        for n in (4, 5, 6, 8):
            lines = self._filler(n)
            blocks = run._parse_ai_blocks(lines)
            drops = n // 2 + 1                             # one past the 50% cap
            groups = [{"members": list(range(1, drops + 2)), "keep": 1}]
            self.assertIsNone(run._apply_cross_dedup(lines, blocks, groups),
                              f"n={n} must still reject {drops} drops")

    # Limitation 3 timing: cross_dedup_llm must carry P3 wall-clock + budget +
    # abandoned-worker fields so a day of cron traces can tell "healthy 26s" from
    # "sat on the 120s ceiling" and attribute budget burn to P3.
    def test_cross_dedup_llm_trace_carries_timing_fields(self):
        lines = self._filler(6)
        fake_agent, calls = self._fixed_agent('{"groups":[]}')
        traces = []
        self._env(NEWS_CROSS_DEDUP_N="3", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            run._cross_dedup_ai(list(lines), "2026/07/21 (Tue)", {}, {})
        t = [f for e, f in traces if e == "cross_dedup_llm"][0]
        self.assertIsInstance(t.get("elapsed_ms"), int)
        self.assertGreaterEqual(t["elapsed_ms"], 0)
        self.assertEqual(t.get("ensemble_timeout_secs"), run.CROSS_DEDUP_TOTAL_TIMEOUT_SECS)
        self.assertEqual(t.get("abandoned"), 0)            # all samples finished
        self.assertIn("remaining_to_kill_at_start", t)     # present (None without cron env)
        self.assertIn("remaining_to_kill_at_end", t)

    def test_cross_dedup_llm_reports_abandoned_workers_past_deadline(self):
        # The core signal of the instrumentation: a sample still running when the
        # ensemble join deadline passes must be counted as abandoned (= P3 sat on its
        # ceiling). Without this test a broken is_alive() count fails silently in the
        # ONLY case abandoned exists to catch.
        import time as _time

        def slow_agent(prompt, timeout_secs, variant, all_items, numbered):
            _time.sleep(0.3)   # outlives the 0.05s join deadline patched below
            return subprocess.CompletedProcess(["nullclaw"], 0, stdout='{"groups":[]}', stderr="")

        lines = self._filler(6)
        traces = []
        self._env(NEWS_CROSS_DEDUP_N="2", NEWS_CROSS_DEDUP_K="2")
        with patch.object(run, "CROSS_DEDUP_TOTAL_TIMEOUT_SECS", 0.05), \
             patch.object(run, "CROSS_DEDUP_STAGGER_SECS", 0.0), \
             patch.object(run, "_run_nullclaw_agent", slow_agent), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            run._cross_dedup_ai(list(lines), "2026/07/21 (Tue)", {}, {})
        t = [f for e, f in traces if e == "cross_dedup_llm"][0]
        self.assertGreaterEqual(t["abandoned"], 1)         # >=1 thread outlived the join
        self.assertIsInstance(t["elapsed_ms"], int)

    # Measured on the 2026-07-18 input over 14 real N=7 runs. Drops per run were mostly
    # 0-3, but one run bridged an unrelated story into a true group and cut the section
    # from 10 blocks to 5, landing exactly on the old 50% cap without tripping it.
    # Concurrent samples turn out to be correlated, so a whole run swings aggressive
    # together and a false pair collects as many votes as a true one — the vote threshold
    # cannot filter that, so the drop cap is what has to hold the line. The cap is 40%
    # and not tighter because two other runs legitimately found 4 drops of 10 (the full
    # same-event trio plus two more true pairs); rejection is all-or-nothing, so a 30%
    # cap threw those correct results away wholesale.
    def test_apply_cross_dedup_rejects_half_section_collapse(self):
        lines = self._filler(10)
        blocks = run._parse_ai_blocks(lines)
        collapse = [{"members": [1, 2, 7, 8], "keep": 2},
                    {"members": [3, 6], "keep": 3},
                    {"members": [9, 10], "keep": 9}]        # 5 drops — the observed run
        self.assertIsNone(run._apply_cross_dedup(lines, blocks, collapse),
                          "a 5-of-10 collapse must be rejected")
        legit = [{"members": [1, 2, 7], "keep": 2},
                 {"members": [3, 6], "keep": 3},
                 {"members": [9, 10], "keep": 9}]            # 4 drops — also observed, correct
        out = run._apply_cross_dedup(lines, blocks, legit)
        self.assertIsNotNone(out, "4 drops of 10 must survive the cap")
        self.assertEqual(len(out), 6)
        moderate = [{"members": [2, 7], "keep": 2},
                    {"members": [3, 6], "keep": 3}]          # 2 drops — the common case
        out = run._apply_cross_dedup(lines, blocks, moderate)
        self.assertIsNotNone(out, "2 drops of 10 must still be allowed")
        self.assertEqual(len(out), 8)
        # the tightened cap must not resurrect the zero-drop defect on short sections
        for n in range(2, 6):
            short = self._filler(n)
            self.assertIsNotNone(
                run._apply_cross_dedup(short, run._parse_ai_blocks(short),
                                       [{"members": [1, 2], "keep": 1}]),
                f"n={n} must still permit a single drop")

    # f) _parse_ai_blocks returning None is currently a silent no-op
    def test_cross_dedup_ai_traces_block_parse_failure(self):
        lines = [f"{run.PAYWALL_CONT_PREFIX}原文：孤兒 [🔗](https://a)  {run.PAYWALL_NOTE}"]
        self.assertIsNone(run._parse_ai_blocks(lines))     # orphan continuation, fails closed
        traces = []
        fake_agent, calls = self._fixed_agent('{"groups":[]}')
        self._env()
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        self.assertEqual(out, lines)
        self.assertEqual(len(calls), 0)                    # no LLM call once parsing failed
        skipped = [f for e, f in traces if e == "cross_dedup_skipped"]
        self.assertEqual(len(skipped), 1, traces)
        self.assertEqual(skipped[0].get("reason"), "parse_failed")

    def test_cross_dedup_ai_traces_too_few_blocks_distinctly(self):
        lines = ["- 只有一則 [🔗](https://a)"]              # parses fine, just nothing to dedup
        self.assertEqual(len(run._parse_ai_blocks(lines)), 1)
        traces = []
        fake_agent, calls = self._fixed_agent('{"groups":[]}')
        self._env()
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        self.assertEqual(out, lines)
        self.assertEqual(len(calls), 0)
        skipped = [f for e, f in traces if e == "cross_dedup_skipped"]
        self.assertEqual(len(skipped), 1, traces)
        reason = skipped[0].get("reason")
        self.assertTrue(reason)
        self.assertNotEqual(reason, "parse_failed")        # must not be confused with a bug

    def test_cross_dedup_ai_traces_votes_and_sample_outcomes(self):
        lines = self._filler(6)
        outputs = [self.RC_FAIL] + ['{"groups":[{"members":[1,6],"keep":1}]}'] * 3
        fake_agent, calls = self._queued_agent(outputs)
        traces = []
        self._env(NEWS_CROSS_DEDUP_N="4", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        summary = [f for e, f in traces if e == "cross_dedup_llm"]
        self.assertEqual(len(summary), 1, traces)
        f = summary[0]
        self.assertEqual(f.get("n"), 4)
        self.assertEqual(f.get("k"), 3)
        self.assertEqual(len(f.get("samples") or []), 4)   # one outcome per sample
        self.assertEqual((f.get("samples") or []).count("ok"), 3)
        votes = [list(v) for v in (f.get("votes") or [])]  # [member_a, member_b, count]
        self.assertIn([1, 6, 3], votes)
        self.assertEqual(f.get("kept"), [1])               # deterministic survivor

    # Limitation 2: concurrent samples share one host+provider, so their failures
    # are correlated and a small surviving set can be a correlated cluster that made
    # the same mistake. Below min_ok successes, abstain entirely rather than let the
    # old proportional rule dilute K toward 1. min_ok(N=7,K=3)=5.
    def test_cross_dedup_ai_abstains_when_too_few_samples_succeed(self):
        lines = self._filler(6)
        # 4 failures + 3 votes for (1,6): ok_count=3 < min_ok=5 -> abstain, nothing dropped.
        outputs = [self.RC_FAIL] * 4 + ['{"groups":[{"members":[1,6],"keep":1}]}'] * 3
        fake_agent, calls = self._queued_agent(outputs)
        traces = []
        self._env(NEWS_CROSS_DEDUP_N="7", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        self.assertEqual(out, lines)                       # unchanged: no merge applied
        self.assertIn("標題6", "\n".join(out))
        summary = [f for e, f in traces if e == "cross_dedup_llm"]
        self.assertEqual(len(summary), 1, traces)
        self.assertEqual(summary[0].get("rejected"), "insufficient_samples")
        self.assertEqual(summary[0].get("ok_samples"), 3)

    def test_cross_dedup_ai_applies_at_exactly_min_ok_samples(self):
        lines = self._filler(6)
        # 2 failures + 5 votes for (1,6): ok_count=5 == min_ok -> not abstained, merges.
        outputs = [self.RC_FAIL] * 2 + ['{"groups":[{"members":[1,6],"keep":1}]}'] * 5
        fake_agent, calls = self._queued_agent(outputs)
        self._env(NEWS_CROSS_DEDUP_N="7", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        self.assertNotIn("標題6", "\n".join(out))          # 5 votes >= K and ok >= min_ok
        self.assertEqual(len(out), 5)

    # Limitation B: a single-vote pair must survive into the trace tally so a future
    # near-miss pass can see the edges the K threshold rejected. The old floor of
    # max(1, k_eff-1) hid every 1-vote pair at the default K=3.
    def test_cross_dedup_ai_tally_retains_single_vote_pairs(self):
        lines = self._filler(6)
        # (1,6) gets 5 votes and merges; (2,3) gets exactly 1 vote -- a near-miss.
        outputs = ['{"groups":[{"members":[2,3],"keep":2}]}'] * 1 + \
                  ['{"groups":[{"members":[1,6],"keep":1}]}'] * 5
        fake_agent, calls = self._queued_agent(outputs)
        traces = []
        self._env(NEWS_CROSS_DEDUP_N="6", NEWS_CROSS_DEDUP_K="3")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda e, **f: traces.append((e, f))):
            run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        summary = [f for e, f in traces if e == "cross_dedup_llm"][0]
        votes = [list(v) for v in (summary.get("votes") or [])]
        self.assertIn([2, 3, 1], votes)                    # single-vote near-miss retained
        self.assertIn([1, 6, 5], votes)

    # g) N=1 restores single-sample behaviour; default N is > 1
    def test_cross_dedup_ai_default_multi_sample_and_env_n1_restores_single(self):
        self.assertIsNotNone(getattr(run, "CROSS_DEDUP_SAMPLES", None),
                             "default sample count N must be a module constant")
        self.assertIsNotNone(getattr(run, "CROSS_DEDUP_VOTE_K", None),
                             "default vote threshold K must be a module constant")
        self.assertGreater(run.CROSS_DEDUP_SAMPLES, 1)
        self.assertGreaterEqual(run.CROSS_DEDUP_SAMPLES, run.CROSS_DEDUP_VOTE_K)

        lines = self._filler(6)
        body = '{"groups":[{"members":[1,6],"keep":1}]}'

        fake_agent, calls = self._fixed_agent(body)
        self._env()
        os.environ.pop("NEWS_CROSS_DEDUP_N", None)
        os.environ.pop("NEWS_CROSS_DEDUP_K", None)
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        self.assertEqual(len(calls), run.CROSS_DEDUP_SAMPLES)

        fake_agent, calls = self._fixed_agent(body)
        self._env(NEWS_CROSS_DEDUP_N="1", NEWS_CROSS_DEDUP_K="1")
        with patch.object(run, "_run_nullclaw_agent", fake_agent), \
             patch.object(run, "log_trace", lambda *a, **k: None):
            out = run._cross_dedup_ai(list(lines), "2026/07/14 (Tue)", {}, {})
        self.assertEqual(len(calls), 1)                    # N=1 -> exactly the old one call
        self.assertNotIn("標題6", "\n".join(out))          # and the lone sample still applies
        self.assertEqual(len(out), 5)


class NewsThemeClassifyPromptTests(unittest.TestCase):
    def test_prompt_lists_blocks_enum_and_dominant_peg(self):
        blocks = run._parse_ai_blocks([
            "- OpenAI 推出 GPT-6 [🔗](https://a)",
            "- 美國擴大 AI 晶片出口管制 [🔗](https://b)",
        ])
        p = run._theme_classify_prompt(blocks, "2026/07/23 (Thu)")
        self.assertIn("#1", p)
        self.assertIn("GPT-6", p)
        self.assertIn(run.THEME_PRODUCT, p)      # enum present
        self.assertIn(run.THEME_POLICY, p)
        self.assertIn("主要新聞點", p)            # dominant news peg language
        self.assertIn("並列難分", p)              # priority is tie-break ONLY, not first-match
        self.assertIn("企業採用", p)              # enterprise adoption mapped (curbs 其他)
        self.assertIn("安全", p)                  # AI safety/alignment reports mapped
        self.assertIn("labels", p)               # asks for JSON labels


class NewsThemeParseTests(unittest.TestCase):
    def test_valid(self):
        out = '{"labels":[{"id":1,"theme":"產品發布"},{"id":2,"theme":"政策監管"}]}'
        self.assertEqual(run._parse_theme_response(out, 2),
                         {1: "產品發布", 2: "政策監管"})

    def test_reject(self):
        bad = [
            "not json",
            '{"nope":[]}',
            '{"labels":[{"id":1,"theme":"產品發布"}]}',                 # count != 2
            '{"labels":[{"id":1,"theme":"產品發布"},{"id":1,"theme":"政策監管"}]}',  # dup id
            '{"labels":[{"id":1,"theme":"產品發布"},{"id":3,"theme":"政策監管"}]}',  # id out of range
            '{"labels":[{"id":1,"theme":"娛樂"},{"id":2,"theme":"政策監管"}]}',      # illegal theme
            '{"labels":[{"id":true,"theme":"產品發布"},{"id":2,"theme":"政策監管"}]}',  # bool id
            '{"labels":[{"id":1,"theme":["產品發布"]},{"id":2,"theme":"政策監管"}]}',   # non-str (unhashable) theme
            '{"labels":[1,2]}',                                                          # non-dict entry
        ]
        for b in bad:
            self.assertIsNone(run._parse_theme_response(b, 2), b)


if __name__ == "__main__":
    unittest.main()
