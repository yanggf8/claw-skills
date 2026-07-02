"""Unit tests for lib/news_quality.py.

Run: python3 lib/test_news_quality.py
"""
import json
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import news_quality  # noqa: E402


class DecodeGoogleNewsUrlTests(unittest.TestCase):
    def test_returns_none_when_http_get_raises(self):
        with mock.patch.object(news_quality, "_http_get", side_effect=OSError("network")):
            result = news_quality.decode_google_news_url("https://news.google.com/rss/articles/CBMi")
        self.assertIsNone(result)

    def test_returns_none_when_http_get_returns_junk(self):
        with mock.patch.object(news_quality, "_http_get", return_value="<html>no tokens</html>"):
            result = news_quality.decode_google_news_url("https://news.google.com/rss/articles/CBMi")
        self.assertIsNone(result)

    def test_returns_none_when_http_post_raises(self):
        html = (
            '<div data-n-a-id="ART123" data-n-a-ts="999" data-n-a-sg="SIG456"></div>'
        )
        with mock.patch.object(news_quality, "_http_get", return_value=html), mock.patch.object(
            news_quality, "_http_post", side_effect=TimeoutError("slow")
        ):
            result = news_quality.decode_google_news_url("https://news.google.com/rss/articles/CBMi")
        self.assertIsNone(result)


class FetchArticleTextTests(unittest.TestCase):
    def test_truncated_true_on_paywall_marker(self):
        body = (
            "<html><body><p>Headline intro.</p>"
            "<p>继续阅读请点击这里，进入FT中文网</p></body></html>"
        )
        with mock.patch.object(news_quality, "_http_get_with_url",
                               return_value=(body, "https://example.com/story")):
            article = news_quality.fetch_article_text("https://example.com/story")
        self.assertTrue(article["truncated"])
        self.assertIn("继续阅读", article["text"])

    def test_truncated_false_on_long_clean_body(self):
        paragraphs = " ".join(["This is a clean news paragraph about markets."] * 40)
        body = f"<html><body><article><p>{paragraphs}</p></article></body></html>"
        with mock.patch.object(news_quality, "_http_get_with_url",
                               return_value=(body, "https://example.com/long-story")):
            article = news_quality.fetch_article_text("https://example.com/long-story")
        self.assertFalse(article["truncated"])
        self.assertGreater(article["word_count"], 100)
        self.assertNotIn("error", article)


def _set_config(deny=(), deny_domains=(), paywall_domains=(), trusted=()):
    """Inject a known active config (module seeds are intentionally empty)."""
    news_quality._ACTIVE_CONFIG = {
        "deny": set(deny), "deny_domains": set(deny_domains),
        "paywall_domains": set(paywall_domains), "trusted": set(trusted),
    }


class ClassifyQualityTests(unittest.TestCase):
    def setUp(self):
        _set_config(deny={"Example Content Farm"},
                    deny_domains={"chinatimes.com"},
                    paywall_domains={"zh.cn.nikkei.com", "ftchinese.com"})

    def tearDown(self):
        news_quality._ACTIVE_CONFIG = None

    def test_deny_source_dropped(self):
        item = {"title": "Some headline", "link": "x", "source_name": "Example Content Farm"}
        verdict = news_quality.classify_quality(item, None)
        self.assertEqual(verdict["action"], "drop")
        self.assertEqual(verdict["reason"], "deny")

    def test_deny_domain_dropped(self):
        item = {"title": "headline", "link": "x", "source_name": "Some Publisher"}
        article = {"host": "www.chinatimes.com", "text": "body", "truncated": False}
        verdict = news_quality.classify_quality(item, article)
        self.assertEqual(verdict["action"], "drop")
        self.assertEqual(verdict["reason"], "deny")

    def test_promo_title_dropped(self):
        item = {"title": "10 tips to boost your portfolio overnight", "link": "x",
                "source_name": "Random Blog"}
        article = {"host": "example.com", "text": "Normal body.", "truncated": False}
        verdict = news_quality.classify_quality(item, article)
        self.assertEqual(verdict["action"], "drop")
        self.assertEqual(verdict["reason"], "promo")

    def test_paywall_domain_is_title_only(self):
        item = {"title": "Markets column", "link": "x", "source_name": "日經中文網"}
        article = {"host": "zh.cn.nikkei.com", "text": "请登录", "truncated": True}
        verdict = news_quality.classify_quality(item, article)
        self.assertEqual(verdict["action"], "title_only")
        self.assertEqual(verdict["reason"], "paywalled")

    def test_clean_item_kept(self):
        item = {"title": "Taiwan exports rise on semiconductor demand", "link": "x",
                "source_name": "Reuters"}
        article = {"host": "reuters.com", "text": "Taiwan export figures rose.", "truncated": False}
        verdict = news_quality.classify_quality(item, article)
        self.assertEqual(verdict["action"], "keep")
        self.assertIsNone(verdict["reason"])


