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


if __name__ == "__main__":
    unittest.main()
