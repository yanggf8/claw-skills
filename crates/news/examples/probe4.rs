//! Differential probe for selection, cross-dedup, and theming.

use news::crossdedup as cd;
use news::render::Numbered;
use news::select::{self, NumberedMap};
use news::text::Item;
use news::theme;
use std::collections::BTreeMap;
use std::io::BufRead;

fn numbered_from(v: &serde_json::Value) -> NumberedMap {
    let mut m = NumberedMap::new();
    for (k, val) in v.as_object().unwrap() {
        m.insert(
            k.parse().unwrap(),
            Numbered {
                title: val["title"].as_str().unwrap_or("").to_string(),
                link: val["link"].as_str().unwrap_or("").to_string(),
                source_name: val["source_name"].as_str().unwrap_or("").to_string(),
            },
        );
    }
    m
}

fn lines_of(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn blocks_json(blocks: &[theme::Block]) -> serde_json::Value {
    serde_json::json!(blocks
        .iter()
        .map(|b| serde_json::json!({
            "idx": b.idx, "start": b.start, "end": b.end,
            "headline": b.headline,
            "original_headline": b.original_headline,
            "access": b.access
        }))
        .collect::<Vec<_>>())
}

fn main() {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let c: serde_json::Value = serde_json::from_str(&line).unwrap();
        let kind = c["kind"].as_str().unwrap();
        let out = match kind {
            "pick_min" => serde_json::json!(select::parse_pick_min(c["v"].as_str())),
            "hints" => {
                let n = numbered_from(&c["numbered"]);
                let pairs = select::dedup_pair_hints(&n, c["overlap"].as_u64().unwrap() as usize);
                serde_json::json!(pairs
                    .iter()
                    .map(|(a, b, ov)| serde_json::json!([a, b, ov]))
                    .collect::<Vec<_>>())
            }
            "hint_block" => {
                let pairs: Vec<(u32, u32, usize)> = c["pairs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| {
                        (
                            p[0].as_u64().unwrap() as u32,
                            p[1].as_u64().unwrap() as u32,
                            p[2].as_u64().unwrap() as usize,
                        )
                    })
                    .collect();
                serde_json::json!(select::format_dedup_hint_block(&pairs))
            }
            "number_items" => {
                let all: Vec<(String, Vec<Item>)> = c["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|entry| {
                        (
                            entry[0].as_str().unwrap().to_string(),
                            entry[1]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|it| Item {
                                    title: it["title"].as_str().unwrap_or("").to_string(),
                                    link: it["link"].as_str().unwrap_or("").to_string(),
                                    source: it["source_name"].as_str().unwrap_or("").to_string(),
                                    ..Default::default()
                                })
                                .collect(),
                        )
                    })
                    .collect();
                let limit = c["limit"].as_u64().map(|v| v as usize);
                let (numbered, raw) = select::number_items_for_prompt(&all, None, &|_| limit);
                let n: BTreeMap<String, serde_json::Value> = numbered
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.to_string(),
                            serde_json::json!({"title": v.title, "link": v.link,
                                               "source_name": v.source_name}),
                        )
                    })
                    .collect();
                serde_json::json!([n, raw])
            }
            "post_dedup" => {
                let n = numbered_from(&c["numbered"]);
                serde_json::json!(select::post_dedup_selected_summary(
                    c["v"].as_str().unwrap(),
                    &n,
                    "probe",
                    c["overlap"].as_u64().unwrap() as usize,
                    c["pick_min"].as_u64().map(|v| v as u32),
                ))
            }
            "parse_blocks" => match theme::parse_ai_blocks(&lines_of(&c["lines"])) {
                Some(b) => blocks_json(&b),
                None => serde_json::Value::Null,
            },
            "strip_bullet" => serde_json::json!(theme::strip_bullet_text(c["v"].as_str().unwrap())),
            "theme_parse" => {
                match theme::parse_theme_response(
                    c["v"].as_str().unwrap(),
                    c["blocks"].as_u64().unwrap() as usize,
                ) {
                    Some(m) => serde_json::json!(m
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.clone()))
                        .collect::<BTreeMap<_, _>>()),
                    None => serde_json::Value::Null,
                }
            }
            "theme_layout" => {
                let blocks = theme::parse_ai_blocks(&lines_of(&c["lines"])).unwrap();
                let labels: BTreeMap<usize, String> = c["labels"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.parse().unwrap(), v.as_str().unwrap().to_string()))
                    .collect();
                let plan = theme::theme_layout_plan(&blocks, &labels);
                serde_json::json!({
                    "headed": plan.headed,
                    "tail": plan.tail,
                    "groups": plan.groups.iter().map(|(k, v)| (k.to_string(), v.clone()))
                        .collect::<BTreeMap<_, _>>(),
                    "placement": plan.placement.iter().map(|(k, v)| (k.to_string(), *v))
                        .collect::<BTreeMap<_, _>>(),
                })
            }
            "theme_render" => {
                let lines = lines_of(&c["lines"]);
                let blocks = theme::parse_ai_blocks(&lines).unwrap();
                let labels: BTreeMap<usize, String> = c["labels"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.parse().unwrap(), v.as_str().unwrap().to_string()))
                    .collect();
                serde_json::json!(theme::theme_render(&lines, &blocks, &labels).unwrap_or(lines))
            }
            "cd_parse" => match cd::parse_cross_dedup_response(
                c["v"].as_str().unwrap(),
                c["blocks"].as_u64().unwrap() as usize,
            ) {
                Some(g) => serde_json::json!(g
                    .iter()
                    .map(|x| serde_json::json!({"members": x.members, "keep": x.keep}))
                    .collect::<Vec<_>>()),
                None => serde_json::Value::Null,
            },
            "cd_votes" => {
                let samples: Vec<Vec<cd::Group>> = c["samples"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| {
                        s.as_array()
                            .unwrap()
                            .iter()
                            .map(|g| cd::Group {
                                members: g["members"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|m| m.as_u64().unwrap() as usize)
                                    .collect(),
                                keep: g["keep"].as_u64().unwrap() as usize,
                            })
                            .collect()
                    })
                    .collect();
                let votes = cd::pair_votes(&samples);
                serde_json::json!(votes
                    .iter()
                    .map(|((a, b), n)| serde_json::json!([a, b, n]))
                    .collect::<Vec<_>>())
            }
            "cd_components" => {
                let pairs: Vec<(usize, usize)> = c["pairs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| (p[0].as_u64().unwrap() as usize, p[1].as_u64().unwrap() as usize))
                    .collect();
                serde_json::json!(cd::components(
                    &pairs,
                    c["blocks"].as_u64().unwrap() as usize
                ))
            }
            "cd_apply" => {
                let lines = lines_of(&c["lines"]);
                let blocks = theme::parse_ai_blocks(&lines).unwrap();
                let groups: Vec<cd::Group> = c["groups"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|g| cd::Group {
                        members: g["members"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|m| m.as_u64().unwrap() as usize)
                            .collect(),
                        keep: g["keep"].as_u64().unwrap() as usize,
                    })
                    .collect();
                match cd::apply_cross_dedup(&lines, &blocks, &groups) {
                    Some(v) => serde_json::json!(v),
                    None => serde_json::Value::Null,
                }
            }
            "cd_survivor" => {
                let lines = lines_of(&c["lines"]);
                let blocks = theme::parse_ai_blocks(&lines).unwrap();
                let members: Vec<usize> = c["members"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|m| m.as_u64().unwrap() as usize)
                    .collect();
                serde_json::json!(cd::survivor(&members, &blocks))
            }
            other => panic!("unknown kind {other}"),
        };
        println!("{}", serde_json::to_string(&out).unwrap());
    }
}
