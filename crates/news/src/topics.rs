//! Per-account topic subscriptions.
//!
//! A tiny JSON file the agent edits on request. Written atomically because a
//! half-written file is indistinguishable from an empty one on the next read,
//! and that silently reverts a user's subscription to the defaults.

use crate::config::topics_file;
use serde_json::Value;
use std::io::Write;

/// Accounts in file order.
///
/// Not a `BTreeMap`: Python's `json.load` yields a dict in file order and
/// `json.dump` writes it back the same way, so a sorted map would silently
/// reshuffle a hand-edited file on the first `add`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Topics(Vec<(String, Vec<String>)>);

impl Topics {
    pub fn get(&self, account: &str) -> Option<&Vec<String>> {
        self.0.iter().find(|(k, _)| k == account).map(|(_, v)| v)
    }

    /// The account's list, appended if it was absent.
    pub fn entry(&mut self, account: &str) -> &mut Vec<String> {
        if let Some(i) = self.0.iter().position(|(k, _)| k == account) {
            return &mut self.0[i].1;
        }
        self.0.push((account.to_string(), Vec::new()));
        &mut self.0.last_mut().expect("just pushed").1
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A missing or unparseable file means "no preferences", not an error: the
/// caller falls back to the built-in AI/tech/general feeds.
pub fn load() -> Topics {
    let text = match std::fs::read_to_string(topics_file()) {
        Ok(t) => t,
        Err(_) => return Topics::default(),
    };
    parse(&text)
}

/// Reads the object through a visitor rather than into `serde_json::Value`.
///
/// `Value`'s map is a `BTreeMap` unless the whole workspace turns on
/// `preserve_order`, so going through it would sort the accounts on the way in
/// and defeat the point of the ordered `Topics` above.
impl<'de> serde::Deserialize<'de> for Topics {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct InOrder;
        impl<'de> serde::de::Visitor<'de> for InOrder {
            type Value = Topics;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object of account -> topic list")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Topics, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, Value>()? {
                    // A non-list value is skipped rather than fatal, matching
                    // the "unreadable file means no preferences" rule at the
                    // level of one account instead of the whole file.
                    if let Some(list) = v.as_array() {
                        out.push((
                            k,
                            list.iter()
                                .filter_map(|s| s.as_str().map(str::to_string))
                                .collect(),
                        ));
                    }
                }
                Ok(Topics(out))
            }
        }
        d.deserialize_map(InOrder)
    }
}

fn parse(text: &str) -> Topics {
    serde_json::from_str(text).unwrap_or_default()
}

fn render(data: &Topics) -> String {
    // Built by hand rather than through serde so the order above survives —
    // serde_json's Map sorts keys unless the whole workspace opts into
    // preserve_order, which is too wide a change for one small file.
    if data.0.is_empty() {
        return "{}\n".to_string();
    }
    let entries: Vec<String> = data
        .0
        .iter()
        .map(|(k, v)| {
            let key = Value::from(k.as_str()).to_string();
            let items: Vec<String> = v
                .iter()
                .map(|t| format!("    {}", Value::from(t.as_str())))
                .collect();
            if items.is_empty() {
                format!("  {key}: []")
            } else {
                format!("  {key}: [\n{}\n  ]", items.join(",\n"))
            }
        })
        .collect();
    format!("{{\n{}\n}}\n", entries.join(",\n"))
}

/// Write to a sibling temp file and rename over the target.
pub fn save(data: &Topics) -> std::io::Result<()> {
    let path = topics_file();
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(dir)?;
    let body = render(data);

    let tmp = dir.join(format!(".news-topics.{}.tmp", std::process::id()));
    let write = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()
    })();
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

pub fn manage_list(account: &str) -> String {
    let data = load();
    match data.get(account).filter(|t| !t.is_empty()) {
        None if account == "main" => {
            format!("📰 {account} 的新聞訂閱：AI、科技半導體、一般新聞（預設）")
        }
        None => format!("📰 {account} 尚未設定新聞主題"),
        Some(topics) => {
            let lines: Vec<String> = topics.iter().map(|t| format!("  • {t}")).collect();
            format!("📰 {account} 的新聞訂閱：\n{}", lines.join("\n"))
        }
    }
}

pub fn manage_add(account: &str, topic: &str) -> String {
    let topic = topic.trim();
    if topic.is_empty() {
        return "請提供要新增的主題名稱".into();
    }
    let mut data = load();
    let topics = data.entry(account);
    if topics.iter().any(|t| t == topic) {
        return format!("✅ 主題「{topic}」已在訂閱中");
    }
    topics.push(topic.to_string());
    let joined = topics.join("、");
    if let Err(e) = save(&data) {
        return format!("⚠️ 無法寫入主題設定：{e}");
    }
    format!("✅ 已新增主題「{topic}」\n目前訂閱：{joined}")
}

pub fn manage_remove(account: &str, topic: &str) -> String {
    let topic = topic.trim();
    if topic.is_empty() {
        return "請提供要移除的主題名稱".into();
    }
    let mut data = load();
    let topics = data.entry(account);
    if !topics.iter().any(|t| t == topic) {
        return format!("⚠️ 主題「{topic}」不在訂閱中");
    }
    topics.retain(|t| t != topic);
    let remaining = topics.join("、");
    let empty = topics.is_empty();
    if let Err(e) = save(&data) {
        return format!("⚠️ 無法寫入主題設定：{e}");
    }
    if empty {
        format!("✅ 已移除主題「{topic}」\n目前無訂閱主題（將使用預設新聞）")
    } else {
        format!("✅ 已移除主題「{topic}」\n目前訂閱：{remaining}")
    }
}

#[cfg(test)]
fn roundtrip(text: &str) -> String {
    render(&parse(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_file_reads_as_no_preferences() {
        assert!(parse("not json").is_empty());
        assert!(parse("[1,2]").is_empty());
    }

    #[test]
    fn account_order_survives_a_round_trip() {
        // Alphabetically nunu < zed < main is false; this is file order, and a
        // sorted map would reorder it.
        let src = "{\n  \"zed\": [\n    \"a\"\n  ],\n  \"main\": [\n    \"b\"\n  ]\n}\n";
        assert_eq!(roundtrip(src), src);
    }

    #[test]
    fn an_account_with_no_topics_renders_as_an_empty_list() {
        let mut t = Topics::default();
        t.entry("main");
        assert_eq!(render(&t), "{\n  \"main\": []\n}\n");
    }

    #[test]
    fn non_ascii_topics_are_written_literally_not_escaped() {
        // Python passes ensure_ascii=False; a \u-escaped file would still parse
        // but is unreadable to the person editing it.
        let mut t = Topics::default();
        t.entry("main").push("台積電".into());
        assert!(render(&t).contains("台積電"));
    }
}
