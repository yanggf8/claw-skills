//! Material gathering, template filling, and config resolution.

use mindfulness_spirit::config::{settings_from, DEFAULT_COLUMN_SLUG};
use mindfulness_spirit::material::{
    is_chinese, material_text, parse_feed, prompt_items, quote, search_url, Item, QUERIES,
};
use mindfulness_spirit::prompt::{checklist, render};
use serde_json::json;
use std::collections::BTreeMap;

// ── query locale ─────────────────────────────────────────────────────────────

#[test]
fn a_query_with_any_cjk_character_is_chinese() {
    assert!(is_chinese("冥想 AI"));
    assert!(is_chinese("身心靈 科技"));
    assert!(!is_chinese("mindfulness AI"));
    assert!(!is_chinese(""));
}

#[test]
fn the_locale_follows_the_query_language() {
    // Sending the Chinese half of the query list to the US locale would return
    // the same English wire copy twice over.
    assert!(search_url("冥想 AI").contains("hl=zh-TW&gl=TW&ceid=TW:zh-Hant"));
    assert!(search_url("mindfulness AI").contains("hl=en-US&gl=US&ceid=US:en"));
}

#[test]
fn the_query_list_reaches_both_language_markets() {
    // Five English, three Chinese. Pinned rather than balanced: the ratio is
    // what has accumulated, and the thing worth protecting is that neither
    // side is empty — a feed that lost its Chinese queries would quietly stop
    // surfacing the Taiwanese coverage this column is written for.
    let chinese = QUERIES.iter().filter(|q| is_chinese(q)).count();
    assert_eq!((chinese, QUERIES.len() - chinese), (3, 5));
}

#[test]
fn a_query_is_percent_encoded_the_way_urllib_does_it() {
    assert_eq!(quote("mindfulness AI"), "mindfulness%20AI");
    assert_eq!(quote("冥想 AI"), "%E5%86%A5%E6%83%B3%20AI");
    // `quote` leaves `/` alone by default; encoding it would send a different
    // search than the Python does.
    assert_eq!(quote("a/b"), "a/b");
}

// ── feed parsing ─────────────────────────────────────────────────────────────

const FEED: &str = r#"<?xml version="1.0"?><rss><channel>
<title>Channel title, not an item</title>
<item><title>Alpha &amp; Beta</title><link>https://a/1</link><source url="x">Example News</source></item>
<item><title><![CDATA[Gamma]]></title><link>https://a/2</link></item>
<item><title>No link here</title></item>
<item><link>https://a/4</link></item>
<item><title>Fifth</title><link>https://a/5</link><source>Fifth Source</source></item>
<extension><item><title>Nested, not ours</title><link>https://nested/1</link></item></extension>
</channel></rss>"#;

#[test]
fn entities_resolve_and_the_source_is_read() {
    let items = parse_feed(FEED, 5);
    assert_eq!(items[0].0, "Alpha & Beta");
    assert_eq!(items[0].1, "https://a/1");
    assert_eq!(items[0].2, "Example News");
}

#[test]
fn a_missing_source_falls_back_to_google_news() {
    let items = parse_feed(FEED, 5);
    assert_eq!(items[1].2, "Google News");
}

#[test]
fn an_item_missing_a_title_or_a_link_is_skipped_not_half_used() {
    let links: Vec<&str> = parse_feed(FEED, 5).iter().map(|i| i.1.clone()).map(|s| Box::leak(s.into_boxed_str()) as &str).collect();
    assert!(!links.contains(&"https://a/4"));
    assert_eq!(links.len(), 3);
}

#[test]
fn the_channel_title_is_not_mistaken_for_an_item() {
    // `./channel/item` is a path, not a search. A title one level up belongs
    // to the feed, not to a story.
    let titles: Vec<String> = parse_feed(FEED, 5).iter().map(|i| i.0.clone()).collect();
    assert!(!titles.iter().any(|t| t.contains("Channel title")), "{titles:?}");
}

#[test]
fn an_item_nested_in_some_other_element_is_not_material() {
    // Extensions embed their own <item>. Matching on the tag name anywhere in
    // the tree, instead of on the path, quietly turns a feed's internals into
    // things the writer is asked to cite.
    let links: Vec<String> = parse_feed(FEED, 9).iter().map(|i| i.1.clone()).collect();
    assert!(!links.iter().any(|l| l.contains("nested")), "{links:?}");
}