class PrecheckActionTests(unittest.TestCase):
    def setUp(self):
        _set_config(deny_domains={"chinatimes.com"},
                    paywall_domains={"zh.cn.nikkei.com", "ftchinese.com"})

    def tearDown(self):
        news_quality._ACTIVE_CONFIG = None

    def test_paywall_deterministic_regardless_of_decode(self):
        """A paywall-domain source resolves to title_only whether the body fetch
        succeeds OR fails — no network-timing nondeterminism (review finding #4)."""
        item = {"title": "Nikkei column", "link": "https://news.google.com/rss/articles/x",
                "source_name": "日經中文網"}
        decoded = "https://zh.cn.nikkei.com/story/1"
        # decode succeeds, fetch raises:
        with mock.patch.object(news_quality, "decode_google_news_url", return_value=decoded), \
             mock.patch.object(news_quality, "fetch_article_text", side_effect=RuntimeError("timeout")):
            r1 = news_quality.precheck_action(item)
        self.assertEqual(r1["action"], "title_only")
        # decode succeeds, fetch succeeds (clean body): host still wins → title_only
        with mock.patch.object(news_quality, "decode_google_news_url", return_value=decoded), \
             mock.patch.object(news_quality, "fetch_article_text",
                               return_value={"host": "zh.cn.nikkei.com", "text": "body", "truncated": False}):
            r2 = news_quality.precheck_action(item)
        self.assertEqual(r2["action"], "title_only")

    def test_paywall_domain_carries_decoded_url(self):
        """title_only verdict must carry the real publisher URL so the render
        stage / replacement lookup has the host. Locks the plumbing contract."""
        item = {"title": "Nikkei column", "link": "https://news.google.com/rss/articles/x",
                "source_name": "日經中文網"}
        decoded = "https://zh.cn.nikkei.com/story/1"
        with mock.patch.object(news_quality, "decode_google_news_url", return_value=decoded):
            r = news_quality.precheck_action(item)
        self.assertEqual(r["action"], "title_only")
        self.assertEqual(r["decoded_url"], decoded)

    def test_config_paywall_flags_nytimes_title_only(self):
        """Regression for the reported incident: with nytimes.com in the config
        paywall list, a NYTimes URL resolves to title_only (was 'keep' when the
        list was empty). Config-driven — NOT a code seed."""
        _set_config(paywall_domains={"nytimes.com"})
        item = {"title": "They built the most powerful AI - The New York Times",
                "link": "https://news.google.com/rss/articles/nyt",
                "source_name": "The New York Times"}
        with mock.patch.object(news_quality, "decode_google_news_url",
                               return_value="https://www.nytimes.com/2026/07/01/tech/ai.html"):
            r = news_quality.precheck_action(item)
        self.assertEqual(r["action"], "title_only")
        self.assertEqual(r["reason"], "paywalled")
        self.assertEqual(r["decoded_url"], "https://www.nytimes.com/2026/07/01/tech/ai.html")

    def test_deny_domain_dropped(self):
        item = {"title": "x", "link": "https://news.google.com/rss/articles/c",
                "source_name": "ChinaTimes"}
        with mock.patch.object(news_quality, "decode_google_news_url",
                               return_value="https://www.chinatimes.com/newspapers/1"):
            r = news_quality.precheck_action(item)
        self.assertEqual(r["action"], "drop")
        self.assertEqual(r["reason"], "deny")

    def test_unknown_source_kept_when_decode_fails(self):
        item = {"title": "Local news", "link": "https://news.google.com/rss/articles/u",
                "source_name": "Obscure Weekly"}
        with mock.patch.object(news_quality, "decode_google_news_url", return_value=None):
            r = news_quality.precheck_action(item)
        self.assertEqual(r["action"], "keep")

    def test_promo_title_dropped_even_when_body_unfetchable(self):
        """Title-safe promo still drops when the body fetch errors (review #6)."""
        item = {"title": "10 ways to double your money", "link": "https://news.google.com/rss/articles/p",
                "source_name": "Promo Blog"}
        with mock.patch.object(news_quality, "decode_google_news_url",
                               return_value="https://promoblog.example/x"), \
             mock.patch.object(news_quality, "fetch_article_text",
                               return_value={"host": "promoblog.example", "error": "Timeout",
                                             "text": "", "truncated": False}):
            r = news_quality.precheck_action(item)
        self.assertEqual(r["action"], "drop")
        self.assertEqual(r["reason"], "promo")

    def test_legit_title_with_limit_word_not_promo(self):
        """限時 inside a real headline must NOT be treated as promo (over-match guard)."""
        self.assertFalse(news_quality._matches_promo_title("央行宣布限時降息措施"))
        # Unambiguous promo title IS flagged.
        self.assertTrue(news_quality._matches_promo_title("10 ways to get rich"))

    def test_legit_body_with_limit_word_kept(self):
        """A real story whose BODY merely mentions 限時/優惠/折扣 must NOT be dropped —
        body text is no longer matched against promo patterns at all."""
        item = {"title": "央行政策動向", "link": "x", "source_name": "Reuters"}
        article = {"host": "reuters.com", "text": "央行推出限時優惠方案以刺激經濟", "truncated": False}
        verdict = news_quality.classify_quality(item, article)
        self.assertEqual(verdict["action"], "keep")

    def test_fetch_cache_avoids_second_decode(self):
        item = {"title": "x", "link": "https://news.google.com/rss/articles/m",
                "source_name": "Reuters"}
        cache: dict = {}
        with mock.patch.object(news_quality, "decode_google_news_url",
                               return_value="https://reuters.com/a") as dec, \
             mock.patch.object(news_quality, "fetch_article_text",
                               return_value={"host": "reuters.com", "text": "body", "truncated": False}):
            news_quality.precheck_action(item, fetch_cache=cache)
            news_quality.precheck_action(item, fetch_cache=cache)
        self.assertEqual(dec.call_count, 1)  # second call served from cache

    def test_precheck_action_uses_caller_supplied_decoded_url(self):
        """Caller-supplied decoded_url bypasses Google News decoding."""
        decoded = "https://www.reuters.com/technology/ai-story/"
        item = {"title": "AI story", "link": "https://www.bing.com/news/apiclick.aspx?x=1",
                "decoded_url": decoded, "source_name": "Reuters"}
        with mock.patch.object(news_quality, "decode_google_news_url") as dec, \
             mock.patch.object(news_quality, "fetch_article_text",
                               return_value={"host": "www.reuters.com", "text": "body",
                                             "truncated": False}):
            r = news_quality.precheck_action(item)
        dec.assert_not_called()
        self.assertEqual(r["action"], "keep")
        self.assertEqual(r["decoded_url"], decoded)

    def test_precheck_action_stale_cache_does_not_shadow_decoded_url_hint(self):
        """Caller decoded_url wins even when fetch_cache has a stale unresolved entry."""
        link = "https://www.bing.com/news/apiclick.aspx?x=1"
        decoded = "https://www.reuters.com/technology/ai-story/"
        item = {"title": "AI story", "link": link, "decoded_url": decoded,
                "source_name": "Reuters"}
        cache = {link: {"action": "keep", "reason": "unresolved", "decoded_url": None}}
        with mock.patch.object(news_quality, "decode_google_news_url") as dec, \
             mock.patch.object(news_quality, "fetch_article_text",
                               return_value={"host": "www.reuters.com", "text": "body",
                                             "truncated": False}):
            r = news_quality.precheck_action(item, fetch_cache=cache)
        dec.assert_not_called()
        self.assertEqual(r["action"], "keep")
        self.assertEqual(r["decoded_url"], decoded)
        self.assertEqual(cache[link]["decoded_url"], decoded)


