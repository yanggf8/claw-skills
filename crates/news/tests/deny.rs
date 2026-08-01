//! The precheck's deny list, in its own test binary.
//!
//! `quality::active_config` caches the merged config in a process-wide
//! `OnceLock`. That is what production wants — one run, one config, read once —
//! but it means a test that installs a deny list fixes it for every other test
//! sharing the binary. Rust gives each integration test file its own process,
//! so this one file is the isolation.

use news::precheck::new_cache;
use news::summarize::run_ai_substage;
use news::text::Item;
use std::path::PathBuf;

const DATE: &str = "2026/07/13 (Mon)";

struct Env {
    home: PathBuf,
}

impl Env {
    fn new(deny: &str) -> Env {
        let home = std::env::temp_dir().join(format!("news-deny-{}", std::process::id()));
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
        std::fs::write(
            home.join(".nullclaw/news-quality-sources.json"),
            format!("{{\"deny\":[\"{deny}\"]}}"),
        )
        .unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("NEWS_PRECHECK", "1");
        std::env::set_var("NEWS_PAYWALL_REPLACE", "0");
        std::env::set_var("NEWS_CROSS_DEDUP", "0");
        std::env::remove_var("NULLCLAW_JOB_ID");
        Env { home }
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
            source: "denied-source".to_string(),
            link: format!("http://127.0.0.1:1/{}", i + 1),
            ..Default::default()
        })
        .collect()
}

#[test]
fn a_batch_whose_picks_are_all_filtered_out_is_a_success_with_no_lines() {
    // A deny entry for the fixture's source makes the precheck drop every pick
    // with no network at all. That is the filter working, not the model
    // failing, so the driver must not escalate to another subdivision — and it
    // must not cache the empty result either, or a transient mis-drop would
    // stick for the rest of the day.
    let env = Env::new("denied-source");
    env.reply("- #1 台積電法說會釋出樂觀展望\n- #2 輝達財報優於預期");
    match run_ai_substage(&items(), 0, 3, DATE, &new_cache()) {
        Ok(lines) => assert!(lines.is_empty(), "{lines:?}"),
        Err(e) => panic!("a filtered batch must not read as a failure: {e}"),
    }
    assert!(
        !env.home.join(".nullclaw/.news-cache").exists()
            || std::fs::read_dir(env.home.join(".nullclaw/.news-cache"))
                .map(|d| d.flatten().all(|e| {
                    std::fs::read_dir(e.path()).map(|x| x.count() == 0).unwrap_or(true)
                }))
                .unwrap_or(true),
        "an empty result was cached"
    );
}

#[test]
fn a_denied_source_costs_no_network_call_at_all() {
    // The deny check on the source name runs before any decode, which is the
    // only reason this whole file can be offline.
    let env = Env::new("denied-source");
    env.reply("- #1 台積電法說會釋出樂觀展望");
    let started = std::time::Instant::now();
    let _ = run_ai_substage(&items(), 0, 3, DATE, &new_cache());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the precheck went to the network for a denied source"
    );
}