#[test]
fn the_per_query_limit_is_honoured() {
    assert_eq!(parse_feed(FEED, 2).len(), 2);
}

#[test]
fn rubbish_yields_no_material_rather_than_an_error() {
    assert!(parse_feed("not xml", 5).is_empty());
    assert!(parse_feed("", 5).is_empty());
}

// ── numbering and rendering ──────────────────────────────────────────────────

fn items() -> Vec<Item> {
    vec![
        Item {
            id: 1,
            title: "測試標題".into(),
            url: "https://example.com/1".into(),
            source: "Example".into(),
        },
        Item {
            id: 2,
            title: "另一則".into(),
            url: "https://example.com/2".into(),
            source: "Google News".into(),
        },
    ]
}

#[test]
fn the_prompt_list_carries_the_number_the_writer_must_cite() {
    assert_eq!(
        prompt_items(&items()),
        "#1 [Example] 測試標題\n#2 [Google News] 另一則"
    );
}

#[test]
fn the_material_tsv_maps_each_number_back_to_its_url() {
    // persona-core reads this to turn `[來源 #N]` into a real link, so the
    // number, the source and the URL must line up column for column.
    assert_eq!(
        material_text(&items()),
        "1\tExample\thttps://example.com/1\n2\tGoogle News\thttps://example.com/2\n"
    );
}

#[test]
fn the_tsv_ends_with_a_newline_even_for_one_item() {
    assert!(material_text(&items()[..1]).ends_with('\n'));
}

// ── templates ────────────────────────────────────────────────────────────────

fn slots(pairs: &[(&'static str, &str)]) -> BTreeMap<&'static str, String> {
    pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
}

#[test]
fn every_named_slot_is_filled() {
    let out = render(
        "voice={voice}\nplan={plan}",
        &slots(&[("voice", "V"), ("plan", "P")]),
    )
    .unwrap();
    assert_eq!(out, "voice=V\nplan=P");
}

#[test]
fn a_slot_the_caller_did_not_supply_is_an_error_not_a_blank() {
    // A silently-empty topic block produces an article that reads fine and is
    // no longer part of a series — which is exactly the failure that went
    // unnoticed for six weeks.
    let e = render("{topic_block}", &slots(&[])).unwrap_err();
    assert!(e.contains("topic_block"), "{e}");
}

#[test]
fn an_unbalanced_brace_is_an_error() {
    assert!(render("{unclosed", &slots(&[])).is_err());
}

#[test]
fn the_checklist_splices_the_draft_without_interpreting_it() {
    // Model output routinely contains braces; a format-style pass over it
    // would try to read them as slots.
    let out = checklist(
        "review this:\n{{WRITER_OUTPUT}}\nend",
        "draft with {braces} and {{more}}",
    );
    assert_eq!(out, "review this:\ndraft with {braces} and {{more}}\nend");
}

// ── settings ─────────────────────────────────────────────────────────────────

#[test]
fn a_persona_is_required() {
    let e = settings_from(&json!({"skills": {"mindfulness_spirit": {}}})).unwrap_err();
    assert!(e.contains("persona_slug"), "{e}");
    assert!(settings_from(&json!({})).is_err());
}

#[test]
fn the_column_comes_from_config_when_it_is_set() {
    let s = settings_from(&json!({
        "skills": {"mindfulness_spirit": {"persona_slug": "ping-w", "column_slug": "season-3"}}
    }))
    .unwrap();
    assert_eq!(s.column_slug, "season-3");
}

#[test]
fn an_unset_column_falls_back_to_the_current_season() {
    let s = settings_from(&json!({
        "skills": {"mindfulness_spirit": {"persona_slug": "ping-w"}}
    }))
    .unwrap();
    assert_eq!(s.column_slug, DEFAULT_COLUMN_SLUG);
}

#[test]
fn a_blank_column_is_treated_as_unset_rather_than_as_a_slug() {
    let s = settings_from(&json!({
        "skills": {"mindfulness_spirit": {"persona_slug": "ping-w", "column_slug": "   "}}
    }))
    .unwrap();
    assert_eq!(s.column_slug, DEFAULT_COLUMN_SLUG);
}

#[test]
fn publish_defaults_on_and_the_cover_url_is_optional() {
    let s = settings_from(&json!({
        "skills": {"mindfulness_spirit": {"persona_slug": "ping-w"}}
    }))
    .unwrap();
    assert!(s.publish);
    assert_eq!(s.main_image_url, None);
}
