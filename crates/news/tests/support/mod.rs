//! Shared harness for the `news shadow-ai` tests (`shadow_ai.rs`,
//! `shadow_stamp.rs`). Offline: the model is a stub agent resolved through
//! `$HOME` (the seam `tests/pipeline.rs` documents), feeds and article pages
//! come from a local publisher stub, and Telegram is a local stub that
//! records every request. Every stub fails closed — an unscripted request
//! gets 418.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::Duration;

pub const DATE: &str = "2026/11/17 (Tue)";
pub const FILE_DATE: &str = "2026-11-17";
pub const JOB: &str = "job-shadow-ai:3";
pub const D1: &str = "微軟公布新一代人工智慧晶片";
pub const D2: &str = "工研院發表語言模型評測報告";
pub const ARTICLE: &str = "<html><body><p>短文</p></body></html>";

// HOME and every NEWS_* knob are process-wide: every test takes this lock for
// its whole body, and gets its own HOME.
static ENV_LOCK: Mutex<()> = Mutex::new(());
static SEQ: AtomicUsize = AtomicUsize::new(0);

pub struct Env {
    pub home: PathBuf,
    _guard: MutexGuard<'static, ()>,
}

/// Every knob the shadow run reads, with the value these tests run under.
pub const KNOBS: [(&str, &str); 9] = [
    ("NEWS_PRECHECK", "1"),
    ("NEWS_PRECHECK_DECODE_TIMEOUT", "0.5"),
    ("NEWS_PRECHECK_FETCH_TIMEOUT", "2"),
    ("NEWS_PAYWALL_REPLACE", "0"),
    ("NEWS_PAYWALL_SUMMARY", "0"),
    ("NEWS_AI_THEME", "off"),
    ("NEWS_CROSS_DEDUP", "0"),
    ("NEWS_LLM_DEDUP_HINTS", "1"),
    ("NEWS_LLM_POST_DEDUP", "1"),
];

/// Removed from every run, in-process and spawned alike.
pub const CLEARED: [&str; 6] = [
    "NULLCLAW_JOB_ID",
    "NULLCLAW_SKILL_TIMEOUT",
    "NULLCLAW_SKILL_STARTED",
    "CLAW_CONFIG",
    "CLAW_ENV",
    "NEWS_AI_SOURCES",
];

