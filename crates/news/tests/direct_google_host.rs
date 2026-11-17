//! A direct feed item whose link is hosted by Google News is not a publisher
//! item (spec §2, §5): it would send precheck — or the reader — back to
//! Google. Dropped at ingestion as `invalid_link`. Pure: no network, no HOME.

use jiff::Timestamp;
use news::direct::{select_entries, FeedCounts, FeedKind, FeedSpec};
use news::feed::FeedEntry;

fn now() -> Timestamp {
    "2026-11-17T08:50:00Z".parse().unwrap()
}

fn spec() -> FeedSpec {
    FeedSpec {
        name: "Feed".into(),
        url: "https://example.com/feed".into(),
        kind: FeedKind::Plain,
        max_items: 10,
        naive_date_offset: None,
    }
}

fn entry(link: &str) -> FeedEntry {
    FeedEntry {
        title: "Headline".into(),
        link: link.into(),
        date: "Tue, 17 Nov 2026 08:00:00 GMT".into(),
        description: String::new(),
    }
}

/// `(kept links, counts)` for a feed holding just `link`.
fn select(link: &str) -> (Vec<String>, FeedCounts) {
    let (items, counts) = select_entries(&spec(), vec![entry(link)], 30.0, now());
    (items.into_iter().map(|i| i.link).collect(), counts)
}

fn dropped() -> FeedCounts {
    FeedCounts {
        fetched: 1,
        invalid_link: 1,
        ..Default::default()
    }
}

fn kept() -> FeedCounts {
    FeedCounts {
        fetched: 1,
        kept: 1,
        ..Default::default()
    }
}

#[test]
fn a_google_news_link_is_an_invalid_link() {
    for link in [
        "https://news.google.com/rss/articles/CBMi-x?oc=5",
        "https://NEWS.GOOGLE.COM/rss/articles/CBMi-x",
        "https://x.news.google.com/articles/y",
        "http://news.google.com:443/rss/articles/z",
    ] {
        assert_eq!(select(link), (vec![], dropped()), "{link}");
    }
}

#[test]
fn a_host_that_only_contains_the_name_is_kept() {
    for link in [
        "https://notnews.google.com.evil.com/a",
        "https://google.com/a",
        "https://xnews.google.com/a",
    ] {
        assert_eq!(select(link), (vec![link.to_string()], kept()), "{link}");
    }
}
