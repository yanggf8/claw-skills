//! Composition tests for the delivery stage, against a stub Telegram endpoint.
//!
//! The unit tests around `render` prove that chunking and the Markdown probe
//! each work. They say nothing about whether the two are wired to each other,
//! or whether the result reaches the API in the shape Telegram expects. That
//! gap is exactly what an adversarial review found in the traffic port — a
//! green suite that never saw the composition — so this file drives the real
//! `deliver_news` and inspects the requests that come out the other side.
//!
//! The stub records every request. A request it did not expect is still
//! recorded rather than dropped, so "the code sent something else" surfaces as
//! a failed assertion on the recorded list rather than as silence.

use news::deliver::deliver_news;
use news::render::PAYWALL_CONT_PREFIX;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

struct Stub {
    base_url: String,
    rx: mpsc::Receiver<String>,
}

impl Stub {
    fn start() -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
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
                let _ = tx.send(format!(
                    "{}\n{}",
                    request_line.trim(),
                    String::from_utf8_lossy(&body)
                ));

                let payload = "{\"ok\":true,\"result\":{}}";
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                );
            }
        });

        Stub {
            base_url: format!("http://127.0.0.1:{port}"),
            rx,
        }
    }

    fn requests(&self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(r) = self
            .rx
            .recv_timeout(std::time::Duration::from_millis(300))
        {
            out.push(r);
        }
        out
    }
}

/// The Telegram client reads its token from the agent config, so the tests
/// point HOME at a scratch directory holding a minimal one.
fn scratch_home() -> tempdir::TempDir {
    let dir = tempdir::TempDir::new();
    std::fs::create_dir_all(dir.path().join(".nullclaw")).unwrap();
    std::fs::write(
        dir.path().join(".nullclaw/config.json"),
        r#"{"channels":{"telegram":{"accounts":{"main":{"bot_token":"T:TOKEN"}}}}}"#,
    )
    .unwrap();
    dir
}