impl Env {
    pub fn new() -> Env {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!(
            "news-shadow-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nullclaw")).unwrap();
        std::fs::create_dir_all(home.join("stub")).unwrap();
        let bin = home.join("nullclaw/zig-out/bin");
        std::fs::create_dir_all(&bin).unwrap();
        // resp.<n> for the nth call, else resp.default; each prompt recorded.
        std::fs::write(
            bin.join("nullclaw"),
            r#"#!/bin/sh
d="$HOME/stub"
n=$(cat "$d/counter" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$d/counter"
printf '%s' "$4" > "$d/prompt.$n"
if [ -f "$d/resp.$n" ]; then cat "$d/resp.$n"
elif [ -f "$d/resp.default" ]; then cat "$d/resp.default"
fi
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("nullclaw"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        std::env::set_var("HOME", &home);
        for k in CLEARED {
            std::env::remove_var(k);
        }
        for (k, v) in KNOBS {
            std::env::set_var(k, v);
        }
        Env {
            home,
            _guard: guard,
        }
    }

    pub fn reply(&self, n: usize, stdout: &str) {
        std::fs::write(self.home.join(format!("stub/resp.{n}")), stdout).unwrap();
    }

    pub fn reply_default(&self, stdout: &str) {
        std::fs::write(self.home.join("stub/resp.default"), stdout).unwrap();
    }

    pub fn agent_calls(&self) -> usize {
        std::fs::read_to_string(self.home.join("stub/counter"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    pub fn trace_lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.home.join(".nullclaw/skill-traces.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    pub fn all_traces(&self) -> Vec<serde_json::Value> {
        self.trace_lines()
            .iter()
            .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
            .collect()
    }

    pub fn traces(&self, event: &str) -> Vec<serde_json::Value> {
        self.all_traces()
            .into_iter()
            .filter(|v| v["event"] == event)
            .collect()
    }

    pub fn failure_log(&self) -> PathBuf {
        self.home.join(".nullclaw/news-failures.log")
    }

    pub fn shadow_dir(&self) -> PathBuf {
        self.home.join(".nullclaw/.news-shadow")
    }

    pub fn shadow_file(&self) -> PathBuf {
        self.shadow_dir().join(format!("{FILE_DATE}.md"))
    }

    /// Names in the shadow directory, sorted.
    pub fn shadow_listing(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.shadow_dir())
            .map(|d| {
                d.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// A bot token for `main`, so a Telegram send has everything it needs
    /// except a reason to happen.
    pub fn write_bot_token(&self) {
        std::fs::write(
            self.home.join(".nullclaw/config.json"),
            r#"{"channels":{"telegram":{"accounts":{"main":{"bot_token":"TESTTOKEN"}}}}}"#,
        )
        .unwrap();
    }

    pub fn write_feed_config(&self, feeds: &[(&str, String)]) {
        let list: Vec<serde_json::Value> = feeds
            .iter()
            .map(|(name, url)| serde_json::json!({"name": name, "url": url, "max_items": 10}))
            .collect();
        std::fs::write(
            self.home.join(".nullclaw/news-ai-feeds.json"),
            serde_json::json!({"feeds": list, "max_age_hours": 30, "feeds_deadline_secs": 10})
                .to_string(),
        )
        .unwrap();
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

// ── a local HTTP stub: publisher feeds, article pages, or Telegram ───────────

#[derive(Clone)]
pub struct Route {
    pub status: u16,
    pub body: String,
}

pub fn ok(body: &str) -> Route {
    Route {
        status: 200,
        body: body.to_string(),
    }
}

pub struct Stub {
    pub base: String,
    seen: mpsc::Receiver<String>,
}

impl Stub {
    /// Fails closed: an unscripted path answers 418. `script` receives the
    /// base URL, because a feed has to name its own article URLs.
    pub fn start(script: impl FnOnce(&str) -> Vec<(&'static str, Route)>) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let routes: Arc<HashMap<String, Route>> = Arc::new(
            script(&base)
                .into_iter()
                .map(|(p, r)| (p.to_string(), r))
                .collect(),
        );
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let routes = Arc::clone(&routes);
                let tx = tx.clone();
                std::thread::spawn(move || serve(stream, &routes, &tx));
            }
        });
        Stub { base, seen: rx }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Every request (`METHOD target`) served so far, sorted.
    pub fn seen(&self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(r) = self.seen.recv_timeout(Duration::from_millis(200)) {
            out.push(r);
        }
        out.sort();
        out
    }
}

fn serve(stream: TcpStream, routes: &HashMap<String, Route>, tx: &mpsc::Sender<String>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line == "\r\n" => break,
            Ok(_) => {
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
            }
            Err(_) => break,
        }
    }
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);
    let _ = tx.send(format!("{method} {target}"));
    let path = target.split('?').next().unwrap_or("");
    let route = routes.get(path).cloned().unwrap_or(Route {
        status: 418,
        body: "unscripted".to_string(),
    });
    let mut out = stream;
    let _ = out.write_all(
        format!(
            "HTTP/1.1 {} X\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            route.status,
            route.body.len()
        )
        .as_bytes(),
    );
    let _ = out.write_all(route.body.as_bytes());
    let _ = out.flush();
}

/// RSS dated relative to the real clock: the direct fetch filters on `now`.
pub fn fresh_rss(items: &[(&str, &str, i64)]) -> String {
    let mut s =
        String::from("<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><title>t</title>");
    for (title, link, minutes_ago) in items {
        let at = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_mins(*minutes_ago))
            .unwrap();
        let date = at.strftime("%a, %d %b %Y %H:%M:%S +0000");
        s.push_str(&format!(
            "<item><title>{title}</title><link>{link}</link><pubDate>{date}</pubDate></item>"
        ));
    }
    s.push_str("</channel></rss>");
    s
}

/// One feed of two fresh items, each with a readable article page.
pub fn publisher() -> Stub {
    Stub::start(|base| {
        let feed = fresh_rss(&[
            (D1, &format!("{base}/art/1"), 10),
            (D2, &format!("{base}/art/2"), 20),
        ]);
        vec![
            ("/ai.xml", ok(&feed)),
            ("/art/1", ok(ARTICLE)),
            ("/art/2", ok(ARTICLE)),
        ]
    })
}

/// The section a two-item direct build renders, as written to the file.
pub fn two_story_section(p: &Stub) -> String {
    format!(
        "**🤖 AI 人工智慧**\n- {D1} [🔗]({})\n- {D2} [🔗]({})\n",
        p.url("/art/1"),
        p.url("/art/2")
    )
}

/// Calls in order for a two-item direct AI section: half [0,1), half [1,2).
pub fn two_story_replies(env: &Env) {
    env.reply(1, &format!("- #1 {D1}"));
    env.reply(2, &format!("- #1 {D2}"));
}

pub fn markers(status: &str) -> String {
    format!("[skill-status:{status}]\n[trace:{JOB}]\n")
}