class HostMatchTests(unittest.TestCase):
    def test_suffix_match_not_substring(self):
        domains = {"ft.com", "chinatimes.com"}
        # exact and subdomain match
        self.assertTrue(news_quality._host_in("ft.com", domains))
        self.assertTrue(news_quality._host_in("www.ft.com", domains))
        self.assertTrue(news_quality._host_in("www.chinatimes.com", domains))
        # must NOT substring-match an unrelated host
        self.assertFalse(news_quality._host_in("craft.com", domains))
        self.assertFalse(news_quality._host_in("notchinatimes.com.evil.example", domains))
        self.assertFalse(news_quality._host_in("", domains))


class LoadQualityConfigTests(unittest.TestCase):
    def test_returns_seeds_when_file_missing(self):
        missing = os.path.join(tempfile.gettempdir(), "news-quality-missing.json")
        cfg = news_quality.load_quality_config(missing)
        self.assertEqual(cfg["trusted"], news_quality.TRUSTED_SOURCES)
        self.assertEqual(cfg["deny"], news_quality.DENY_SOURCES)
        self.assertEqual(cfg["paywall_domains"], news_quality.PAYWALL_DOMAINS)

    def test_merges_when_file_present(self):
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
            json.dump(
                {
                    "trusted": ["My Trusted Outlet"],
                    "deny": ["Another Content Farm"],
                    "paywall_domains": ["paywall.example.com"],
                },
                fh,
            )
            path = fh.name
        try:
            cfg = news_quality.load_quality_config(path)
            # Seeds are empty; the config file supplies the values.
            self.assertIn("My Trusted Outlet", cfg["trusted"])
            self.assertIn("Another Content Farm", cfg["deny"])
            self.assertIn("paywall.example.com", cfg["paywall_domains"])
        finally:
            os.unlink(path)


if __name__ == "__main__":
    unittest.main()