mod tempdir {
    //! A scratch directory that removes itself. Small enough not to justify a
    //! dependency, and keeps HOME-dependent tests from touching the real one.
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> TempDir {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let p = std::env::temp_dir()
                .join(format!("news-test-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).expect("scratch dir");
            TempDir(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn send(body: &str) -> Vec<String> {
    let home = scratch_home();
    let stub = Stub::start();
    let mut out = Vec::new();
    let mut err = Vec::new();
    // Serialised by the mutex in the caller: env vars are process-wide.
    std::env::set_var("HOME", home.path());
    let outcome = deliver_news(
        Some("12345"),
        body,
        "main",
        Some(stub.base_url.clone()),
        &mut out,
        &mut err,
    );
    assert_eq!(
        outcome,
        claw_core::delivery::DeliveryOutcome::Sent,
        "stderr: {}",
        String::from_utf8_lossy(&err)
    );
    stub.requests()
}

/// `deliver_news` mutates `HOME`, which is process-wide, so the tests that use
/// it run one at a time.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn field(request: &str, name: &str) -> Option<String> {
    let body = request.split_once('\n')?.1;
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get(name).map(|x| match x {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

#[test]
fn a_short_digest_is_one_request_in_markdown() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let requests = send("📰 早安新聞摘要\n- 甲 [🔗](https://a/1)");
    assert_eq!(requests.len(), 1);
    assert_eq!(field(&requests[0], "parse_mode").as_deref(), Some("Markdown"));
    let text = field(&requests[0], "text").unwrap();
    // No part numbering on a single chunk.
    assert!(!text.starts_with("(1/1)"), "unexpected numbering: {text}");
    assert!(text.contains("[🔗](https://a/1)"));
}

#[test]
fn a_long_digest_is_split_and_every_part_is_numbered() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let body = (0..300)
        .map(|i| format!("- 這是一則相當長的中文新聞標題編號 {i} [🔗](https://example.com/{i})"))
        .collect::<Vec<_>>()
        .join("\n");
    let requests = send(&body);
    assert!(requests.len() > 1, "expected a split, got {}", requests.len());

    let total = requests.len();
    for (i, r) in requests.iter().enumerate() {
        let text = field(r, "text").unwrap();
        assert!(
            text.starts_with(&format!("({}/{total})\n", i + 1)),
            "part {} not numbered: {}",
            i + 1,
            &text[..text.len().min(40)]
        );
    }
    // Chunks must fill the character budget. Measuring the limit in bytes
    // instead would still split cleanly and still keep every headline — it
    // would just send a Chinese digest as roughly three times as many
    // messages, which no assertion above would notice.
    let widest = requests
        .iter()
        .filter_map(|r| field(r, "text"))
        .map(|t| t.chars().count())
        .max()
        .unwrap();
    assert!(
        widest > 3000,
        "chunks are far below the 3800-character limit ({widest}); \
         the limit is probably being counted in bytes"
    );

    // Every headline survives the split — the point of chunking on line
    // boundaries rather than on a byte offset.
    let joined: String = requests
        .iter()
        .filter_map(|r| field(r, "text"))
        .collect::<Vec<_>>()
        .join("");
    for i in 0..300 {
        assert!(joined.contains(&format!("編號 {i} ")), "lost headline {i}");
    }
}

#[test]
fn an_unbalanced_asterisk_drops_the_whole_message_to_plaintext() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // One bad chunk must disarm the parse mode for every chunk, not just its
    // own: Telegram rejects the bad one otherwise and that part is simply lost.
    let requests = send("- 長科*成關鍵受惠股 [🔗](https://a/1)");
    assert_eq!(requests.len(), 1);
    assert_eq!(field(&requests[0], "parse_mode"), None);
}

#[test]
fn a_link_url_containing_an_underscore_stays_in_markdown() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // The probe skips link markup wholesale. Counting the URL's underscores
    // would send every ordinary digest as plaintext, expanding each [🔗] into
    // a bare URL.
    let requests = send("- 甲 [🔗](https://example.com/a_b_c)");
    assert_eq!(field(&requests[0], "parse_mode").as_deref(), Some("Markdown"));
}

#[test]
fn a_paywall_pair_is_never_split_across_two_messages() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // The continuation carries the original headline; alone in a later message
    // it reads as an orphaned 原文 note with nothing above it.
    //
    // The filler is sized so the pair lands ON a chunk boundary. Dropped in the
    // middle of a chunk it is never at risk, and the test would pass with the
    // guard deleted — which is what a first version of it did.
    let head =
        "- 免費替代標題 [🔗](https://free/1)".to_string();
    let cont = format!(
        "{PAYWALL_CONT_PREFIX}原文：付費標題 [🔗](https://paid/1)  ⚠️ 付費牆（原文需訂閱）"
    );
    let filler_line = "- 填充新聞標題 [🔗](https://example.com/x)";
    let per = filler_line.chars().count() + 1;
    // Leave room for the head but not for the continuation behind it.
    let target = 3800 - head.chars().count() - 2;
    let n_filler = target / per;
    let filler = std::iter::repeat_n(filler_line, n_filler)
        .collect::<Vec<_>>()
        .join("\n");
    let requests = send(&format!("{filler}\n{head}\n{cont}\n{filler}"));
    assert!(requests.len() > 1);

    for r in &requests {
        let text = field(r, "text").unwrap();
        let has_head = text.contains("免費替代標題");
        let has_cont = text.contains("原文：付費標題");
        assert_eq!(
            has_head, has_cont,
            "paywall pair separated; chunk was:\n{text}"
        );
    }
}

#[test]
fn no_chat_id_prints_the_body_and_sends_nothing() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = scratch_home();
    let stub = Stub::start();
    let mut out = Vec::new();
    let mut err = Vec::new();
    std::env::set_var("HOME", home.path());
    let outcome = deliver_news(None, "- 甲", "main", Some(stub.base_url.clone()), &mut out, &mut err);
    assert_eq!(outcome, claw_core::delivery::DeliveryOutcome::PrintedToStdout);
    assert_eq!(String::from_utf8_lossy(&out).trim(), "- 甲");
    assert!(stub.requests().is_empty(), "a manual run must not send");
}
