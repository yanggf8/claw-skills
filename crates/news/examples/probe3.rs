//! Differential probe for the render layer.

use news::render::{self, Numbered, Replacement};
use std::collections::HashMap;
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
        let v = c["v"].as_str().unwrap_or("");
        let out = match kind {
            "strip_links" => serde_json::json!(render::strip_links_keep_spacing(v)),
            "visible" => serde_json::json!(render::markdown_visible_text(v)),
            "trim_links" => serde_json::json!(render::trim_links_to_limit(
                v,
                c["limit"].as_u64().unwrap() as usize
            )),
            "trim_lines" => serde_json::json!(render::trim_lines_to_limit(
                v,
                c["limit"].as_u64().unwrap() as usize
            )),
            "trim_digest" => serde_json::json!(render::trim_digest_links(v)),
            "split" => serde_json::json!(render::split_message_preserving_lines(
                v,
                c["limit"].as_u64().unwrap() as usize
            )),
            "md_safe" => match render::markdown_chunk_is_safe(v) {
                Ok(()) => serde_json::json!([true, ""]),
                Err(reason) => serde_json::json!([false, reason]),
            },
            "attach_links" => {
                let mut map = news::render::LinkMap::default();
                for pair in c["map"].as_array().unwrap() {
                    map.insert(
                        pair[0].as_str().unwrap().to_string(),
                        pair[1].as_str().unwrap().to_string(),
                    );
                }
                serde_json::json!(render::attach_links(v, &map))
            }
            "attach_numbered" => {
                let mut numbered: HashMap<u32, Numbered> = HashMap::new();
                for (k, val) in c["numbered"].as_object().unwrap() {
                    numbered.insert(
                        k.parse().unwrap(),
                        Numbered {
                            title: val["title"].as_str().unwrap_or("").to_string(),
                            link: val["link"].as_str().unwrap_or("").to_string(),
                            source_name: val["source_name"].as_str().unwrap_or("").to_string(),
                        },
                    );
                }
                let mut paywall: HashMap<u32, Replacement> = HashMap::new();
                if let Some(obj) = c["paywall"].as_object() {
                    for (k, val) in obj {
                        let r = &val["replacement"];
                        paywall.insert(
                            k.parse().unwrap(),
                            Replacement {
                                title_zh: r["title_zh"].as_str().unwrap_or("").to_string(),
                                link: r["link"].as_str().unwrap_or("").to_string(),
                            },
                        );
                    }
                }
                let (text, n) = render::attach_numbered_links(v, &numbered, &paywall);
                serde_json::json!([text, n])
            }
            other => panic!("unknown kind {other}"),
        };
        println!("{}", serde_json::to_string(&out).unwrap());
    }
}
