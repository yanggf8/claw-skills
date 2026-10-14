//! The precheck's author deny list, in its own test binary.
//!
//! Same isolation rule as `tests/deny.rs`: `quality::active_config` caches the
//! merged config in a process-wide `OnceLock`, so a config installed here must
//! not share a process with a test installing a different one. Every test in
//! this file installs the same `deny_authors` config, in this order-agnostic
//! way; the harness itself is a copy of `deny.rs`'s, for the same reasons.

use news::precheck::{new_cache, tier1_filter_items};
use news::summarize::run_ai_substage;
use news::text::Item;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

const DATE: &str = "2026/07/13 (Mon)";
const CONFIG: &str = "{\"deny_authors\":[\"蔣季容\"]}";

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct Env {
    home: PathBuf,
    _guard: MutexGuard<'static, ()>,
}

impl Env {
    fn new() -> Env {
        // Held for the whole Env lifetime: HOME is process-global, so the lock
        // must cover the test body, not just the setup.
        let guard = ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join(format!("news-deny-authors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nullclaw")).unwrap();
        std::fs::create_dir_all(home.join("stub")).unwrap();
        let bin = home.join("nullclaw/zig-out/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            bin.join("nullclaw"),
            "#!/bin/sh\nd=\"$HOME/stub\"\ncat \"$d/resp.default\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("nullclaw"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        std::fs::write(home.join(".nullclaw/news-quality-sources.json"), CONFIG).unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("NEWS_PRECHECK", "1");
        std::env::set_var("NEWS_PAYWALL_REPLACE", "0");
        std::env::set_var("NEWS_CROSS_DEDUP", "0");
        std::env::remove_var("NULLCLAW_JOB_ID");
        Env { home, _guard: guard }
    }

    fn reply(&self, stdout: &str) -> &Env {
        std::fs::write(self.home.join("stub/resp.default"), stdout).unwrap();
        self
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn items() -> Vec<Item> {
    ["台積電法說會釋出樂觀展望", "輝達財報優於預期", "記憶體股走弱"]
        .iter()
        .enumerate()
        .map(|(i, t)| Item {
            title: (*t).to_string(),
            source: "Yahoo新聞".to_string(),
            link: format!("http://127.0.0.1:1/{}", i + 1),
            ..Default::default()
        })
        .collect()
}

#[test]
fn a_denied_author_title_is_dropped_at_tier1() {
    // Title-borne bylines (「記者名報導 - 標題」 copies, or 「| 記者 | 新聞」
    // suffixed ones) are dropped before the model sees anything — which also
    // takes them out of every fallback listing and out of dedup, because both
    // feed from the same Tier-1 output.
    let _env = Env::new();
    let batch = vec![
        Item {
            title: "蔣季容報導 - 北市長辯論會協商過程曝光".to_string(),
            source: "Yahoo新聞".to_string(),
            link: "http://127.0.0.1:1/1".to_string(),
            ..Default::default()
        },
        Item {
            title: "台積電法說會釋出樂觀展望".to_string(),
            source: "Yahoo新聞".to_string(),
            link: "http://127.0.0.1:1/2".to_string(),
            ..Default::default()
        },
    ];
    let kept = tier1_filter_items(batch);
    assert_eq!(
        kept.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
        ["台積電法說會釋出樂觀展望"]
    );
}

#[test]
fn an_author_deny_without_body_excerpts_keeps_the_picks() {
    // The body check reads precheck excerpts; links here dead-end at
    // 127.0.0.1:1, so no pick has one. Fail-open is the contract: the wiring
    // must neither drop the picks nor read the batch as a failure.
    //
    // Coverage note: the positive path (excerpt exists → bullet dropped) is
    // unit-tested on the helper in precheck.rs; driving it through this
    // harness would need a live local article server, which the offline
    // convention here deliberately avoids. So this test alone would not catch
    // a summarize.rs call site that stopped passing the deny set.
    let env = Env::new();
    env.reply("- #1 台積電法說會釋出樂觀展望\n- #2 輝達財報優於預期");
    match run_ai_substage(&items(), 0, 3, DATE, &new_cache()) {
        Ok(lines) => assert_eq!(lines.len(), 2, "{lines:?}"),
        Err(e) => panic!("missing excerpts must not fail the batch: {e}"),
    }
}
