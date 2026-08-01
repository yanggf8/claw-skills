//! Differential probe: reads one JSON case per line from stdin, prints one
//! JSON result per line. The Python side runs the same corpus through
//! news/scripts/run.py and the two outputs are compared byte for byte.
//!
//! Not part of the shipped binary; it exists so parity is measured against the
//! real Python rather than against my memory of it.

use news::validate::*;
use std::collections::HashSet;
use std::io::BufRead;

fn main() {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let case: serde_json::Value = serde_json::from_str(&line).unwrap();
        let summary = case["summary"].as_str().unwrap();
        let numbered: HashSet<u32> = case["numbered"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u32)
            .collect();

        let out = serde_json::json!({
            "bullets": news_bullet_lines(summary),
            "content": content_lines(summary),
            "shape": shape_ok(summary, &numbered),
            "marker_stats": marker_stats(summary, &numbered),
            "ids": leading_marker_ids(summary, &numbered),
            "lang_stats": language_stats(summary),
            "lang": language_ok(summary),
            "neutralized": neutralize_markdown(summary),
            "stripped": summary.lines().map(strip_marker_prefix).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string(&out).unwrap());
    }
}
