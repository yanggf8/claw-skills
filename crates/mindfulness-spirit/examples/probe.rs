//! Differential probe against mindfulness-spirit/scripts/run.py.

use mindfulness_spirit::material::*;
use std::io::BufRead;

fn main() {
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let c: serde_json::Value = serde_json::from_str(&line).unwrap();
        let v = c["v"].as_str().unwrap_or("");
        let out = match c["kind"].as_str().unwrap() {
            "is_chinese" => serde_json::json!(is_chinese(v)),
            "url" => serde_json::json!(search_url(v)),
            "quote" => serde_json::json!(quote(v)),
            "feed" => serde_json::json!(parse_feed(v, 5)
                .iter()
                .map(|(t, l, s)| serde_json::json!([t, l, s]))
                .collect::<Vec<_>>()),
            "render" => {
                let items: Vec<Item> = c["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .map(|(i, x)| Item {
                        id: i + 1,
                        title: x[0].as_str().unwrap().into(),
                        url: x[1].as_str().unwrap().into(),
                        source: x[2].as_str().unwrap().into(),
                    })
                    .collect();
                serde_json::json!([prompt_items(&items), material_text(&items)])
            }
            other => panic!("unknown {other}"),
        };
        println!("{}", serde_json::to_string(&out).unwrap());
    }
}
