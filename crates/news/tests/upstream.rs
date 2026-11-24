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

// ── the replacement search is routed by the query's script ───────────────────

#[test]
fn a_chinese_headline_searches_the_taiwanese_edition_only() {
    // An edition is a language filter, not a region preference: measured
    // 2026-09-16, a Chinese query returned 0 items in en-US in 8 of 8 cases.
    assert_eq!(search_editions("台積電2奈米提前量產"), vec![GN_EDITION_TW]);
    assert_eq!(
        search_markets("台積電2奈米提前量產"),
        vec!["zh-TW".to_string()]
    );
}

#[test]
fn an_english_headline_searches_both_editions_taiwan_second() {
    // en-US is where the English coverage is, and zh-TW is where the
    // *Chinese rewrite* of the same story is — the cross-language replacement
    // the judge exists to accept. Dropping zh-TW would retire that path.
    assert_eq!(
        search_editions("U.S. Pressures Mexico to Box Out China AI Hardware Exports"),
        vec![GN_EDITION_US, GN_EDITION_TW]
    );
    assert_eq!(
        search_markets("Nvidia reports record datacenter revenue"),
        vec!["en-US".to_string(), "zh-TW".to_string()]
    );
}

#[test]
fn a_chinese_suffix_does_not_make_an_english_headline_chinese() {
    // The suffix is not part of the headline; treating it as one would send
    // this query to zh-TW, which returns nothing for it.
    assert_eq!(
        search_editions("Nvidia beats earnings expectations - 自由時報"),
        vec![GN_EDITION_US, GN_EDITION_TW]
    );
}

#[test]
fn a_search_url_names_the_edition_it_was_given() {
    assert_eq!(
        search_feed_url("AI", GN_EDITION_TW),
        "https://news.google.com/rss/search?q=AI+when:1d&hl=zh-TW&gl=TW&ceid=TW:zh-Hant"
    );
    assert_eq!(
        search_feed_url("AI", GN_EDITION_US),
        "https://news.google.com/rss/search?q=AI+when:1d&hl=en-US&gl=US&ceid=US:en"
    );
}

