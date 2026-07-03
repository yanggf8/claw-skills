from __future__ import annotations

import hashlib
import json
import os
import re
from datetime import date
from typing import Any
from urllib.parse import urlencode, urlparse

TRUSTED_SOURCES: set[str] = {
    "Nikkei",
    "日經中文網",
    "Reuters",
    "Bloomberg",
    "Financial Times",
    "FT中文網",
    "中央社",
    "BBC",
    "The Wall Street Journal",
}

# Seeds are intentionally EMPTY — per the repo's "no hardcode / no default data"
# rule, the operator owns these lists via ~/.nullclaw/news-quality-sources.json
# (loaded and merged by load_quality_config). Shipping a publisher in a baked-in
# drop list would silently remove a real source with no opt-out.
DENY_SOURCES: set[str] = set()

# Hosts dropped entirely (low-trust / blocked / un-fetchable). Distinct from
# PAYWALL_DOMAINS: deny = remove the item; paywall = keep it as its headline.
DENY_DOMAINS: set[str] = set()

# Hosts that are gated but reputable. We cannot read the body, so the item keeps
# its (LLM) headline rather than being dropped.
PAYWALL_DOMAINS: set[str] = set()

PAYWALL_MARKERS: list[str] = [
    "继续阅读",
    "繼續閱讀",
    "请登录",
    "請登入",
    "進入FT中文網",
    "进入FT中文网",
    "continue reading",
    "subscribe to read",
    "sign in to read",
    "已是会员",
]

# Promo patterns: ONLY unambiguous, anchored promotional markers safe to match
# against a short headline. We deliberately exclude bare substrings like
# 優惠|折扣|限時 — they over-match ordinary business words inside legitimate news
# (e.g. 限時降息) and silently drop real stories. Promo dropping is title-only.
PROMO_TITLE_PATTERNS: list[re.Pattern[str]] = [
    re.compile(r"\b\d+\s+(tips|ways|reasons|things)\b", re.IGNORECASE),
    re.compile(r"(press release|sponsored|advertorial)", re.IGNORECASE),
    re.compile(r"立即(选购|購買|下單)"),
]

_USER_AGENT = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
)

_DEFAULT_CONFIG_PATH = os.path.join(
    os.path.expanduser("~/.nullclaw"),
    "news-quality-sources.json",
)

_CACHE_ROOT = os.path.join(
    os.path.expanduser("~/.nullclaw"),
    ".news-quality-cache",
)


def _http_get(url: str, timeout: float = 5.0) -> str:
    import urllib.request

    req = urllib.request.Request(url, headers={"User-Agent": _USER_AGENT})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return resp.read().decode("utf-8", errors="replace")


def _http_get_with_url(url: str, timeout: float = 5.0) -> tuple[str, str]:
    """Like _http_get but also returns the final (post-redirect) URL, so the host
    used for paywall/deny classification reflects where we actually landed."""
    import urllib.request

    req = urllib.request.Request(url, headers={"User-Agent": _USER_AGENT})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = resp.read().decode("utf-8", errors="replace")
        return body, resp.geturl()


