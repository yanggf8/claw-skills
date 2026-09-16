//! The Bing market override.
//!
//! Its own binary because `NEWS_PAYWALL_REPLACE_BING_MKT` is process-wide and
//! `cargo test` runs the tests in one binary concurrently — the same reason the
//! deny list has `tests/deny.rs`. A stray `de-DE` seen by a neighbouring test
//! would make it pass for the wrong reason.

use news::config::paywall_replace_bing_mkt;
use news::feed::bing_news_feed_url;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn unset_means_auto_so_the_caller_supplies_the_market() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("NEWS_PAYWALL_REPLACE_BING_MKT");

    // The default used to be a hard-coded `en-US`, which silently returned
    // nothing for every Chinese query: measured 2026-09-16, the stripped
    // Chinese query scored 0 items under `en-US` and 9 under `zh-TW`.
    assert_eq!(paywall_replace_bing_mkt(), None);
    let zh = bing_news_feed_url("台積電2奈米提前量產", "zh-TW");
    assert!(zh.contains("mkt=zh-TW"), "{zh}");
}

#[test]
fn a_pinned_market_still_wins_over_the_query() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("NEWS_PAYWALL_REPLACE_BING_MKT", "de-DE");

    // Read while it is still set. The accessor reads the environment per call
    // rather than caching at first use, so asserting after the cleanup below
    // would be asserting against a different state.
    let pinned = paywall_replace_bing_mkt();
    let url = bing_news_feed_url("台積電2奈米提前量產", "zh-TW");
    std::env::remove_var("NEWS_PAYWALL_REPLACE_BING_MKT");

    // An operator who set this keeps their value: the knob is a documented
    // escape hatch, and changing the default is not licence to ignore it.
    assert_eq!(pinned.as_deref(), Some("de-DE"));
    assert!(url.contains("mkt=de-DE"), "{url}");
}

#[test]
fn a_blank_pinned_market_counts_as_unset() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("NEWS_PAYWALL_REPLACE_BING_MKT", "   ");

    let blank = paywall_replace_bing_mkt();
    let url = bing_news_feed_url("台積電2奈米提前量產", "zh-TW");
    std::env::remove_var("NEWS_PAYWALL_REPLACE_BING_MKT");

    // `..._MKT=` in a cron env is a typo, not a request for an empty market.
    assert_eq!(blank, None);
    assert!(url.contains("mkt=zh-TW"), "{url}");
}
