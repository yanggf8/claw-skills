//! Feed parsing, URL construction, and the quality verdict.

use news::feed::*;
use news::quality::*;
use std::collections::HashSet;

// ── URL construction ─────────────────────────────────────────────────────────

#[test]
fn a_topic_is_percent_encoded_but_a_slash_is_not() {
    // `quote` leaves `/` alone; encoding it would send a different search to
    // Google than the Python does.
    assert_eq!(quote("台積電"), "%E5%8F%B0%E7%A9%8D%E9%9B%BB");
    assert_eq!(quote("a/b"), "a/b");
    assert_eq!(quote("a b"), "a%20b");
    assert_eq!(quote("~_.-"), "~_.-");
}

#[test]
fn a_topic_url_carries_the_one_day_window_and_the_taiwan_locale() {
    assert_eq!(
        topic_feed_url("AI"),
        "https://news.google.com/rss/search?q=AI+when:1d&hl=zh-TW&gl=TW&ceid=TW:zh-Hant"
    );
}

#[test]
fn every_built_in_feed_resolves() {
    for (name, _) in FEEDS {
        assert!(feed_url(name).is_some(), "{name}");
    }
    assert!(feed_url("nope").is_none());
}

// ── URL splitting ────────────────────────────────────────────────────────────

#[test]
fn the_host_component_keeps_its_userinfo_and_port() {
    // It is compared against the paywall list, and dropping either would
    // silently change which items are classified as gated.
    let (host, path, query) =
        split_url("https://user:pw@Host.EXAMPLE.com:8443/p?q=1#frag").unwrap();
    assert_eq!(host, "user:pw@Host.EXAMPLE.com:8443");
    assert_eq!(path, "/p");
    assert_eq!(query, "q=1");
}

#[test]
fn something_that_is_not_a_url_has_no_host() {
    assert!(split_url("notaurl").is_none());
}

#[test]
fn a_bing_click_tracker_is_unwrapped_to_the_real_article() {
    // Left wrapped, every replacement link would point at bing.com and the
    // same-publisher check would compare the wrong host.
    let item = news::text::Item {
        link: "https://www.bing.com/news/apiclick.aspx?ref=x&url=https%3A%2F%2Freal.example.com%2Fa%3Fb%3D1".into(),
        ..Default::default()
    };
    let out = normalize_replacement_candidate(item);
    assert_eq!(out.link, "https://real.example.com/a?b=1");
    assert_eq!(out.decoded_url.as_deref(), Some("https://real.example.com/a?b=1"));
}

#[test]
fn a_host_merely_ending_in_bing_dot_com_is_also_unwrapped() {
    // Recording the boundary, not endorsing it: the check is `ends_with`, so
    // `notbing.com` passes it. Harmless today because the only source of these
    // links is Bing itself, and both implementations behave this way — but it
    // is a suffix check pretending to be a host check.
    let item = news::text::Item {
        link: "https://notbing.com/news/apiclick.aspx?url=https%3A%2F%2Fx.com".into(),
        ..Default::default()
    };
    assert_eq!(normalize_replacement_candidate(item).link, "https://x.com");
}

#[test]
fn a_link_that_is_not_a_click_tracker_is_left_exactly_as_it_was() {
    for link in [
        "https://www.bing.com/news/other.aspx?url=https%3A%2F%2Fx.com",
        "https://bing.com/news/apiclick.aspx?url=",
    ] {
        let item = news::text::Item {
            link: link.into(),
            ..Default::default()
        };
        let out = normalize_replacement_candidate(item);
        assert_eq!(out.link, link);
        assert_eq!(out.decoded_url, None);
    }
}

// ── RSS ──────────────────────────────────────────────────────────────────────

const FEED: &str = r#"<?xml version="1.0"?><rss><channel>
<item><title>Alpha &amp; Beta - Reuters</title><link>https://a/1</link><pubDate>Mon, 01 Jan 2026</pubDate></item>
<item><title><![CDATA[Gamma &amp; Delta - AP]]></title><link>https://a/2</link></item>
<item><title>   </title><link>https://a/3</link></item>
<item><link>https://a/4</link></item>
<item><title>Has &#39;quote&#39; - X</title><link>https://a/5</link><pubDate> spaced </pubDate></item>
</channel></rss>"#;

#[test]
fn entities_in_a_title_are_resolved() {
    let items = parse_rss(FEED, 15);
    assert_eq!(items[0].title, "Alpha & Beta - Reuters");
    assert_eq!(items[0].source, "Reuters");
    assert_eq!(items[0].pub_date, "Mon, 01 Jan 2026");
}

#[test]
fn cdata_is_taken_literally() {
    // Unescaping it would turn a headline that genuinely contains `&amp;`
    // into one containing a bare ampersand.
    let items = parse_rss(FEED, 15);
    assert_eq!(items[1].title, "Gamma &amp; Delta - AP");
}

#[test]
fn an_item_with_no_usable_title_is_skipped() {
    // Both the whitespace-only title and the missing one.
    let links: Vec<&str> = parse_rss(FEED, 15).iter().map(|i| i.link.clone()).map(|s| Box::leak(s.into_boxed_str()) as &str).collect();
    assert!(!links.contains(&"https://a/3"));
    assert!(!links.contains(&"https://a/4"));
}

#[test]
fn numeric_entities_are_resolved_and_fields_are_trimmed() {
    let items = parse_rss(FEED, 15);
    let last = items.last().unwrap();
    assert_eq!(last.title, "Has 'quote' - X");
    assert_eq!(last.pub_date, "spaced");
}