def _http_post(url: str, data: str, timeout: float = 5.0) -> str:
    import urllib.request

    req = urllib.request.Request(
        url,
        data=data.encode("utf-8"),
        headers={
            "User-Agent": _USER_AGENT,
            "Content-Type": "application/x-www-form-urlencoded;charset=UTF-8",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return resp.read().decode("utf-8", errors="replace")


def _seed_config() -> dict[str, set[str]]:
    return {
        "trusted": set(TRUSTED_SOURCES),
        "deny": set(DENY_SOURCES),
        "deny_domains": set(DENY_DOMAINS),
        "paywall_domains": set(PAYWALL_DOMAINS),
    }


def load_quality_config(path: str | None = None) -> dict[str, set[str]]:
    cfg = _seed_config()
    config_path = path if path is not None else _DEFAULT_CONFIG_PATH
    try:
        with open(config_path, encoding="utf-8") as fh:
            data = json.load(fh)
        if not isinstance(data, dict):
            return cfg
        for key in ("trusted", "deny", "deny_domains", "paywall_domains"):
            if isinstance(data.get(key), list):
                cfg[key] |= {str(x) for x in data[key]}
    except Exception:
        pass
    return cfg


_ACTIVE_CONFIG: dict[str, set[str]] | None = None


def active_config() -> dict[str, set[str]]:
    """Return the merged trusted/deny/paywall config, loading it once and caching.

    Lets the optional ~/.nullclaw/news-quality-sources.json overrides actually
    affect classification (the seed module-level sets are the fallback).
    """
    global _ACTIVE_CONFIG
    if _ACTIVE_CONFIG is None:
        _ACTIVE_CONFIG = load_quality_config()
    return _ACTIVE_CONFIG


def _deny_sources() -> set[str]:
    return active_config()["deny"]


# Note: the `trusted` config dimension is loaded and mergeable for forward
# compatibility but is intentionally not consulted by the current deterministic
# classification (paywall/deny decide everything). Do not add a `_trusted_sources`
# accessor until a classification branch actually gates on it.


def _host_in(host: str, domains: set[str]) -> bool:
    """Suffix-aware host match: host equals the domain or is a subdomain of it.
    Avoids substring false-positives (e.g. 'ft.com' must not match 'craft.com')."""
    if not host:
        return False
    host = host.lower().rstrip(".")
    for d in domains:
        d = d.lower().strip().lstrip(".")
        if d and (host == d or host.endswith("." + d)):
            return True
    return False


def _host_is_deny(host: str) -> bool:
    return _host_in(host, active_config()["deny_domains"])


def _host_is_paywall(host: str, paywall_domains: set[str] | None = None) -> bool:
    domains = paywall_domains if paywall_domains is not None else active_config()["paywall_domains"]
    return _host_in(host, domains)


def _text_has_paywall_marker(text: str) -> bool:
    return any(marker in text for marker in PAYWALL_MARKERS)


def _matches_promo_title(text: str) -> bool:
    if not text:
        return False
    return any(pat.search(text) for pat in PROMO_TITLE_PATTERNS)


def _google_news_article_url(rss_link: str) -> str:
    if "hl=en-US" in rss_link:
        return rss_link
    sep = "&" if "?" in rss_link else "?"
    return f"{rss_link}{sep}hl=en-US&gl=US&ceid=US:en"


def _cache_path(rss_link: str) -> str:
    digest = hashlib.sha1(rss_link.encode("utf-8")).hexdigest()
    day_dir = os.path.join(_CACHE_ROOT, date.today().isoformat())
    return os.path.join(day_dir, f"{digest}.url")


def _cache_read(rss_link: str) -> str | None:
    try:
        path = _cache_path(rss_link)
        if os.path.isfile(path):
            with open(path, encoding="utf-8") as fh:
                cached = fh.read().strip()
            return cached or None
    except Exception:
        pass
    return None


def _cache_write(rss_link: str, url: str) -> None:
    try:
        path = _cache_path(rss_link)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(url)
    except Exception:
        pass


def sweep_decode_cache(ttl_days: int = 7) -> None:
    """Delete per-day decode-cache subdirs older than ttl_days. Best-effort;
    the caller (news skill) invokes this alongside its own cache sweep so the
    decode cache does not grow one directory per day forever."""
    import shutil
    import time as _time
    try:
        if not os.path.isdir(_CACHE_ROOT):
            return
        cutoff = _time.time() - ttl_days * 86400
        for entry in os.listdir(_CACHE_ROOT):
            p = os.path.join(_CACHE_ROOT, entry)
            try:
                if os.path.isdir(p) and os.path.getmtime(p) < cutoff:
                    shutil.rmtree(p, ignore_errors=True)
            except OSError:
                pass
    except Exception:
        pass


def _extract_google_news_tokens(html: str) -> tuple[str, str, str] | None:
    article_id = re.search(r'data-n-a-id="([^"]+)"', html)
    article_ts = re.search(r'data-n-a-ts="([^"]+)"', html)
    article_sg = re.search(r'data-n-a-sg="([^"]+)"', html)
    if not (article_id and article_ts and article_sg):
        return None
    return article_id.group(1), article_ts.group(1), article_sg.group(1)


def _build_batchexecute_payload(article_id: str, article_ts: str, article_sg: str) -> str:
    inner = json.dumps(
        [
            "garturlreq",
            [
                [
                    "X",
                    "X",
                    ["X", "X"],
                    None,
                    None,
                    1,
                    1,
                    "US:en",
                    None,
                    1,
                    None,
                    None,
                    None,
                    None,
                    None,
                    0,
                    1,
                ],
                "X",
                "X",
                1,
                [1, 1, 1],
                1,
                1,
                None,
                0,
                0,
                None,
                0,
            ],
            article_id,
            article_ts,
            article_sg,
        ],
        separators=(",", ":"),
    )
    outer = json.dumps(
        [[["Fbv4je", inner, None, "generic"]]],
        separators=(",", ":"),
    )
    return urlencode({"f.req": outer})


def _parse_garturlres(response_text: str) -> str | None:
    if response_text.startswith(")]}'"):
        response_text = response_text[4:]
    match = re.search(
        r'garturlres\\?",\\"(https?://[^\\"]+)',
        response_text,
    )
    if not match:
        match = re.search(
            r'garturlres","(https?://[^"]+)"',
            response_text,
        )
    if not match:
        match = re.search(
            r'garturlres[^h]*(https?://[^\s"\\]+)',
            response_text,
        )
    return match.group(1) if match else None


def decode_google_news_url(rss_link: str, timeout: float = 5.0) -> str | None:
    try:
        cached = _cache_read(rss_link)
        if cached:
            return cached

        article_url = _google_news_article_url(rss_link)
        html = _http_get(article_url, timeout=timeout)
        tokens = _extract_google_news_tokens(html)
        if tokens is None:
            return None
        article_id, article_ts, article_sg = tokens

        payload = _build_batchexecute_payload(article_id, article_ts, article_sg)
        post_url = (
            "https://news.google.com/_/DotsSplashUi/data/batchexecute"
            "?rpcids=Fbv4je&source-path=%2Frss%2Farticles"
        )
        response_text = _http_post(post_url, payload, timeout=timeout)
        decoded = _parse_garturlres(response_text)
        if decoded:
            _cache_write(rss_link, decoded)
        return decoded
    except Exception:
        return None


def _strip_html(html: str) -> str:
    text = re.sub(r"(?is)<script[^>]*>.*?</script>", " ", html)
    text = re.sub(r"(?is)<style[^>]*>.*?</style>", " ", text)
    text = re.sub(r"(?s)<[^>]+>", " ", text)
    text = re.sub(r"\s+", " ", text)
    return text.strip()


def fetch_article_text(url: str, timeout: float = 5.0) -> dict[str, Any]:
    empty: dict[str, Any] = {
        "final_url": url,
        "host": "",
        "text": "",
        "word_count": 0,
        "truncated": False,
    }
    try:
        html, final_url = _http_get_with_url(url, timeout=timeout)
        text = _strip_html(html)
        host = urlparse(final_url).netloc  # post-redirect host for classification
        truncated = _host_is_paywall(host) or _text_has_paywall_marker(text)
        return {
            "final_url": final_url,
            "host": host,
            "text": text,
            "word_count": len(text.split()),
            "truncated": truncated,
        }
    except Exception as exc:
        result = dict(empty)
        result["error"] = type(exc).__name__
        return result


def classify_quality(item: dict[str, Any], article: dict[str, Any] | None) -> dict[str, Any]:
    """Three-way verdict from an item + (optional) fetched article.

    Returns {"action": "keep"|"drop"|"title_only", "reason": str|None}.
    - drop:       deny source/domain, or marketing/PR/listicle.
    - title_only: paywalled/truncated reputable content — render as the original
                  RSS headline + link (we cannot read the body).
    - keep:       everything else.
    Deterministic: does not depend on whether a network fetch happened to succeed.
    """
    try:
        source_name = str(item.get("source_name") or "")
        title = str(item.get("title") or "")
        host = str(article.get("host") or "") if article else ""

        # 1. Deny: low-trust / blocked source or domain → drop entirely.
        if source_name in _deny_sources() or _host_is_deny(host):
            return {"action": "drop", "reason": "deny"}

        # 2. Marketing / PR / listicle → drop ONLY on an unambiguous TITLE pattern.
        #    We intentionally do NOT drop on body text: the broad body patterns
        #    (優惠|折扣|限時) over-match ordinary business words inside legitimate
        #    articles, silently losing real news. The LLM prompt already excludes
        #    純行銷推廣, so body-level promo dropping adds risk without much gain.
        if _matches_promo_title(title):
            return {"action": "drop", "reason": "promo"}

        # 3. Paywalled / truncated → keep as the LLM's headline only.
        if _host_is_paywall(host) or (article is not None and article.get("truncated") is True):
            return {"action": "title_only", "reason": "paywalled"}

        return {"action": "keep", "reason": None}
    except Exception:
        return {"action": "keep", "reason": None}


def precheck_action(
    item: dict[str, Any],
    *,
    decode_timeout: float = 5.0,
    fetch_timeout: float = 5.0,
    fetch_cache: dict | None = None,
) -> dict[str, Any]:
    """Resolve an RSS item to a precheck verdict.

    Returns {"action": "keep"|"drop"|"title_only", "reason": str|None,
             "decoded_url": str|None}. Deterministic and defensive (never raises).

    `fetch_cache` (optional per-run dict) memoizes decode+fetch by link so the
    same item processed twice (e.g. AI Level-3 re-subdivision) hits no network.
    """
    try:
        source_name = str(item.get("source_name") or "")
        link = str(item.get("link") or "")
        decoded_url_hint = str(item.get("decoded_url") or "")

        if not decoded_url_hint and fetch_cache is not None and link in fetch_cache:
            return dict(fetch_cache[link])

        def _finish(result: dict[str, Any]) -> dict[str, Any]:
            if fetch_cache is not None and link:
                fetch_cache[link] = dict(result)
            return result

        # Deny by source name needs no network.
        if source_name in _deny_sources():
            return _finish({"action": "drop", "reason": "deny", "decoded_url": None})

        decoded_url = decoded_url_hint or decode_google_news_url(link, timeout=decode_timeout)
        if decoded_url is None:
            # Cannot resolve the publisher. Deterministic fail-open: keep.
            # (Paywall/deny can only be asserted once we know the host; an
            #  unknown host is left untouched rather than guessed.)
            return _finish({"action": "keep", "reason": "unresolved", "decoded_url": None})

        host = urlparse(decoded_url).netloc
        # Host-level deny/paywall are deterministic regardless of body fetch.
        if _host_is_deny(host):
            return _finish({"action": "drop", "reason": "deny", "decoded_url": decoded_url})
        if _host_is_paywall(host):
            return _finish({"action": "title_only", "reason": "paywalled", "decoded_url": decoded_url})

        article = fetch_article_text(decoded_url, timeout=fetch_timeout)
        if article.get("error"):
            # Body unavailable for a non-deny, non-paywall host. Still drop on an
            # unambiguous promo TITLE (the body promo check can't run); else keep.
            if _matches_promo_title(str(item.get("title") or "")):
                return _finish({"action": "drop", "reason": "promo", "decoded_url": decoded_url})
            return _finish({"action": "keep", "reason": "fetch_error", "decoded_url": decoded_url})

        verdict = classify_quality(item, article)
        return _finish({
            "action": verdict["action"],
            "reason": verdict["reason"],
            "decoded_url": decoded_url,
        })
    except Exception:
        return {"action": "keep", "reason": None, "decoded_url": None}