#[test]
fn the_bing_market_argument_is_what_the_url_carries() {
    // No env here: `NEWS_PAYWALL_REPLACE_BING_MKT` is process-wide, so its
    // override lives in its own binary (`tests/bing_market.rs`), the same way
    // the deny list does. This file stays pure URL construction.
    let en = bing_news_feed_url("Nvidia reports record revenue", "en-US");
    assert!(en.contains("mkt=en-US"), "{en}");
    let zh = bing_news_feed_url("台積電2奈米提前量產", "zh-TW");
    assert!(zh.contains("mkt=zh-TW"), "{zh}");
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
    assert_eq!(
        out.decoded_url.as_deref(),
        Some("https://real.example.com/a?b=1")
    );
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
    let links: Vec<&str> = parse_rss(FEED, 15)
        .iter()
        .map(|i| i.link.clone())
        .map(|s| Box::leak(s.into_boxed_str()) as &str)
        .collect();
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

// Captured 2026-11-24. Google escapes `=` and `&` inside the inner JSON string,
// so on the wire they are `\\u003d` and `\\u0026`.
const GARTURL_BRACES: &str = include_str!("fixtures/garturlres-braces.txt");
const GARTURL_AMPERSANDS: &str = include_str!("fixtures/garturlres-ampersands.txt");
const GARTURL_SINGLE_PARAM: &str = include_str!("fixtures/garturlres-single-param.txt");

/// A response carrying `url` exactly as the inner JSON would write it, so a
/// `\u003d` given here reaches the wire as `\\u003d`.
fn garturl_response(url: &str) -> String {
    let inner = format!(r#"["garturlres","{url}",1]"#);
    let inner = serde_json::to_string(&inner).unwrap();
    format!(
        ")]}}'\n\n[[\"wrb.fr\",\"Fbv4je\",{inner},null,null,null,\"generic\"],[\"di\",14],[\"af.httprm\",13,\"-1\",8]]"
    )
}

fn parsed(response: &str) -> Option<String> {
    parse_garturlres(response)
}

fn url(s: &str) -> Option<String> {
    Some(s.to_string())
}

#[test]
fn a_captured_response_keeps_its_query_string() {
    assert_eq!(
        parsed(GARTURL_BRACES),
        url("https://www.moneydj.com/funddj/ya/yp050000.djhtm?a={9FEE3D36-4B6F-42A0-BEA3-276241743873}")
    );
    assert_eq!(
        parsed(GARTURL_AMPERSANDS),
        url("https://www.digitimes.com.tw/tech/dt/n/shwnws.asp?cnlid=13&id=0000775464_3D13JVQ87E22T53LCHY1S&cat=30")
    );
    assert_eq!(
        parsed(GARTURL_SINGLE_PARAM),
        url("https://www.tristarnews.com.tw/news_ii.html?ID=39275")
    );
}

#[test]
fn the_captured_wire_shape_is_what_the_helper_builds() {
    // Every test below leans on the helper, so it has to write the bytes
    // Google does, not merely something the parser accepts.
    let built = garturl_response(r"https://www.tristarnews.com.tw/news_ii.html?ID\u003d39275");
    assert_eq!(
        built.lines().nth(2).unwrap().split(",null").next(),
        GARTURL_SINGLE_PARAM.lines().nth(2).unwrap().split(",null").next()
    );
}

#[test]
fn every_json_escape_in_the_article_url_is_decoded() {
    for (inner, want) in [
        (r"https://a.example/x?p\u003d1\u0026q\u003d2", "https://a.example/x?p=1&q=2"),
        (r"https:\/\/a.example\/x\/y", "https://a.example/x/y"),
        (r"https://a.example/\u65b0\u805e/1", "https://a.example/新聞/1"),
        (r"https://a.example/\ud83d\ude80?k\u003dv", "https://a.example/🚀?k=v"),
        ("https://a.example/新聞?k=v", "https://a.example/新聞?k=v"),
        ("HTTPS://A.example/x", "HTTPS://A.example/x"),
        ("http://a.example", "http://a.example"),
    ] {
        assert_eq!(parsed(&garturl_response(inner)), url(want), "{inner}");
    }
}

/// Rows that only look like the answer to a text search: the name inside
/// another cell's string, and in rows that answer some other call.
const DECOYS: &str = r#"["x","garturlres\",\"https://wrong.example/1\""],["wrb.fr","Other","[\"garturlres\",\"https://wrong.example/2\"]"],["er","Fbv4je","[\"garturlres\",\"https://wrong.example/3\"]"]"#;

#[test]
fn an_article_url_is_found_wherever_the_envelope_puts_it() {
    for (what, row) in [
        ("stringified", r#"["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x?p\\u003d1\",1]",null]"#),
        ("more inside the string", r#"["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x?p\\u003d1\"] tail"]"#),
        ("an array", r#"["wrb.fr","Fbv4je",["garturlres","https://a.example/x?p\u003d1",1]]"#),
        ("the row itself", r#"["garturlres","https://a.example/x?p\u003d1"]"#),
    ] {
        for response in [
            format!("[{row}]"),
            format!("[{DECOYS},{row}]"),
            format!(")]}}'\n\n[{DECOYS},{row}]"),
            // Length-prefixed chunks, which is how the endpoint streams.
            format!(")]}}'\n\n104\n[{DECOYS}]\n25\n[{row}]\n9\n"),
            format!("[{DECOYS},{row},[\"garturlres\",\"https://wrong.example/4\"]]"),
        ] {
            assert_eq!(parsed(&response), url("https://a.example/x?p=1"), "{what}: {response}");
        }
    }
}

#[test]
fn data_after_the_envelope_does_not_lose_the_article_url() {
    for tail in ["\n25\n[[\"e\",4]]", "garbage", "]]]", "\n\"garturlres\",\"https://wrong.example/\""] {
        let response = format!("{GARTURL_SINGLE_PARAM}{tail}");
        assert_eq!(
            parsed(&response),
            url("https://www.tristarnews.com.tw/news_ii.html?ID=39275"),
            "{tail}"
        );
    }
}

#[test]
fn a_truncated_response_never_yields_a_shortened_url() {
    // Every prefix of a real capture is either undecodable or the whole URL.
    let full = "https://www.digitimes.com.tw/tech/dt/n/shwnws.asp?cnlid=13&id=0000775464_3D13JVQ87E22T53LCHY1S&cat=30";
    let complete = GARTURL_AMPERSANDS.find(r#"30\""#).unwrap() + r#"30\""#.len();
    for end in 0..=GARTURL_AMPERSANDS.len() {
        let want = (end >= complete).then(|| full.to_string());
        assert_eq!(parsed(&GARTURL_AMPERSANDS[..end]), want, "cut at {end}");
    }
}

#[test]
fn a_url_that_cannot_be_decoded_whole_is_refused() {
    for inner in [
        // Half of a surrogate pair, and escapes JSON does not have.
        r"https://a.example/x?p\ud83d",
        r"https://a.example/x?p\ud83dx",
        r"https://a.example/x?p\u00zz",
        r"https://a.example/x?p\q",
        // Escaped once more than the shape says: still escaped when read.
        r"https://a.example/x?p\\u003d1",
        // Nothing a URL contains, and both are only reachable by an escape.
        r"https://a.example/x?p\n1",
        r"https://a.example/x?p\u00001",
    ] {
        assert_eq!(parsed(&garturl_response(inner)), None, "{inner}");
    }
}

#[test]
fn a_decoded_value_that_is_not_an_http_url_is_refused() {
    for inner in [
        "ftp://a.example/x",
        "javascript:alert(1)",
        r"javascript:alert(\u0027https://a.example/x\u0027)",
        "httpsx://a.example/x",
        "/relative/path",
        "https://",
        "https:///path",
        "",
    ] {
        assert_eq!(parsed(&garturl_response(inner)), None, "{inner}");
    }
    // A payload that was read and holds no usable URL is the answer. Nothing
    // else in the response may stand in for it.
    for row in [
        r#"["garturlres","ftp://a.example/x","https://wrong.example/"]"#,
        r#"["garturlres",null,"https://wrong.example/"]"#,
        r#"["garturlres",1,"https://wrong.example/"]"#,
        r#"["garturlres"],["x","https://wrong.example/"]"#,
        r#"["garturlres",null],["x","garturlres\",\"https://wrong.example/\""]"#,
    ] {
        assert_eq!(parsed(&format!("[{row}]")), None, "{row}");
    }
}

#[test]
fn an_unreadable_envelope_still_yields_a_whole_url() {
    let want = url("https://a.example/x?p=1&q=新");
    for (what, response) in [
        ("cut after the payload", r#"[["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x?p\\u003d1\\u0026q\\u003d\\u65b0\""#),
        ("inner quotes not escaped", r#"[["wrb.fr","Fbv4je","["garturlres","https://a.example/x?p\u003d1\u0026q\u003d\u65b0"]"]]"#),
        ("only some quotes escaped", r#"[["wrb.fr","Fbv4je","[\"garturlres",\"https://a.example/x?p\\u003d1\\u0026q\\u003d\\u65b0\",1]"]]"#),
        ("text ahead of the envelope", r#"oops [["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x?p\\u003d1\\u0026q\\u003d\\u65b0\",1]"]]"#),
        ("an inner payload left open", r#"[["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x?p\\u003d1\\u0026q\\u003d\\u65b0\","]]"#),
        ("one more level of escaping", r#"[["wrb.fr","Fbv4je","\"[\\\"garturlres\\\",\\\"https://a.example/x?p\\\\u003d1\\\\u0026q\\\\u003d\\\\u65b0\\\",1]\""]"#),
        ("spaced", r#"[["garturlres" , "https://a.example/x?p\u003d1\u0026q\u003d\u65b0" "#),
    ] {
        assert_eq!(parsed(response), want, "{what}");
    }
}

#[test]
fn an_unreadable_envelope_never_yields_part_of_a_url() {
    for (what, response) in [
        ("cut in an escape", r#"[["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x?p\\u00"#),
        ("cut after a backslash", r#"[["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x?p\"#),
        ("cut before the closing quote", r#"[["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x"#),
        ("the outer string ends first", r#"[["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x"],["y\""#),
        ("the outer string ends behind a backslash", r#"[["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x\\"],["y\""#),
        ("half a surrogate pair", r#"[["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x\\ud83d\""#),
        ("escaped deeper than its quotes", r#"["garturlres","https://a.example/x?p\\u003d1""#),
        ("quotes no JSON depth produces", r#"[\\"garturlres\\",\\"https://a.example/x\\""#),
        ("not http", r#"[["wrb.fr","Fbv4je","[\"garturlres\",\"ftp://a.example/x\""#),
        ("the URL is not next", r#"[["wrb.fr","Fbv4je","[\"garturlres\",null,\"https://wrong.example/\""#),
        ("the first answer is the only one", r#"["garturlres","https://a.example/x?p\q"],["garturlres","https://wrong.example/""#),
    ] {
        assert_eq!(parsed(response), None, "{what}");
    }
    // An escaped quote is part of the URL, not the end of it.
    assert_eq!(
        parsed(r#"["garturlres","https://a.example/x?p\""#),
        None
    );
    assert_eq!(
        parsed(r#"["garturlres","https://a.example/x?p\"""#),
        url(r#"https://a.example/x?p""#)
    );
    assert_eq!(
        parsed(r#"["garturlres","https://a.example/x?p\"q""#),
        url(r#"https://a.example/x?p"q"#)
    );
    assert_eq!(
        parsed(r#"[["wrb.fr","Fbv4je","[\"garturlres\",\"https://a.example/x?p\\\"q\""#),
        url(r#"https://a.example/x?p"q"#)
    );
}

#[test]
fn a_response_with_no_article_url_is_none_and_never_a_panic() {
    let deep = "[".repeat(100_000);
    let slashes = format!("garturlres\",{}\"https://a.example/x", "\\".repeat(70_000));
    for response in [
        "", " ", ")]}'", ")]}'\n\n", "null", "0", "[]", "[[]]", "{}", "[[null]]", "[[\"wrb.fr\"]]",
        "[[\"wrb.fr\",\"Fbv4je\"]]",
        "[[\"wrb.fr\",\"Fbv4je\",null,null,null,[3],\"generic\"]]",
        "[[\"wrb.fr\",\"Fbv4je\",\"\"]]",
        "[[\"wrb.fr\",\"Fbv4je\",\"not json\"]]",
        "garturlres", "garturlres\"", "garturlres\",\"", "garturlres\\\",\\\"", "garturlres\",\"\"",
        "\u{feff})]}'", "新聞garturlres\",\"新聞", "<html>Sorry...</html>",
        deep.as_str(), slashes.as_str(),
    ] {
        let shown: String = response.chars().take(60).collect();
        assert_eq!(parsed(response), None, "{shown}");
    }
}

#[test]
fn an_article_url_with_braces_or_non_ascii_is_fetched_as_written() {
    // The precheck fetches the decoded URL. Braces stay raw in a query, as a
    // browser sends them; non-ASCII goes out percent-encoded.
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let targets = [
        ("/yp050000.djhtm?a={9FEE3D36-4B6F}", "/yp050000.djhtm?a={9FEE3D36-4B6F}"),
        ("/shwnws.asp?cnlid=13&id=7_3D&cat=30", "/shwnws.asp?cnlid=13&id=7_3D&cat=30"),
        ("/新聞?q=電", "/%E6%96%B0%E8%81%9E?q=%E9%9B%BB"),
    ];
    let server = std::thread::spawn(move || {
        (0..targets.len())
            .map(|_| {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap() > 2 {
                    line.clear();
                }
                let body = "<p>one two three</p>";
                write!(
                    &stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                request
            })
            .collect::<Vec<_>>()
    });
    for (target, _) in targets {
        let article = fetch_article_text(&format!("{base}{target}"), std::time::Duration::from_secs(5));
        assert_eq!(article.error, None, "{target}");
        assert_eq!(article.text, "one two three", "{target}");
    }
    let got = server.join().unwrap();
    let want: Vec<String> =
        targets.iter().map(|(_, wire)| format!("GET {wire} HTTP/1.1\r\n")).collect();
    assert_eq!(got, want);
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
    assert_eq!(
        classify_quality("Reuters", "普通標題", None),
        (Action::Keep, None)
    );
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

// ── request timeouts ─────────────────────────────────────────────────────────

#[test]
fn a_refused_connection_returns_at_once_rather_than_stalling() {
    // `ureq::builder().timeout(d)` does not bound the connect phase — only
    // `timeout_connect` does — so a fetch built the obvious way waits out
    // ureq's own 30-second default no matter what it was asked for. Measured
    // 2026-08-01: 30.03s for a 50ms request. The Python this was ported from
    // has no such gap, since `urlopen(timeout=N)` covers the connect.
    let started = std::time::Instant::now();
    let items = news::feed::fetch_feed(
        "http://127.0.0.1:1/rss",
        5,
        std::time::Duration::from_secs(1),
    );
    assert!(items.is_empty());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "a refused connection took {:?}; the connect phase is unbounded again",
        started.elapsed()
    );
}

#[test]
fn an_unreachable_host_is_bounded_by_the_budget_it_was_given() {
    // TEST-NET-3: reserved for documentation, routed nowhere, so the connect
    // hangs rather than being refused. That is the case the missing
    // `timeout_connect` actually cost 30 seconds on.
    let started = std::time::Instant::now();
    let items = news::feed::fetch_feed(
        "http://203.0.113.1/rss",
        5,
        std::time::Duration::from_secs(1),
    );
    assert!(items.is_empty());
    if started.elapsed() < std::time::Duration::from_millis(200) {
        // Some networks refuse instead of dropping. Say so rather than pass
        // quietly — a vacuous pass here is what let the bug live.
        eprintln!(
            "note: this host refuses TEST-NET-3 instead of blackholing it, \
             so the connect timeout was not exercised"
        );
        return;
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "a 1s budget took {:?}",
        started.elapsed()
    );
}