#[test]
fn the_item_cap_is_honoured() {
    assert_eq!(parse_rss(FEED, 2).len(), 2);
}

#[test]
fn a_document_that_is_not_xml_yields_nothing_rather_than_failing() {
    assert!(parse_rss("not xml at all", 15).is_empty());
}

#[test]
fn a_truncated_document_keeps_whatever_parsed_before_the_break() {
    let items = parse_rss(
        "<rss><channel><item><title>unclosed</title><link>https://a/9</link></channel></rss>",
        15,
    );
    // The item never closes, so it never completes — matching the Python,
    // where the parse error is swallowed after the loop.
    assert!(items.is_empty());
}

// ── quality ──────────────────────────────────────────────────────────────────

#[test]
fn a_host_matches_its_domain_and_its_subdomains_but_not_a_lookalike() {
    let domains: HashSet<String> = ["ft.com".to_string()].into();
    assert!(host_in("ft.com", &domains));
    assert!(host_in("www.ft.com", &domains));
    assert!(host_in("FT.COM", &domains));
    assert!(host_in("ft.com.", &domains));
    // Substring matching would catch this one, which is the bug the suffix
    // rule exists to avoid.
    assert!(!host_in("craft.com", &domains));
    assert!(!host_in("", &domains));
}

#[test]
fn a_domain_entry_may_carry_a_leading_dot_or_stray_spaces() {
    assert!(host_in("ft.com", &[".ft.com".to_string()].into()));
    assert!(host_in("ft.com", &[" ft.com ".to_string()].into()));
    assert!(!host_in("ft.com", &["".to_string()].into()));
}

#[test]
fn only_anchored_promotional_markers_match() {
    assert!(matches_promo_title("5 tips for AI"));
    assert!(matches_promo_title("10  WAYS to win"));
    assert!(matches_promo_title("Press Release: x"));
    assert!(matches_promo_title("立即購買"));
    assert!(matches_promo_title("立即选购"));
}

#[test]
fn the_promo_pattern_covers_only_the_three_phrasings_it_lists() {
    // 立即選購 — traditional 選 with traditional 購 — is not among them, and
    // neither is 立即购买. Widening the list is a behaviour change, so the gap
    // is pinned rather than quietly closed.
    assert!(!matches_promo_title("立即選購"));
    assert!(!matches_promo_title("立即购买"));
}

#[test]
fn ordinary_business_words_are_not_promotional() {
    // 限時 appears in real news (限時降息); a bare substring rule would drop it.
    assert!(!matches_promo_title("限時降息"));
    assert!(!matches_promo_title("5tips"));
    assert!(!matches_promo_title("things 3"));
    assert!(!matches_promo_title(""));
}

#[test]
fn html_is_reduced_to_its_text_with_scripts_and_styles_removed() {
    assert_eq!(strip_html("<script>var x=1;</script>body"), "body");
    assert_eq!(strip_html("<SCRIPT\nsrc=a>x</SCRIPT>keep"), "keep");
    assert_eq!(strip_html("<style>a{}</style>text"), "text");
    assert_eq!(strip_html("a\n\n  b\t c"), "a b c");
    assert_eq!(strip_html("  <b>x</b>  "), "x");
}

#[test]
fn the_decoded_article_url_is_read_from_the_response() {
    // Three patterns, because the escaping depth varies with how the payload
    // came back.
    assert_eq!(
        parse_garturlres(
            ")]}'\n[[\"wrb.fr\",\"Fbv4je\",\"[\\\"garturlres\\\",\\\"https://example.com/a?b=1\\\"]\"]]"
        )
        .as_deref(),
        Some("https://example.com/a?b=1")
    );
    assert_eq!(
        parse_garturlres(r#"[["garturlres","https://plain.example.com/x"]]"#).as_deref(),
        Some("https://plain.example.com/x")
    );
    assert_eq!(parse_garturlres("nothing here"), None);
}

#[test]
fn a_paywall_marker_in_the_body_is_recognised_in_both_scripts() {
    assert!(text_has_paywall_marker("請登入後繼續閱讀"));
    assert!(text_has_paywall_marker("please continue reading"));
    assert!(!text_has_paywall_marker("一般內文"));
}

#[test]
fn an_item_with_no_article_and_no_deny_entry_is_kept() {
    // The lists ship empty, so nothing is denied until an operator says so.
    assert_eq!(classify_quality("Reuters", "普通標題", None), (Action::Keep, None));
}

#[test]
fn a_promotional_title_is_dropped_even_with_no_body() {
    assert_eq!(
        classify_quality("Reuters", "5 tips for better AI", None),
        (Action::Drop, Some("promo"))
    );
}

#[test]
fn a_truncated_body_makes_the_item_headline_only() {
    let article = Article {
        truncated: true,
        ..Default::default()
    };
    assert_eq!(
        classify_quality("Reuters", "普通標題", Some(&article)),
        (Action::TitleOnly, Some("paywalled"))
    );
}

// ── replacement publisher check ──────────────────────────────────────────────

#[test]
fn two_hosts_of_one_publisher_are_recognised_as_the_same() {
    use news::precheck::same_registered_domain;
    assert!(same_registered_domain("www.nytimes.com", "cn.nytimes.com"));
    assert!(same_registered_domain("nytimes.com:443", "nytimes.com"));
    assert!(!same_registered_domain("nytimes.com", "reuters.com"));
    assert!(!same_registered_domain("", "reuters.com"));
}
