//! Differential probe for the fetch/quality layer.

use news::feed;
use news::quality;
use news::text::Item;
use std::io::BufRead;

fn main() {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let c: serde_json::Value = serde_json::from_str(&line).unwrap();
        let kind = c["kind"].as_str().unwrap();
        let v = c["v"].as_str().unwrap();
        let out = match kind {
            "quote" => serde_json::json!(feed::quote(v)),
            "topic_url" => serde_json::json!(feed::topic_feed_url(v)),
            "bing_url" => serde_json::json!(feed::bing_news_feed_url(v)),
            "gnews_url" => serde_json::json!(quality::google_news_article_url_pub(v)),
            "payload" => {
                let p: Vec<&str> = v.split('\u{1}').collect();
                serde_json::json!(quality::build_batchexecute_payload_pub(p[0], p[1], p[2]))
            }
            "garturl" => serde_json::json!(quality::parse_garturlres(v)),
            "strip_html" => serde_json::json!(quality::strip_html(v)),
            "promo" => serde_json::json!(quality::matches_promo_title(v)),
            "host_in" => {
                let p: Vec<&str> = v.split('\u{1}').collect();
                let set: std::collections::HashSet<String> =
                    p[1].split(',').filter(|s| !s.is_empty()).map(String::from).collect();
                serde_json::json!(quality::host_in(p[0], &set))
            }
            "netloc" => serde_json::json!(feed::split_url(v).map(|(h, _, _)| h).unwrap_or_default()),
            "normalize" => {
                let it = Item { link: v.to_string(), ..Default::default() };
                let n = feed::normalize_replacement_candidate(it);
                serde_json::json!({"link": n.link, "decoded_url": n.decoded_url})
            }
            "rss" => {
                let items = feed::parse_rss(v, 15);
                serde_json::json!(items
                    .iter()
                    .map(|i| serde_json::json!({
                        "title": i.title, "link": i.link,
                        "pub_date": i.pub_date, "source_name": i.source
                    }))
                    .collect::<Vec<_>>())
            }
            other => panic!("unknown kind {other}"),
        };
        println!("{}", serde_json::to_string(&out).unwrap());
    }
}
