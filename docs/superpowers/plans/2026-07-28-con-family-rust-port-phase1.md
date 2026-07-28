# claw-skills Rust Port — Phase ① Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `claw-core` Rust foundation and port `doughcon` to Rust, proving that a native binary runs correctly under nullclaw cron with intact skill-contract markers and Telegram delivery.

**Architecture:** A Cargo workspace inside `~/a/claw-skills/crates/`. One `claw-core` library crate holding the host contract (config, telegram, delivery, markers, budget, outcome), plus one binary crate per skill — `doughcon` is the first. The Python `lib/` and `scripts/run.py` stay in place untouched; cutover is a single line in `SKILL.md`, and rollback is reverting that line. A separate Zig patch to nullclaw supplies the delivery budget that scheduled runs currently lack.

**Tech Stack:** Rust 2021, `ureq` 2 (blocking HTTP, matches `price-cli`), `serde_json` (adapter boundary only), `jiff` with `tzdb-bundle-always` (DST), `libc` (CLOCK_MONOTONIC). Zig for the nullclaw patch. Python 3 stays as the behaviour oracle.

**Spec:** `docs/specs/2026-07-28-con-family-rust-port-phase1-design.md` — read it before starting. The scheduler contract itself is canonical in `CLAUDE.md` → "Scheduler contract (hard constraints)".

## Global Constraints

- **The Python `lib/` and `doughcon/scripts/run.py` are FROZEN. Do not edit, refactor, or delete them.** `cct` (`~/a/cct/skills/cct`) and `autocli` (`~/.nullclaw/skills/autocli`) import `delivery` and `trace_marker` from `lib/` from outside this repo.
- **Behaviour parity is the acceptance bar, not "looks right".** Every difference from Python must appear in the Intentional Differences list (Task 9) or it is a bug.
- **Markers are literal stdout lines**: `[skill-status:ok|degraded|failed]` and `[trace:<NULLCLAW_JOB_ID>]`, emitted only when `NULLCLAW_JOB_ID` is set, only after delivery is resolved. Body goes to stdout; diagnostics go to stderr.
- **`verified` is not a boolean.** `0=unverified 1=ok 2=degraded 3=failed_verify`. `degraded` is `verified=2` → `last_status=error` → retry → operator alert. It is **not** a soft warning.
- **No JSON as the app data model.** Parse at the adapter boundary into Rust types immediately.
- **Every crate builds with zero warnings.** `cargo build --release` output must be clean.
- Rust edition 2021. Workspace members declared in `~/a/claw-skills/Cargo.toml`.

---

## File Structure

**Created:**

| Path | Responsibility |
|---|---|
| `Cargo.toml` | Workspace manifest, members list |
| `crates/claw-core/Cargo.toml` | Foundation crate manifest |
| `crates/claw-core/src/lib.rs` | Module wiring, re-exports |
| `crates/claw-core/src/marker.rs` | Emit the two contract lines + `[skill-event]` |
| `crates/claw-core/src/config.rs` | Config path resolution + bot-token lookup |
| `crates/claw-core/src/telegram.rs` | Bounded-retry send |
| `crates/claw-core/src/budget.rs` | Delivery deadline from cron env, CLOCK_MONOTONIC |
| `crates/claw-core/src/delivery.rs` | `deliver_or_fail` behaviour matrix |
| `crates/claw-core/src/outcome.rs` | Exit code / skill status / marker eligibility, kept independent |
| `crates/claw-core/tests/*.rs` | Ported unit tests |
| `crates/doughcon/Cargo.toml` | Pilot binary manifest |
| `crates/doughcon/src/main.rs` | CLI, DST gate, mode dispatch, exit ownership |
| `crates/doughcon/src/pizzint.rs` | HTTP + payload adapter |
| `crates/doughcon/src/report.rs` | Level/index derivation + message formatting |
| `doughcon/tests/test_run_characterization.py` | Oracle tests against the **Python** (Task 7) |
| `tools/differential/run.sh` | Runs both implementations over fixtures, diffs (Task 9) |
| `tools/install-skill.sh` | Strict atomic build→verify→stage→publish→activate (Task 11) |

**Modified:**

| Path | Change |
|---|---|
| `.gitignore` | Ignore skill `bin/` directories and `target/` |
| `doughcon/SKILL.md` | `## Script` flipped at cutover (Task 12) |
| `~/nullclaw/src/gateway.zig` | Scheduled skill spawn sets the budget env (Task 10) |
| `~/nullclaw/src/cron.zig` | New `SKILL_STARTED_ENV` const + helper (Task 10) |

---

### Task 1: Workspace scaffold + `claw-core::marker`

The smallest contract surface, and it has a complete Python test file to port. Workspace scaffolding folds in here because this is the first thing that needs it.

**Files:**
- Create: `Cargo.toml`, `crates/claw-core/Cargo.toml`, `crates/claw-core/src/lib.rs`, `crates/claw-core/src/marker.rs`
- Create: `crates/claw-core/tests/marker.rs`
- Modify: `.gitignore`

**Interfaces:**
- Produces:
  - `pub enum SkillStatus { Ok, Degraded, Failed }` with `pub fn as_str(&self) -> &'static str`
  - `pub fn emit_skill_status(status: SkillStatus, out: &mut impl Write) -> io::Result<()>`
  - `pub fn emit_trace(out: &mut impl Write) -> io::Result<()>`
  - `pub fn emit_fallback(skill: &str, primary: &str, fallback: &str, reason: &str, scope: &str, elapsed_ms: Option<u64>, err: &mut impl Write) -> io::Result<()>`
  - Both emit functions read `NULLCLAW_JOB_ID` from the environment on **every call** (it is the per-run trace id, not a stable job id).

**Note on the invalid-status test:** Python's `emit_skill_status("bogus")` raises `ValueError` *before* checking the environment. Rust's typed `SkillStatus` makes that unrepresentable, which is a strengthening, not a regression — record it in the Intentional Differences list. The parse function still needs the test, so also produce `pub fn parse_status(s: &str) -> Option<SkillStatus>`.

- [ ] **Step 1: Create the workspace manifest**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/claw-core", "crates/doughcon"]

[workspace.package]
edition = "2021"
```

- [ ] **Step 2: Create the claw-core manifest**

`crates/claw-core/Cargo.toml`:
```toml
[package]
name = "claw-core"
version = "0.1.0"
edition.workspace = true

[dependencies]
ureq = "2"
serde_json = "1"
libc = "0.2"
```

- [ ] **Step 3: Extend .gitignore**

Append to `.gitignore`:
```
# Rust build artifacts
target/

# built skill binaries (produced by tools/install-skill.sh, never committed)
*/bin/
```

- [ ] **Step 4: Write the failing tests**

`crates/claw-core/tests/marker.rs`:
```rust
use claw_core::marker::{emit_fallback, emit_skill_status, emit_trace, parse_status, SkillStatus};

/// Tests mutate the process environment, so they must not run concurrently.
/// Rust runs tests in threads by default; this file is run with --test-threads=1
/// (see Step 6) and every test sets the variable it depends on explicitly.
fn with_job_id<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
    match value {
        Some(v) => std::env::set_var("NULLCLAW_JOB_ID", v),
        None => std::env::remove_var("NULLCLAW_JOB_ID"),
    }
    let r = f();
    std::env::remove_var("NULLCLAW_JOB_ID");
    r
}

fn capture(f: impl FnOnce(&mut Vec<u8>)) -> String {
    let mut buf = Vec::new();
    f(&mut buf);
    String::from_utf8(buf).unwrap()
}

#[test]
fn status_noop_when_job_id_unset() {
    let out = with_job_id(None, || capture(|b| emit_skill_status(SkillStatus::Ok, b).unwrap()));
    assert_eq!(out, "");
}

#[test]
fn status_emits_when_job_id_set() {
    let out = with_job_id(Some("job-123:7"), || {
        capture(|b| emit_skill_status(SkillStatus::Ok, b).unwrap())
    });
    assert_eq!(out, "[skill-status:ok]\n");
}

#[test]
fn status_emits_degraded() {
    let out = with_job_id(Some("job-123:7"), || {
        capture(|b| emit_skill_status(SkillStatus::Degraded, b).unwrap())
    });
    assert_eq!(out, "[skill-status:degraded]\n");
}

#[test]
fn status_emits_failed() {
    let out = with_job_id(Some("job-123:7"), || {
        capture(|b| emit_skill_status(SkillStatus::Failed, b).unwrap())
    });
    assert_eq!(out, "[skill-status:failed]\n");
}

#[test]
fn trace_noop_when_job_id_unset() {
    let out = with_job_id(None, || capture(|b| emit_trace(b).unwrap()));
    assert_eq!(out, "");
}

#[test]
fn trace_emits_exact_job_id() {
    // nullclaw compares the marker payload to the run trace id with mem.eql,
    // so this must be byte-exact — no trimming, no normalisation.
    let out = with_job_id(Some("job-abc:42"), || capture(|b| emit_trace(b).unwrap()));
    assert_eq!(out, "[trace:job-abc:42]\n");
}

#[test]
fn parse_status_rejects_unknown() {
    assert_eq!(parse_status("ok"), Some(SkillStatus::Ok));
    assert_eq!(parse_status("degraded"), Some(SkillStatus::Degraded));
    assert_eq!(parse_status("failed"), Some(SkillStatus::Failed));
    assert_eq!(parse_status("bogus"), None);
    assert_eq!(parse_status("OK"), None, "matching is case-sensitive");
}

#[test]
fn fallback_always_emits_and_punctuates_by_elapsed() {
    // Never job-id gated — manual runs must stay diagnosable.
    let with_ms = with_job_id(None, || {
        capture(|b| emit_fallback("weather", "CWA", "HKO", "CWA returned HTTP 502", "the Taipei forecast", Some(1200), b).unwrap())
    });
    assert_eq!(
        with_ms,
        "[skill-event] weather skill fell back from CWA to HKO because CWA returned HTTP 502. Fallback covered the Taipei forecast and took 1200ms.\n"
    );

    let without_ms = with_job_id(None, || {
        capture(|b| emit_fallback("weather", "CWA", "HKO", "CWA returned HTTP 502", "the Taipei forecast", None, b).unwrap())
    });
    assert_eq!(
        without_ms,
        "[skill-event] weather skill fell back from CWA to HKO because CWA returned HTTP 502. Fallback covered the Taipei forecast.\n"
    );
}
```

- [ ] **Step 5: Run tests to verify they fail**

Run: `cd ~/a/claw-skills && cargo test -p claw-core --test marker`
Expected: FAIL — `unresolved import claw_core::marker`

- [ ] **Step 6: Implement marker.rs**

`crates/claw-core/src/marker.rs`:
```rust
//! Scheduler verification markers.
//!
//! nullclaw's classifySkillRun matches these as LITERAL stdout lines. Emit them
//! only after delivery is resolved, and only when NULLCLAW_JOB_ID is set, so
//! manual runs stay clean. NULLCLAW_JOB_ID holds the per-RUN trace id, so it is
//! read fresh on every call — never cached at startup.

use std::io::{self, Write};

pub const JOB_ID_ENV: &str = "NULLCLAW_JOB_ID";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillStatus {
    Ok,
    Degraded,
    Failed,
}

impl SkillStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillStatus::Ok => "ok",
            SkillStatus::Degraded => "degraded",
            SkillStatus::Failed => "failed",
        }
    }
}

pub fn parse_status(s: &str) -> Option<SkillStatus> {
    match s {
        "ok" => Some(SkillStatus::Ok),
        "degraded" => Some(SkillStatus::Degraded),
        "failed" => Some(SkillStatus::Failed),
        _ => None,
    }
}

fn job_id() -> Option<String> {
    std::env::var(JOB_ID_ENV).ok().filter(|v| !v.is_empty())
}

pub fn emit_skill_status(status: SkillStatus, out: &mut impl Write) -> io::Result<()> {
    if job_id().is_none() {
        return Ok(());
    }
    writeln!(out, "[skill-status:{}]", status.as_str())?;
    out.flush()
}

pub fn emit_trace(out: &mut impl Write) -> io::Result<()> {
    let Some(id) = job_id() else { return Ok(()) };
    writeln!(out, "[trace:{id}]")?;
    out.flush()
}

/// Natural-language fallback event for an agent reading the trace later.
/// Never job-id gated; stderr by default so it cannot pollute verified stdout.
pub fn emit_fallback(
    skill: &str,
    primary: &str,
    fallback: &str,
    reason: &str,
    scope: &str,
    elapsed_ms: Option<u64>,
    err: &mut impl Write,
) -> io::Result<()> {
    let tail = match elapsed_ms {
        Some(ms) => format!("Fallback covered {scope} and took {ms}ms."),
        None => format!("Fallback covered {scope}."),
    };
    writeln!(
        err,
        "[skill-event] {skill} skill fell back from {primary} to {fallback} because {reason}. {tail}"
    )?;
    err.flush()
}
```

`crates/claw-core/src/lib.rs`:
```rust
pub mod marker;
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd ~/a/claw-skills && cargo test -p claw-core --test marker -- --test-threads=1`
Expected: PASS, 8 tests.

- [ ] **Step 8: Commit**

```bash
cd ~/a/claw-skills
git add Cargo.toml .gitignore crates/claw-core
git commit -m "feat(claw-core): scheduler markers, ported from lib/trace_marker.py

Emits the two literal stdout lines nullclaw matches, gated on NULLCLAW_JOB_ID
being set so manual runs stay clean. The job id is read fresh per call because
nullclaw sets it to the per-RUN trace id, which classifySkillRun compares byte
for byte.

Typed SkillStatus makes the Python ValueError-on-bad-status path
unrepresentable; parse_status keeps the string boundary tested."
```

---

### Task 2: `claw-core::config`

**Files:**
- Create: `crates/claw-core/src/config.rs`, `crates/claw-core/tests/config.rs`
- Modify: `crates/claw-core/src/lib.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `pub fn resolve_config_path(explicit: Option<&Path>) -> PathBuf`
  - `pub fn get_bot_token(account: &str, explicit: Option<&Path>) -> Option<String>`

**Behaviour to preserve exactly** (`lib/telegram.py:30-49`):
1. Precedence: explicit argument → `$CLAW_CONFIG` → `~/.nullclaw/config.json`.
2. Any file-open or JSON-parse error yields `None` — never a panic.
3. nullclaw schema first: `channels.telegram.accounts.<account>.bot_token`.
4. Fall back to openclaw `channels.telegram.botToken` **even when the file also has an `accounts` map** but lacks the requested account.
5. An empty-string token is falsy in Python; treat empty as absent.

- [ ] **Step 1: Write the failing tests**

`crates/claw-core/tests/config.rs`:
```rust
use claw_core::config::{get_bot_token, resolve_config_path};
use std::io::Write;
use std::path::PathBuf;

fn write_tmp(name: &str, body: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("claw-core-cfg-{name}-{}.json", std::process::id()));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    p
}

#[test]
fn explicit_path_wins_over_env() {
    let explicit = write_tmp("explicit", r#"{"channels":{"telegram":{"botToken":"EXPLICIT"}}}"#);
    let other = write_tmp("env", r#"{"channels":{"telegram":{"botToken":"ENV"}}}"#);
    std::env::set_var("CLAW_CONFIG", &other);
    assert_eq!(get_bot_token("main", Some(&explicit)).as_deref(), Some("EXPLICIT"));
    assert_eq!(resolve_config_path(Some(&explicit)), explicit);
    std::env::remove_var("CLAW_CONFIG");
}

#[test]
fn nullclaw_account_schema_preferred() {
    let p = write_tmp(
        "nullclaw",
        r#"{"channels":{"telegram":{"accounts":{"main":{"bot_token":"ACCT"}},"botToken":"SINGLE"}}}"#,
    );
    assert_eq!(get_bot_token("main", Some(&p)).as_deref(), Some("ACCT"));
}

#[test]
fn falls_back_to_single_token_when_account_absent() {
    // Mixed-schema file, requested account missing: Python still falls through
    // to botToken rather than failing. Preserve that.
    let p = write_tmp(
        "mixed",
        r#"{"channels":{"telegram":{"accounts":{"other":{"bot_token":"ACCT"}},"botToken":"SINGLE"}}}"#,
    );
    assert_eq!(get_bot_token("main", Some(&p)).as_deref(), Some("SINGLE"));
}

#[test]
fn missing_file_is_none_not_panic() {
    let p = PathBuf::from("/nonexistent/claw-core/definitely-not-here.json");
    assert_eq!(get_bot_token("main", Some(&p)), None);
}

#[test]
fn malformed_json_is_none_not_panic() {
    let p = write_tmp("malformed", "{ this is not json");
    assert_eq!(get_bot_token("main", Some(&p)), None);
}

#[test]
fn empty_token_treated_as_absent() {
    let p = write_tmp("empty", r#"{"channels":{"telegram":{"accounts":{"main":{"bot_token":""}},"botToken":"SINGLE"}}}"#);
    assert_eq!(get_bot_token("main", Some(&p)).as_deref(), Some("SINGLE"));
}

#[test]
fn no_telegram_section_is_none() {
    let p = write_tmp("bare", r#"{"channels":{}}"#);
    assert_eq!(get_bot_token("main", Some(&p)), None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p claw-core --test config -- --test-threads=1`
Expected: FAIL — `unresolved import claw_core::config`

- [ ] **Step 3: Implement config.rs**

`crates/claw-core/src/config.rs`:
```rust
//! Config path + bot-token resolution.
//!
//! JSON is quarantined here: serde_json is used to read the file and the result
//! is immediately reduced to an Option<String>. No JSON value escapes this module.

use std::path::{Path, PathBuf};

pub const CONFIG_ENV: &str = "CLAW_CONFIG";

pub fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".nullclaw/config.json")
}

pub fn resolve_config_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    match std::env::var(CONFIG_ENV) {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => default_config_path(),
    }
}

/// Returns None on ANY failure — missing file, bad permissions, malformed JSON,
/// missing keys, or an empty token. Never panics, never distinguishes causes:
/// the caller's only question is "do I have a token".
pub fn get_bot_token(account: &str, explicit: Option<&Path>) -> Option<String> {
    let path = resolve_config_path(explicit);
    let body = std::fs::read_to_string(path).ok()?;
    let cfg: serde_json::Value = serde_json::from_str(&body).ok()?;
    let telegram = cfg.get("channels")?.get("telegram")?;

    let account_token = telegram
        .get("accounts")
        .and_then(|a| a.get(account))
        .and_then(|a| a.get("bot_token"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty());
    if let Some(t) = account_token {
        return Some(t.to_string());
    }

    telegram
        .get("botToken")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
}
```

Add `pub mod config;` to `crates/claw-core/src/lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p claw-core --test config -- --test-threads=1`
Expected: PASS, 7 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/claw-core
git commit -m "feat(claw-core): config path + bot-token resolution

Preserves the mixed-schema fallback: a file with an accounts map but no entry
for the requested account still falls through to openclaw's single botToken.
Every failure mode collapses to None, matching Python's bare except — the
caller's only question is whether a token exists."
```

---

### Task 3: `claw-core::telegram`

The retry engine. Tested against an in-process stub HTTP server so no network and no extra dependency.

**Files:**
- Create: `crates/claw-core/src/telegram.rs`, `crates/claw-core/tests/telegram.rs`, `crates/claw-core/tests/support/stub_server.rs`
- Modify: `crates/claw-core/src/lib.rs`

**Interfaces:**
- Consumes: `config::get_bot_token`
- Produces:
  - `pub struct SendOptions { pub account: String, pub config_path: Option<PathBuf>, pub deadline_s: Option<f64>, pub parse_mode: Option<String>, pub base_url: Option<String> }` with `Default` giving `account="main"`, `parse_mode=Some("Markdown")`, everything else `None`.
  - `pub fn send(chat_id: &str, text: &str, opts: &SendOptions) -> bool`
  - `pub const PER_ATTEMPT_TIMEOUT_S: f64 = 15.0;`
  - `pub const DEFAULT_DEADLINE_S: f64 = 30.0;`
  - `pub const BACKOFFS_S: [f64; 2] = [2.0, 5.0];`

**`base_url` is a test seam.** It defaults to `https://api.telegram.org`, matching Python's hardcoded host. Record it in Intentional Differences as a test-only addition with an identical default.

**Behaviour to preserve exactly** (`lib/telegram.py:52-159`):
1. No token → return false, **no telegram diagnostic printed**, zero HTTP attempts.
2. Budget = `deadline_s` if given else `DEFAULT_DEADLINE_S`. Budget ≤ 0 → log and return false with zero attempts.
3. The budget clock starts **after** token lookup and payload construction.
4. Max 3 attempts (`1 + BACKOFFS_S.len()`).
5. Per-attempt timeout = `min(15.0, remaining)`.
6. Retryable: HTTP 429, HTTP 500–599, connection/transport errors, timeouts. **Everything else stops immediately — including 408.**
7. HTTP 200 = success **without reading or parsing the response body**.
8. Any non-200 2xx = failure, no retry.
9. Backoff sleeps `min(backoff, remaining_budget)`; if remaining ≤ 0 before sleeping, log and return false.
10. Payload always includes `disable_web_page_preview: true`; `parse_mode` included when `Some`, **key entirely absent** when `None`.
11. All diagnostics go to stderr prefixed `[telegram] `.

- [ ] **Step 1: Write the stub server**

`crates/claw-core/tests/support/stub_server.rs`:
```rust
//! Minimal single-purpose HTTP stub. Serves a scripted sequence of responses so
//! retry behaviour can be asserted without network access.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

pub struct Recorded {
    pub body: String,
}

pub struct Stub {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    _shutdown: mpsc::Sender<()>,
}

impl Stub {
    pub fn attempts(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    pub fn body(&self, i: usize) -> String {
        self.requests.lock().unwrap()[i].body.clone()
    }
}

/// `statuses` is consumed one entry per request. A `None` entry means "hang
/// past the per-attempt timeout" (used to exercise timeout handling); the stub
/// sleeps `hang_ms` then closes without responding.
pub fn start(statuses: Vec<Option<u16>>, hang_ms: u64) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let (tx, rx) = mpsc::channel::<()>();

    std::thread::spawn(move || {
        let mut seq = statuses.into_iter();
        for stream in listener.incoming() {
            if rx.try_recv().is_ok() {
                break;
            }
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }
            let mut body = vec![0u8; content_length];
            let _ = reader.read_exact(&mut body);
            recorded.lock().unwrap().push(Recorded {
                body: String::from_utf8_lossy(&body).to_string(),
            });

            match seq.next().unwrap_or(Some(200)) {
                None => {
                    std::thread::sleep(std::time::Duration::from_millis(hang_ms));
                }
                Some(code) => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.flush();
                }
            }
        }
    });

    Stub {
        base_url: format!("http://{addr}"),
        requests,
        _shutdown: tx,
    }
}
```

- [ ] **Step 2: Write the failing tests**

`crates/claw-core/tests/telegram.rs`:
```rust
mod support { pub mod stub_server; }
use claw_core::telegram::{send, SendOptions};
use std::io::Write;
use std::path::PathBuf;
use support::stub_server;

fn cfg_with_token() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("claw-core-tg-{}.json", std::process::id()));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(br#"{"channels":{"telegram":{"botToken":"T"}}}"#).unwrap();
    p
}

fn opts(base: &str) -> SendOptions {
    SendOptions {
        config_path: Some(cfg_with_token()),
        base_url: Some(base.to_string()),
        ..Default::default()
    }
}

#[test]
fn success_first_try() {
    let s = stub_server::start(vec![Some(200)], 0);
    assert!(send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 1);
}

#[test]
fn parse_mode_default_included_and_none_omitted() {
    let s = stub_server::start(vec![Some(200), Some(200)], 0);
    let mut o = opts(&s.base_url);
    assert!(send("chat", "hi", &o));
    o.parse_mode = None;
    assert!(send("chat", "hi", &o));
    assert!(s.body(0).contains("\"parse_mode\":\"Markdown\""));
    assert!(!s.body(1).contains("parse_mode"), "key must be absent, not null");
    assert!(s.body(0).contains("\"disable_web_page_preview\":true"));
}

#[test]
fn retries_502_502_then_succeeds() {
    let s = stub_server::start(vec![Some(502), Some(502), Some(200)], 0);
    assert!(send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 3);
}

#[test]
fn three_502_returns_false() {
    let s = stub_server::start(vec![Some(502), Some(502), Some(502)], 0);
    assert!(!send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 3, "exactly 3 attempts, never 4");
}

#[test]
fn permanent_403_does_not_retry() {
    let s = stub_server::start(vec![Some(403)], 0);
    assert!(!send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 1);
}

#[test]
fn http_408_is_permanent_not_retryable() {
    // Deliberate: Python's _is_retryable_http covers only 429 and 5xx.
    // 408 Request Timeout looks retryable but is NOT. Preserve it.
    let s = stub_server::start(vec![Some(408)], 0);
    assert!(!send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 1);
}

#[test]
fn http_429_retries() {
    let s = stub_server::start(vec![Some(429), Some(200)], 0);
    assert!(send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 2);
}

#[test]
fn non_200_2xx_is_failure_without_retry() {
    let s = stub_server::start(vec![Some(204)], 0);
    assert!(!send("chat", "hi", &opts(&s.base_url)));
    assert_eq!(s.attempts(), 1);
}

#[test]
fn success_does_not_require_ok_true_body() {
    // The stub returns Content-Length: 0. Python treats HTTP 200 as success
    // without parsing the body; Rust must not start requiring {"ok":true}.
    let s = stub_server::start(vec![Some(200)], 0);
    assert!(send("chat", "hi", &opts(&s.base_url)));
}

#[test]
fn zero_deadline_skips_entirely() {
    let s = stub_server::start(vec![Some(200)], 0);
    let mut o = opts(&s.base_url);
    o.deadline_s = Some(0.0);
    assert!(!send("chat", "hi", &o));
    assert_eq!(s.attempts(), 0, "no HTTP attempt when the budget is already spent");
}

#[test]
fn deadline_blocks_second_attempt() {
    let s = stub_server::start(vec![None, Some(200)], 300);
    let mut o = opts(&s.base_url);
    o.deadline_s = Some(0.25);
    assert!(!send("chat", "hi", &o));
    assert_eq!(s.attempts(), 1, "budget exhausted after the first slow attempt");
}

#[test]
fn backoff_schedule_is_two_then_five_seconds() {
    // The gap between attempts is BACKOFFS_S, not a fixed or exponential delay.
    // Asserted on elapsed wall time with generous slack so a loaded machine does
    // not make this flaky, but tight enough to catch a wrong schedule.
    let s = stub_server::start(vec![Some(502), Some(502), Some(200)], 0);
    let mut o = opts(&s.base_url);
    o.deadline_s = Some(30.0);
    let t0 = std::time::Instant::now();
    assert!(send("chat", "hi", &o));
    let elapsed = t0.elapsed().as_secs_f64();
    assert_eq!(s.attempts(), 3);
    assert!(elapsed >= 7.0, "expected >= 2s + 5s of backoff, got {elapsed:.2}s");
    assert!(elapsed < 12.0, "backoff took far longer than 2s + 5s: {elapsed:.2}s");
}

#[test]
fn backoff_is_clipped_by_remaining_budget() {
    // Python sleeps min(backoff, remaining). With a 3s budget the 5s second
    // backoff must not be slept in full.
    let s = stub_server::start(vec![Some(502), Some(502), Some(200)], 0);
    let mut o = opts(&s.base_url);
    o.deadline_s = Some(3.0);
    let t0 = std::time::Instant::now();
    let _ = send("chat", "hi", &o);
    assert!(t0.elapsed().as_secs_f64() < 6.0, "slept past the budget");
}

#[test]
fn no_token_returns_false_without_attempting() {
    let s = stub_server::start(vec![Some(200)], 0);
    let o = SendOptions {
        config_path: Some(PathBuf::from("/nonexistent/none.json")),
        base_url: Some(s.base_url.clone()),
        ..Default::default()
    };
    assert!(!send("chat", "hi", &o));
    assert_eq!(s.attempts(), 0);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p claw-core --test telegram -- --test-threads=1`
Expected: FAIL — `unresolved import claw_core::telegram`

- [ ] **Step 4: Implement telegram.rs**

`crates/claw-core/src/telegram.rs`:
```rust
//! Bounded-retry Telegram send.
//!
//! Ported from lib/telegram.py. The retry policy is deliberate and narrow:
//! only 429 and 5xx plus transport failures are retried. 408 is NOT retryable.
//! HTTP 200 is success without reading the response body.
//!
//! JSON appears only as the request payload built here; no JSON value escapes.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::config::get_bot_token;

pub const PER_ATTEMPT_TIMEOUT_S: f64 = 15.0;
pub const DEFAULT_DEADLINE_S: f64 = 30.0;
pub const BACKOFFS_S: [f64; 2] = [2.0, 5.0];
pub const DEFAULT_BASE_URL: &str = "https://api.telegram.org";

pub struct SendOptions {
    pub account: String,
    pub config_path: Option<PathBuf>,
    pub deadline_s: Option<f64>,
    pub parse_mode: Option<String>,
    /// Test seam only. Defaults to the real host.
    pub base_url: Option<String>,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self {
            account: "main".into(),
            config_path: None,
            deadline_s: None,
            parse_mode: Some("Markdown".into()),
            base_url: None,
        }
    }
}

fn log(msg: &str) {
    let mut err = std::io::stderr();
    let _ = writeln!(err, "[telegram] {msg}");
    let _ = err.flush();
}

fn is_retryable_http(code: u16) -> bool {
    code == 429 || (500..600).contains(&code)
}

pub fn send(chat_id: &str, text: &str, opts: &SendOptions) -> bool {
    // No token: silent false. The [delivery] diagnostic is the caller's job.
    let Some(token) = get_bot_token(&opts.account, opts.config_path.as_deref()) else {
        return false;
    };

    let base = opts.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL);
    let url = format!("{}/bot{}/sendMessage", base.trim_end_matches('/'), token);

    let mut payload = serde_json::Map::new();
    payload.insert("chat_id".into(), serde_json::Value::from(chat_id));
    payload.insert("text".into(), serde_json::Value::from(text));
    payload.insert("disable_web_page_preview".into(), serde_json::Value::Bool(true));
    if let Some(pm) = &opts.parse_mode {
        payload.insert("parse_mode".into(), serde_json::Value::from(pm.as_str()));
    }
    let body = serde_json::Value::Object(payload).to_string();

    let budget = opts.deadline_s.unwrap_or(DEFAULT_DEADLINE_S);
    if budget <= 0.0 {
        log(&format!("deadline already exhausted (budget={budget:.1}s); skipping send"));
        return false;
    }

    // Budget clock starts here — after token lookup and payload construction,
    // matching Python.
    let start = Instant::now();
    let max_attempts = 1 + BACKOFFS_S.len();

    for attempt in 1..=max_attempts {
        let elapsed = start.elapsed().as_secs_f64();
        let remaining = budget - elapsed;
        if remaining <= 0.0 {
            log(&format!("deadline exhausted before attempt {attempt}/{max_attempts}"));
            return false;
        }
        let per_attempt = PER_ATTEMPT_TIMEOUT_S.min(remaining);

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs_f64(per_attempt))
            .build();
        let result = agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body);

        match result {
            Ok(resp) => {
                if resp.status() == 200 {
                    return true;
                }
                log(&format!(
                    "unexpected status {} on attempt {attempt}/{max_attempts}",
                    resp.status()
                ));
                return false;
            }
            Err(ureq::Error::Status(code, _)) => {
                if !is_retryable_http(code) {
                    log(&format!("permanent HTTP {code} on attempt {attempt}/{max_attempts}"));
                    return false;
                }
                let left = budget - start.elapsed().as_secs_f64();
                log(&format!(
                    "attempt {attempt}/{max_attempts} got HTTP {code} (remaining={left:.1}s)"
                ));
            }
            Err(ureq::Error::Transport(t)) => {
                let left = budget - start.elapsed().as_secs_f64();
                log(&format!(
                    "attempt {attempt}/{max_attempts} got transport error (remaining={left:.1}s): {t}"
                ));
            }
        }

        if attempt >= max_attempts {
            break;
        }
        let backoff = BACKOFFS_S[attempt - 1];
        if backoff > 0.0 {
            let remaining_budget = (budget - start.elapsed().as_secs_f64()).max(0.0);
            if remaining_budget <= 0.0 {
                log("no time left for backoff; giving up");
                return false;
            }
            std::thread::sleep(Duration::from_secs_f64(backoff.min(remaining_budget)));
        }
    }

    log(&format!("all {max_attempts} attempts failed"));
    false
}
```

Add `pub mod telegram;` to `lib.rs`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p claw-core --test telegram -- --test-threads=1`
Expected: PASS, 14 tests. Two of them sleep through real backoff, so this suite takes ~15 s — that is expected, not a hang.

- [ ] **Step 6: Commit**

```bash
git add crates/claw-core
git commit -m "feat(claw-core): bounded-retry telegram send

Ported from lib/telegram.py with the retry policy preserved exactly: 429 and
5xx plus transport errors retry, everything else stops immediately — including
408, which looks retryable and is not. HTTP 200 is success without parsing the
body, so a bodyless 200 still counts.

Tested against an in-process TCP stub, so attempt counts and the deadline
cutoff are asserted without network."
```

---

### Task 4: `claw-core::budget`

**Files:**
- Create: `crates/claw-core/src/budget.rs`, `crates/claw-core/tests/budget.rs`
- Modify: `crates/claw-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub fn monotonic_secs() -> f64` — CLOCK_MONOTONIC, the same clock domain as Python's `time.monotonic()` on Linux.
  - `pub fn resolve_delivery_deadline() -> Option<f64>`
  - `pub const SKILL_TIMEOUT_ENV: &str = "NULLCLAW_SKILL_TIMEOUT";`
  - `pub const SKILL_STARTED_ENV: &str = "NULLCLAW_SKILL_STARTED";`

**⚠ Clock-domain hazard.** Python computes `elapsed = time.monotonic() - started`. On Linux `time.monotonic()` is `CLOCK_MONOTONIC` (system-wide, boot-relative), **not** a Unix epoch timestamp. If the producer writes wall-clock seconds, the subtraction yields a large negative number which `max(0.0, ...)` silently clamps to zero — elapsed becomes permanently 0 and the bug is invisible. Task 10 must write CLOCK_MONOTONIC; the test below pins the assumption from the consumer side.

**Behaviour to preserve exactly** (`lib/delivery.py:74-106`):
1. `NULLCLAW_SKILL_TIMEOUT` unset/empty → `None` (telegram uses its own default).
2. Unparseable timeout → `None`.
3. Timeout ≤ 0 → `None`.
4. `NULLCLAW_SKILL_STARTED` set and parseable → `max(0, timeout - max(0, now - started) - 1.0)`.
5. `NULLCLAW_SKILL_STARTED` unparseable → fall through to `max(0, timeout - 1.0)`.
6. `NULLCLAW_SKILL_STARTED` unset → `max(0, timeout - 1.0)`.

- [ ] **Step 1: Write the failing tests**

`crates/claw-core/tests/budget.rs`:
```rust
use claw_core::budget::{monotonic_secs, resolve_delivery_deadline, SKILL_STARTED_ENV, SKILL_TIMEOUT_ENV};

fn clear() {
    std::env::remove_var(SKILL_TIMEOUT_ENV);
    std::env::remove_var(SKILL_STARTED_ENV);
}

#[test]
fn unset_timeout_is_none() {
    clear();
    assert_eq!(resolve_delivery_deadline(), None);
}

#[test]
fn malformed_timeout_is_none() {
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "not-a-number");
    assert_eq!(resolve_delivery_deadline(), None);
    clear();
}

#[test]
fn non_positive_timeout_is_none() {
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "0");
    assert_eq!(resolve_delivery_deadline(), None);
    std::env::set_var(SKILL_TIMEOUT_ENV, "-5");
    assert_eq!(resolve_delivery_deadline(), None);
    clear();
}

#[test]
fn timeout_without_started_reserves_one_second() {
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "30");
    assert_eq!(resolve_delivery_deadline(), Some(29.0));
    clear();
}

#[test]
fn malformed_started_falls_back_to_timeout_minus_one() {
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "30");
    std::env::set_var(SKILL_STARTED_ENV, "yesterday");
    assert_eq!(resolve_delivery_deadline(), Some(29.0));
    clear();
}

#[test]
fn started_subtracts_elapsed() {
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "30");
    // Pretend the skill started 10 monotonic seconds ago.
    std::env::set_var(SKILL_STARTED_ENV, format!("{}", monotonic_secs() - 10.0));
    let got = resolve_delivery_deadline().unwrap();
    assert!((got - 19.0).abs() < 0.5, "expected ~19.0, got {got}");
    clear();
}

#[test]
fn future_start_clamps_elapsed_to_zero() {
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "30");
    std::env::set_var(SKILL_STARTED_ENV, format!("{}", monotonic_secs() + 1000.0));
    let got = resolve_delivery_deadline().unwrap();
    assert!((got - 29.0).abs() < 0.5, "expected ~29.0, got {got}");
    clear();
}

#[test]
fn exhausted_budget_floors_at_zero() {
    clear();
    std::env::set_var(SKILL_TIMEOUT_ENV, "5");
    std::env::set_var(SKILL_STARTED_ENV, format!("{}", monotonic_secs() - 999.0));
    assert_eq!(resolve_delivery_deadline(), Some(0.0));
    clear();
}

#[test]
fn monotonic_is_not_a_unix_epoch_timestamp() {
    // Regression guard for the clock-domain hazard. A Unix epoch value in 2026
    // is ~1.78e9; CLOCK_MONOTONIC is seconds since boot and will be far smaller
    // on any machine that has not been up for 50 years. If this ever fails,
    // monotonic_secs() has been changed to wall clock and every elapsed
    // computation is silently wrong.
    let t = monotonic_secs();
    assert!(t > 0.0);
    assert!(t < 1.0e9, "monotonic_secs() returned {t}, which looks like wall clock");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p claw-core --test budget -- --test-threads=1`
Expected: FAIL — `unresolved import claw_core::budget`

- [ ] **Step 3: Implement budget.rs**

`crates/claw-core/src/budget.rs`:
```rust
//! Wall-clock budget for delivery, derived from the cron environment.
//!
//! CLOCK DOMAIN: NULLCLAW_SKILL_STARTED is CLOCK_MONOTONIC seconds, matching
//! Python's time.monotonic() on Linux. A wall-clock producer would make every
//! elapsed computation silently zero (the negative difference clamps), so the
//! producer side (nullclaw) and this consumer must agree. See tests.

pub const SKILL_TIMEOUT_ENV: &str = "NULLCLAW_SKILL_TIMEOUT";
pub const SKILL_STARTED_ENV: &str = "NULLCLAW_SKILL_STARTED";

pub fn monotonic_secs() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: ts is a valid, properly aligned timespec we own.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

/// None means "no budget information" — telegram falls back to its own cap.
/// Every malformed input degrades to None or to the timeout-only path; nothing
/// here fails loudly, matching Python.
pub fn resolve_delivery_deadline() -> Option<f64> {
    let raw_timeout = std::env::var(SKILL_TIMEOUT_ENV).ok().filter(|v| !v.is_empty())?;
    let timeout: f64 = raw_timeout.parse().ok()?;
    if timeout <= 0.0 {
        return None;
    }

    if let Ok(raw_started) = std::env::var(SKILL_STARTED_ENV) {
        if !raw_started.is_empty() {
            if let Ok(started) = raw_started.parse::<f64>() {
                let elapsed = (monotonic_secs() - started).max(0.0);
                let remaining = (timeout - elapsed).max(0.0);
                // Reserve 1s for the skill to exit cleanly after delivery.
                return Some((remaining - 1.0).max(0.0));
            }
        }
    }
    Some((timeout - 1.0).max(0.0))
}
```

Add `pub mod budget;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p claw-core --test budget -- --test-threads=1`
Expected: PASS, 9 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/claw-core
git commit -m "feat(claw-core): delivery budget from the cron environment

Ported from delivery.py's _resolve_delivery_deadline, including the branch that
has never executed in production — nullclaw sets NULLCLAW_SKILL_TIMEOUT only for
manual runs and never sets NULLCLAW_SKILL_STARTED at all. Task 10 fixes that;
this is the consumer side.

Pins the clock-domain assumption with a test: NULLCLAW_SKILL_STARTED is
CLOCK_MONOTONIC, not wall clock. Getting that wrong clamps elapsed to zero
forever and looks completely healthy."
```

---

### Task 5: `claw-core::delivery`

**Files:**
- Create: `crates/claw-core/src/delivery.rs`, `crates/claw-core/tests/delivery.rs`
- Modify: `crates/claw-core/src/lib.rs`

**Interfaces:**
- Consumes: `telegram::{send, SendOptions}`, `budget::resolve_delivery_deadline`
- Produces:
  - `pub enum DeliveryOutcome { PrintedToStdout, Sent, FailedFatal, FailedSoft }`
  - `pub fn deliver(chat_id: Option<&str>, body: &str, opts: &DeliverOptions, out: &mut impl Write, err: &mut impl Write) -> DeliveryOutcome`
  - `pub struct DeliverOptions { pub account: String, pub fail_on_delivery_error: bool, pub parse_mode: Option<String>, pub config_path: Option<PathBuf>, pub base_url: Option<String> }` — `Default` gives `account="main"`, `fail_on_delivery_error=true`, `parse_mode=Some("Markdown")`.

**Design difference from Python, deliberate:** Python's `deliver_or_fail` calls `sys.exit(1)` itself. In Rust the function **returns `FailedFatal`** and the binary owns the exit — that is the outcome model the spec requires (exit code and semantic status must stay independent). Callers must map `FailedFatal` to `exit(1)` **before emitting markers**. Record in Intentional Differences.

**Behaviour to preserve exactly** (`lib/delivery.py:18-71`):
1. `None` or empty chat id → print body + newline to stdout, return `PrintedToStdout`. No send attempted.
2. Send succeeds → **no output at all**, return `Sent`.
3. Send fails, `fail_on_delivery_error=true` → body to **stdout first**, then `[delivery] telegram send failed for chat=<id> account=<acct>` to **stderr**, return `FailedFatal`.
4. Send fails, `fail_on_delivery_error=false` → same two writes, return `FailedSoft`.
5. The deadline passed to telegram comes from `resolve_delivery_deadline()`.

- [ ] **Step 1: Write the failing tests**

`crates/claw-core/tests/delivery.rs`:
```rust
mod support { pub mod stub_server; }
use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use std::io::Write;
use std::path::PathBuf;
use support::stub_server;

fn cfg() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("claw-core-del-{}.json", std::process::id()));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(br#"{"channels":{"telegram":{"botToken":"T"}}}"#).unwrap();
    p
}

fn opts(base: &str) -> DeliverOptions {
    DeliverOptions { config_path: Some(cfg()), base_url: Some(base.into()), ..Default::default() }
}

fn run(chat: Option<&str>, body: &str, o: &DeliverOptions) -> (DeliveryOutcome, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let r = deliver(chat, body, o, &mut out, &mut err);
    (r, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn none_chat_prints_body_to_stdout() {
    let s = stub_server::start(vec![], 0);
    let (r, out, err) = run(None, "hello", &opts(&s.base_url));
    assert!(matches!(r, DeliveryOutcome::PrintedToStdout));
    assert_eq!(out, "hello\n");
    assert_eq!(err, "");
    assert_eq!(s.attempts(), 0);
}

#[test]
fn empty_chat_prints_body_to_stdout() {
    let s = stub_server::start(vec![], 0);
    let (r, out, _) = run(Some(""), "hello", &opts(&s.base_url));
    assert!(matches!(r, DeliveryOutcome::PrintedToStdout));
    assert_eq!(out, "hello\n");
    assert_eq!(s.attempts(), 0);
}

#[test]
fn success_emits_nothing() {
    let s = stub_server::start(vec![Some(200)], 0);
    let (r, out, err) = run(Some("chat"), "hello", &opts(&s.base_url));
    assert!(matches!(r, DeliveryOutcome::Sent));
    assert_eq!(out, "", "channel has the body; do not echo it");
    assert_eq!(err, "");
}

#[test]
fn failure_default_is_fatal_and_preserves_body_on_stdout() {
    let s = stub_server::start(vec![Some(403)], 0);
    let (r, out, err) = run(Some("chat9"), "hello", &opts(&s.base_url));
    assert!(matches!(r, DeliveryOutcome::FailedFatal));
    assert_eq!(out, "hello\n", "body must survive on stdout for cron capture");
    assert!(err.contains("[delivery] telegram send failed for chat=chat9 account=main"));
}

#[test]
fn failure_opt_out_is_soft_but_still_writes_both() {
    let s = stub_server::start(vec![Some(403)], 0);
    let mut o = opts(&s.base_url);
    o.fail_on_delivery_error = false;
    let (r, out, err) = run(Some("chat9"), "hello", &o);
    assert!(matches!(r, DeliveryOutcome::FailedSoft));
    assert_eq!(out, "hello\n");
    assert!(err.contains("[delivery] telegram send failed"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p claw-core --test delivery -- --test-threads=1`
Expected: FAIL — `unresolved import claw_core::delivery`

- [ ] **Step 3: Implement delivery.rs**

`crates/claw-core/src/delivery.rs`:
```rust
//! Canonical delivery helper.
//!
//! Centralises the "skill silently succeeds after a delivery failure" bug class:
//! on failure the body is preserved on stdout so the cron capture still has the
//! data, and the diagnostic goes to stderr.
//!
//! Unlike the Python original this does NOT exit the process. It returns an
//! outcome and the binary decides, because exit code and semantic status are
//! independent in nullclaw's classification.

use std::io::Write;
use std::path::PathBuf;

use crate::budget::resolve_delivery_deadline;
use crate::telegram::{send, SendOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    PrintedToStdout,
    Sent,
    FailedFatal,
    FailedSoft,
}

pub struct DeliverOptions {
    pub account: String,
    pub fail_on_delivery_error: bool,
    pub parse_mode: Option<String>,
    pub config_path: Option<PathBuf>,
    pub base_url: Option<String>,
}

impl Default for DeliverOptions {
    fn default() -> Self {
        Self {
            account: "main".into(),
            fail_on_delivery_error: true,
            parse_mode: Some("Markdown".into()),
            config_path: None,
            base_url: None,
        }
    }
}

pub fn deliver(
    chat_id: Option<&str>,
    body: &str,
    opts: &DeliverOptions,
    out: &mut impl Write,
    err: &mut impl Write,
) -> DeliveryOutcome {
    let chat = chat_id.filter(|c| !c.is_empty());
    let Some(chat) = chat else {
        let _ = writeln!(out, "{body}");
        let _ = out.flush();
        return DeliveryOutcome::PrintedToStdout;
    };

    let send_opts = SendOptions {
        account: opts.account.clone(),
        config_path: opts.config_path.clone(),
        deadline_s: resolve_delivery_deadline(),
        parse_mode: opts.parse_mode.clone(),
        base_url: opts.base_url.clone(),
    };

    if send(chat, body, &send_opts) {
        return DeliveryOutcome::Sent;
    }

    // stdout first — it is the data. stderr second — it is the diagnostic.
    let _ = writeln!(out, "{body}");
    let _ = out.flush();
    let _ = writeln!(
        err,
        "[delivery] telegram send failed for chat={chat} account={}",
        opts.account
    );
    let _ = err.flush();

    if opts.fail_on_delivery_error {
        DeliveryOutcome::FailedFatal
    } else {
        DeliveryOutcome::FailedSoft
    }
}
```

Add `pub mod delivery;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p claw-core --test delivery -- --test-threads=1`
Expected: PASS, 5 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/claw-core
git commit -m "feat(claw-core): deliver_or_fail behaviour matrix

Four branches preserved exactly: no chat id prints the body; success emits
nothing; failure writes the body to stdout BEFORE the diagnostic to stderr.

Deliberate difference from Python: this returns FailedFatal instead of calling
sys.exit(1). nullclaw classifies exit code and semantic status independently,
so the binary must own the exit — and must exit before emitting markers."
```

---

### Task 6: `claw-core::outcome`

Ties exit code, skill status, and marker emission into one place so no skill can get the ordering wrong.

**Files:**
- Create: `crates/claw-core/src/outcome.rs`, `crates/claw-core/tests/outcome.rs`
- Modify: `crates/claw-core/src/lib.rs`

**Interfaces:**
- Consumes: `marker::{emit_skill_status, emit_trace, SkillStatus}`
- Produces:
  - `pub enum Finish { Marked { status: SkillStatus, exit: i32 }, Unmarked { exit: i32 } }`
  - `pub fn finish(f: Finish, out: &mut impl Write) -> i32` — emits markers when `Marked` (status first, then trace), emits nothing when `Unmarked`, and returns the exit code the caller passes to `std::process::exit`.

**Why `Unmarked` exists:** a hard delivery failure and a record-mode filesystem failure both exit non-zero **without** markers, because nullclaw's `exit_code != 0` branch overrides marker parsing anyway and the Python emits nothing there.

- [ ] **Step 1: Write the failing tests**

`crates/claw-core/tests/outcome.rs`:
```rust
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};

fn with_job_id<T>(v: Option<&str>, f: impl FnOnce() -> T) -> T {
    match v {
        Some(v) => std::env::set_var("NULLCLAW_JOB_ID", v),
        None => std::env::remove_var("NULLCLAW_JOB_ID"),
    }
    let r = f();
    std::env::remove_var("NULLCLAW_JOB_ID");
    r
}

#[test]
fn marked_ok_emits_status_then_trace_in_that_order() {
    let mut out = Vec::new();
    let code = with_job_id(Some("t-1"), || {
        finish(Finish::Marked { status: SkillStatus::Ok, exit: 0 }, &mut out)
    });
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8(out).unwrap(), "[skill-status:ok]\n[trace:t-1]\n");
}

#[test]
fn marked_degraded_still_exits_zero() {
    // degraded is a SEMANTIC status, not a process failure. nullclaw turns it
    // into verified=2 / last_status=error on its own.
    let mut out = Vec::new();
    let code = with_job_id(Some("t-2"), || {
        finish(Finish::Marked { status: SkillStatus::Degraded, exit: 0 }, &mut out)
    });
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8(out).unwrap(), "[skill-status:degraded]\n[trace:t-2]\n");
}

#[test]
fn unmarked_emits_nothing_and_returns_exit() {
    let mut out = Vec::new();
    let code = with_job_id(Some("t-3"), || finish(Finish::Unmarked { exit: 1 }, &mut out));
    assert_eq!(code, 1);
    assert_eq!(String::from_utf8(out).unwrap(), "");
}

#[test]
fn marked_is_silent_without_job_id() {
    let mut out = Vec::new();
    let code = with_job_id(None, || {
        finish(Finish::Marked { status: SkillStatus::Ok, exit: 0 }, &mut out)
    });
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8(out).unwrap(), "");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p claw-core --test outcome -- --test-threads=1`
Expected: FAIL — `unresolved import claw_core::outcome`

- [ ] **Step 3: Implement outcome.rs**

`crates/claw-core/src/outcome.rs`:
```rust
//! Process outcome: exit code and semantic status are INDEPENDENT.
//!
//! nullclaw's precedence (classifySkillRun): a timeout or a non-zero exit
//! overrides all markers; only on exit 0 does it read the marker lines. So a
//! non-zero exit path must not emit markers, and a semantic `degraded` must
//! still exit 0 and let nullclaw decide it is verified=2.

use std::io::Write;

use crate::marker::{emit_skill_status, emit_trace, SkillStatus};

pub enum Finish {
    /// Exit 0 paths that report a semantic status.
    Marked { status: SkillStatus, exit: i32 },
    /// Hard failures: nullclaw's exit_code != 0 branch wins, so emit nothing.
    Unmarked { exit: i32 },
}

pub fn finish(f: Finish, out: &mut impl Write) -> i32 {
    match f {
        Finish::Marked { status, exit } => {
            let _ = emit_skill_status(status, out);
            let _ = emit_trace(out);
            exit
        }
        Finish::Unmarked { exit } => exit,
    }
}
```

Add `pub mod outcome;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p claw-core -- --test-threads=1`
Expected: PASS, all suites — marker 8 + config 7 + telegram 14 + budget 9 + delivery 5 + outcome 4 = 47 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/claw-core
git commit -m "feat(claw-core): outcome model keeping exit code and status independent

nullclaw reads markers only on exit 0, so a hard failure must emit none and a
semantic degraded must still exit 0. Collapsing these into a Result or a bool
is how a port silently changes what the scheduler records."
```

---

### Task 7: doughcon characterization tests (against the **Python**)

doughcon has zero tests. Before any Rust exists, pin the current behaviour by testing the Python. These tests become the differential oracle in Task 9.

**Files:**
- Create: `doughcon/tests/test_run_characterization.py`
- Create: `doughcon/tests/fixtures/{full.json,all_null.json,zero_index_with_data.json,no_timestamp.json,malformed.json}`

**Interfaces:**
- Produces: fixture files and documented expected behaviour that Task 8 and Task 9 both consume.

**Note:** These tests run the Python via subprocess with a stubbed PizzINT URL, so they need no network. `doughcon/scripts/run.py` hardcodes the URL, so the test monkeypatches `fetch_doughcon` by importing the module rather than shelling out where possible; the subprocess cases use a `PIZZINT_BASE` env var **only if** it exists. It does not — so the plan uses in-process import for payload cases and subprocess only for the DST gate and argument handling, which need no network.

- [ ] **Step 1: Create the fixtures**

`doughcon/tests/fixtures/full.json`:
```json
{"defcon_level": 3, "overall_index": 42, "timestamp": "2026-06-03T03:23:38.739Z",
 "data": [{"current_popularity": 55}, {"current_popularity": 12}]}
```

`doughcon/tests/fixtures/all_null.json`:
```json
{"defcon_level": 5, "overall_index": 0, "timestamp": "2026-06-03T03:23:38.739Z",
 "data": [{"current_popularity": null}, {"current_popularity": null}]}
```

`doughcon/tests/fixtures/zero_index_with_data.json`:
```json
{"defcon_level": 5, "overall_index": 0, "timestamp": "2026-06-03T03:23:38.739Z",
 "data": [{"current_popularity": 0}, {"current_popularity": 3}]}
```

`doughcon/tests/fixtures/no_timestamp.json`:
```json
{"defcon_level": 4, "overall_index": 7, "data": [{"current_popularity": 9}]}
```

`doughcon/tests/fixtures/malformed.json`:
```json
[1, 2, 3]
```

- [ ] **Step 2: Write the characterization tests**

`doughcon/tests/test_run_characterization.py`:
```python
"""Characterization tests for doughcon/scripts/run.py.

These pin CURRENT behaviour before the Rust port. They are the oracle: if one
of these changes, the port changed behaviour and that must be a recorded,
deliberate decision — not a surprise.

Run: python3 doughcon/tests/test_run_characterization.py
"""
import io
import json
import os
import sys
import unittest
from contextlib import redirect_stdout, redirect_stderr
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "scripts"))
sys.path.insert(0, os.path.join(HERE, "..", "..", "lib"))

import run as doughcon  # noqa: E402


def fixture(name):
    with open(os.path.join(HERE, "fixtures", name)) as f:
        return json.load(f)


class IndexDerivationTests(unittest.TestCase):
    """The -1 sentinel is NOT 'index == 0'."""

    def _index(self, data):
        raw_index = data.get("overall_index")
        places = data.get("data", [])
        all_null = all(p.get("current_popularity") is None for p in places) if places else True
        return -1 if (raw_index is None or (raw_index == 0 and all_null)) else raw_index

    def test_normal_index_passes_through(self):
        self.assertEqual(self._index(fixture("full.json")), 42)

    def test_zero_with_all_null_is_minus_one(self):
        self.assertEqual(self._index(fixture("all_null.json")), -1)

    def test_zero_with_real_data_stays_zero(self):
        # A genuine zero is NOT no-data. This is the subtle one.
        self.assertEqual(self._index(fixture("zero_index_with_data.json")), 0)

    def test_missing_index_is_minus_one(self):
        self.assertEqual(self._index({"data": [{"current_popularity": 5}]}), -1)

    def test_empty_places_counts_as_all_null(self):
        self.assertEqual(self._index({"overall_index": 0, "data": []}), -1)


class FormatUpdatedTests(unittest.TestCase):
    def test_uses_api_timestamp_not_run_time(self):
        out = doughcon.format_updated(fixture("full.json"))
        self.assertIn("2026-06-03", out)
        self.assertIn("CST", out)
        self.assertIn("美東", out)

    def test_formats_to_minutes_not_seconds(self):
        out = doughcon.format_updated(fixture("full.json"))
        self.assertNotIn(":38", out, "API path is minute-resolution")

    def test_missing_timestamp_falls_back_to_run_time_with_seconds(self):
        out = doughcon.format_updated(fixture("no_timestamp.json"))
        self.assertTrue(out.endswith("CST"))
        # cst_now() is second-resolution: HH:MM:SS
        self.assertRegex(out, r"\d{2}:\d{2}:\d{2} CST$")

    def test_unparseable_timestamp_falls_back_silently(self):
        out = doughcon.format_updated({"timestamp": "not-a-date"})
        self.assertRegex(out, r"\d{2}:\d{2}:\d{2} CST$")


class DeliverModeTests(unittest.TestCase):
    def _run(self, argv, fetch_result=None, fetch_raises=None):
        out, err = io.StringIO(), io.StringIO()
        code = 0
        def fake_fetch():
            if fetch_raises:
                raise fetch_raises
            return fetch_result
        with mock.patch.object(doughcon, "fetch_doughcon", fake_fetch), \
             mock.patch.object(sys, "argv", ["run.py"] + argv), \
             redirect_stdout(out), redirect_stderr(err):
            try:
                doughcon.main()
            except SystemExit as e:
                code = e.code or 0
        return code, out.getvalue(), err.getvalue()

    def test_deliver_no_chat_prints_body_and_marks_ok(self):
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "t-1"}, clear=True):
            code, out, err = self._run([], fetch_result=fixture("full.json"))
        self.assertEqual(code, 0)
        self.assertIn("🍕 DOUGHCON 情報", out)
        self.assertIn("目前等級：DOUGHCON 3", out)
        self.assertIn("指數：42", out)
        self.assertIn("`t-1`", out, "job id is appended to the body in deliver mode")
        self.assertIn("[skill-status:ok]", out)
        self.assertIn("[trace:t-1]", out)

    def test_no_data_marks_degraded_not_ok(self):
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "t-2"}, clear=True):
            code, out, _ = self._run([], fetch_result=fixture("all_null.json"))
        self.assertEqual(code, 0)
        self.assertIn("指數：-1", out)
        self.assertIn("[skill-status:degraded]", out)

    def test_upstream_failure_is_degraded_exit_zero(self):
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "t-3"}, clear=True):
            code, out, _ = self._run([], fetch_raises=RuntimeError("boom"))
        self.assertEqual(code, 0, "upstream failure is a soft degrade, not exit 1")
        self.assertIn("[WARN: doughcon unavailable", out)
        self.assertIn("[skill-status:degraded]", out)

    def test_markers_absent_without_job_id(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            code, out, _ = self._run([], fetch_result=fixture("full.json"))
        self.assertEqual(code, 0)
        self.assertNotIn("[skill-status:", out)
        self.assertNotIn("[trace:", out)
        self.assertNotIn("`", out, "no job-id suffix on the body either")


class DstGateTests(unittest.TestCase):
    def _run_gate(self, et_hour, job_id="t-g"):
        out, err = io.StringIO(), io.StringIO()
        code = 0
        with mock.patch.object(sys, "argv", ["run.py", "--et-hour", str(et_hour)]), \
             mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": job_id}, clear=True), \
             redirect_stdout(out), redirect_stderr(err):
            try:
                doughcon.main()
            except SystemExit as e:
                code = e.code or 0
        return code, out.getvalue(), err.getvalue()

    def test_gate_mismatch_is_ok_with_markers_and_no_body(self):
        from datetime import datetime
        wrong = (datetime.now(doughcon._NY).hour + 5) % 24
        code, out, err = self._run_gate(wrong)
        self.assertEqual(code, 0)
        self.assertIn("[skip: US-Eastern hour", err)
        self.assertIn("[skill-status:ok]", out)
        self.assertIn("[trace:t-g]", out)
        self.assertNotIn("DOUGHCON 情報", out)

    def test_out_of_range_hour_is_accepted_and_skips(self):
        # argparse does NOT validate 0-23. -1 and 99 are permanent skips.
        code, out, err = self._run_gate(99)
        self.assertEqual(code, 0)
        self.assertIn("[skip:", err)


if __name__ == "__main__":
    unittest.main(verbosity=2)
```

- [ ] **Step 3: Run the characterization tests against the Python**

Run: `cd ~/a/claw-skills && python3 doughcon/tests/test_run_characterization.py`
Expected: PASS. **If any test fails, the test is wrong about the Python, not the other way round — fix the test.** These describe reality; they do not prescribe it.

- [ ] **Step 4: Commit**

```bash
git add doughcon/tests
git commit -m "test(doughcon): characterization tests pinning current Python behaviour

doughcon had zero tests. These pin the behaviour the Rust port must reproduce,
written against the Python and passing before any Rust exists.

The subtle ones: a genuine overall_index of 0 with real popularity data stays 0
and is NOT the -1 no-data sentinel; the API-timestamp path formats to minutes
while the fallback formats to seconds; a DST-gate skip is exit 0 WITH markers
and no body; --et-hour is not range-validated."
```

---

### Task 8: doughcon Rust implementation

**Files:**
- Create: `crates/doughcon/Cargo.toml`, `crates/doughcon/src/main.rs`, `crates/doughcon/src/pizzint.rs`, `crates/doughcon/src/report.rs`
- Create: `crates/doughcon/tests/report.rs`

**Interfaces:**
- Consumes: `claw_core::{delivery, marker, outcome}`
- Produces: binary `doughcon` accepting `--mode deliver|record`, `--deliver-to <CHAT_ID>`, `--account <NAME>`, `--et-hour <H>`.

**Argument parsing:** hand-rolled, not `clap`. The surface is four flags and `clap` would add a dependency plus its own `--help` formatting; more importantly `clap` would *validate* `--et-hour` as a range if configured naively, and the characterization test pins that it is **not** validated.

- [ ] **Step 1: Create the manifest**

`crates/doughcon/Cargo.toml`:
```toml
[package]
name = "doughcon"
version = "0.1.0"
edition.workspace = true

[[bin]]
name = "doughcon"
path = "src/main.rs"

[dependencies]
claw-core = { path = "../claw-core" }
ureq = "2"
serde_json = "1"
jiff = { version = "0.1", features = ["tzdb-bundle-always"] }
```

- [ ] **Step 2: Write the failing report tests**

`crates/doughcon/tests/report.rs`:
```rust
use doughcon::report::{derive_index, format_body, NO_DATA};

#[test]
fn normal_index_passes_through() {
    assert_eq!(derive_index(Some(42), &[Some(55.0), Some(12.0)]), 42);
}

#[test]
fn zero_with_all_null_is_no_data() {
    assert_eq!(derive_index(Some(0), &[None, None]), NO_DATA);
}

#[test]
fn zero_with_real_data_stays_zero() {
    // A genuine zero is not no-data.
    assert_eq!(derive_index(Some(0), &[Some(0.0), Some(3.0)]), 0);
}

#[test]
fn missing_index_is_no_data() {
    assert_eq!(derive_index(None, &[Some(5.0)]), NO_DATA);
}

#[test]
fn empty_places_counts_as_all_null() {
    assert_eq!(derive_index(Some(0), &[]), NO_DATA);
}

#[test]
fn body_matches_python_layout() {
    let b = format_body("3", 42, "2026-06-03 11:23 CST（美東 06-03 23:23 EDT）", None);
    assert_eq!(
        b,
        "🍕 DOUGHCON 情報\n目前等級：DOUGHCON 3\n指數：42\n更新：2026-06-03 11:23 CST（美東 06-03 23:23 EDT）"
    );
}

#[test]
fn body_appends_job_id_when_present() {
    let b = format_body("3", 42, "U", Some("t-1"));
    assert!(b.ends_with("\n\n`t-1`"));
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p doughcon --test report`
Expected: FAIL — crate does not build, `report` module missing.

- [ ] **Step 4: Implement report.rs**

`crates/doughcon/src/report.rs`:
```rust
//! Index derivation and message layout.

pub const NO_DATA: i64 = -1;

/// "No data" is NOT `index == 0`. Zero collapses to the sentinel only when
/// every place reports null popularity; an empty place list also counts as
/// all-null. A genuine zero with real data stays zero.
pub fn derive_index(raw_index: Option<i64>, popularity: &[Option<f64>]) -> i64 {
    let all_null = if popularity.is_empty() {
        true
    } else {
        popularity.iter().all(|p| p.is_none())
    };
    match raw_index {
        None => NO_DATA,
        Some(0) if all_null => NO_DATA,
        Some(v) => v,
    }
}

pub fn format_body(level: &str, index: i64, updated: &str, job_id: Option<&str>) -> String {
    let mut s = format!(
        "🍕 DOUGHCON 情報\n目前等級：DOUGHCON {level}\n指數：{index}\n更新：{updated}"
    );
    if let Some(id) = job_id {
        s.push_str(&format!("\n\n`{id}`"));
    }
    s
}
```

Create `crates/doughcon/src/lib.rs` so the integration test and `main.rs` share the same modules:

```rust
//! Library half of the doughcon skill. The binary in main.rs consumes these
//! modules, and tests/ imports them directly — a bin-only crate cannot be
//! imported by an integration test.

pub mod pizzint;
pub mod report;
```

and declare both targets in `crates/doughcon/Cargo.toml`, above `[dependencies]`:

```toml
[lib]
name = "doughcon"
path = "src/lib.rs"
```

- [ ] **Step 5: Run report tests to verify they pass**

Run: `cargo test -p doughcon --test report`
Expected: PASS, 7 tests.

- [ ] **Step 6: Implement pizzint.rs**

`crates/doughcon/src/pizzint.rs`:
```rust
//! PizzINT dashboard adapter. JSON stops here.

use std::time::Duration;

pub struct Snapshot {
    pub level: String,
    pub raw_index: Option<i64>,
    pub popularity: Vec<Option<f64>>,
    pub timestamp: Option<String>,
}

pub const DEFAULT_URL: &str = "https://pizzint.watch/api/dashboard-data";
const TIMEOUT_S: u64 = 20;

pub fn fetch(base_url: Option<&str>) -> Result<Snapshot, String> {
    let url = base_url.unwrap_or(DEFAULT_URL);
    let body = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(TIMEOUT_S))
        .build()
        .get(url)
        .set("Accept", "application/json")
        .set("User-Agent", "nullclaw/1.0")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    parse(&body)
}

/// A payload that is not a JSON object is rejected HERE, so it routes to the
/// same degraded path as a fetch failure. Python would have thrown later,
/// outside the fetch handler, and exited hard — recorded as an intentional
/// difference.
pub fn parse(body: &str) -> Result<Snapshot, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let obj = v.as_object().ok_or_else(|| "payload is not a JSON object".to_string())?;

    let level = match obj.get("defcon_level") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => "?".to_string(),
    };
    let raw_index = obj.get("overall_index").and_then(|v| v.as_i64());
    let popularity = obj
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .map(|p| p.get("current_popularity").and_then(|v| v.as_f64()))
                .collect()
        })
        .unwrap_or_default();
    let timestamp = obj.get("timestamp").and_then(|t| t.as_str()).map(String::from);

    Ok(Snapshot { level, raw_index, popularity, timestamp })
}
```

- [ ] **Step 7: Implement main.rs**

`crates/doughcon/src/main.rs`:
```rust
//! doughcon — fetch the PizzINT DOUGHCON level and deliver or record.
//!
//! Exit ownership lives here: claw_core::delivery reports an outcome and this
//! binary decides the exit code, because a hard delivery failure must exit
//! BEFORE markers while a semantic degrade must exit 0 WITH them.

use std::io::Write;

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};
use doughcon::pizzint;
use doughcon::report::{derive_index, format_body, NO_DATA};
use jiff::{tz::TimeZone, Timestamp, Zoned};

struct Args {
    mode: String,
    deliver_to: Option<String>,
    account: String,
    et_hour: Option<i32>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { mode: "deliver".into(), deliver_to: None, account: "main".into(), et_hour: None };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let need = |i: usize| -> Result<String, String> {
            argv.get(i + 1).cloned().ok_or_else(|| format!("{} requires a value", argv[i]))
        };
        match argv[i].as_str() {
            "--mode" => { a.mode = need(i)?; i += 2; }
            "--deliver-to" => { a.deliver_to = Some(need(i)?); i += 2; }
            "--account" => { a.account = need(i)?; i += 2; }
            // Deliberately NOT range-validated: the Python accepts -1 and 99,
            // which become permanent skips. Pinned by characterization test.
            "--et-hour" => { a.et_hour = Some(need(i)?.parse().map_err(|_| "--et-hour must be an integer".to_string())?); i += 2; }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if a.mode != "deliver" && a.mode != "record" {
        return Err(format!("--mode must be deliver or record, got {}", a.mode));
    }
    Ok(a)
}

fn history_log_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join(".nullclaw/doughcon-history.log")
}

fn cst_now() -> String {
    let tz = TimeZone::fixed(jiff::tz::offset(8));
    Timestamp::now().to_zoned(tz).strftime("%Y-%m-%d %H:%M:%S CST").to_string()
}

/// API timestamp → Taipei + US-Eastern, minute resolution. Falls back to the
/// run time (SECOND resolution) whenever the timestamp is missing or unparseable.
fn format_updated(raw: Option<&str>) -> String {
    let Some(raw) = raw else { return cst_now() };
    let Ok(ts) = raw.parse::<Timestamp>() else { return cst_now() };
    let (Ok(tpe), Ok(ny)) = (TimeZone::get("Asia/Taipei"), TimeZone::get("America/New_York")) else {
        let tz = TimeZone::fixed(jiff::tz::offset(8));
        return ts.to_zoned(tz).strftime("%Y-%m-%d %H:%M CST").to_string();
    };
    let cst = ts.to_zoned(tpe).strftime("%Y-%m-%d %H:%M CST").to_string();
    let et: Zoned = ts.to_zoned(ny);
    format!("{cst}（美東 {} {}）", et.strftime("%m-%d %H:%M"), et.time_zone().to_offset(et.timestamp()).1)
}

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => { let _ = writeln!(err, "[ERROR: {e}]"); std::process::exit(2); }
    };

    // DST gate. Fail-open: if tz data is unavailable, warn and run anyway.
    if let Some(target) = args.et_hour {
        match TimeZone::get("America/New_York") {
            Err(_) => { let _ = writeln!(err, "[WARN: --et-hour requires tz data; running unconditionally]"); }
            Ok(ny) => {
                let now = Timestamp::now().to_zoned(ny);
                if now.hour() as i32 != target {
                    let _ = writeln!(err, "[skip: US-Eastern hour {:02} != target {:02}]", now.hour(), target);
                    std::process::exit(finish(Finish::Marked { status: SkillStatus::Ok, exit: 0 }, &mut out));
                }
            }
        }
    }

    let base = std::env::var("DOUGHCON_BASE_URL").ok();
    let snapshot = match pizzint::fetch(base.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            if args.mode == "deliver" {
                let msg = format!("[WARN: doughcon unavailable - {e}]");
                let opts = DeliverOptions {
                    account: args.account.clone(),
                    fail_on_delivery_error: false,
                    ..Default::default()
                };
                // Return value deliberately ignored: an upstream failure degrades
                // even if the delivery of that warning also failed.
                let _ = deliver(args.deliver_to.as_deref(), &msg, &opts, &mut out, &mut err);
                std::process::exit(finish(Finish::Marked { status: SkillStatus::Degraded, exit: 0 }, &mut out));
            }
            let _ = writeln!(err, "[ERROR: doughcon unavailable - {e}]");
            std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
        }
    };

    let index = derive_index(snapshot.raw_index, &snapshot.popularity);
    let updated = format_updated(snapshot.timestamp.as_deref());

    if args.mode == "deliver" {
        let job_id = std::env::var("NULLCLAW_JOB_ID").ok().filter(|v| !v.is_empty());
        let body = format_body(&snapshot.level, index, &updated, job_id.as_deref());
        let opts = DeliverOptions { account: args.account.clone(), ..Default::default() };
        let outcome = deliver(args.deliver_to.as_deref(), &body, &opts, &mut out, &mut err);
        if outcome == DeliveryOutcome::FailedFatal {
            std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
        }
        let status = if index != NO_DATA { SkillStatus::Ok } else { SkillStatus::Degraded };
        std::process::exit(finish(Finish::Marked { status, exit: 0 }, &mut out));
    }

    let line = format!("{}  DOUGHCON {}  index={}\n", cst_now(), snapshot.level, index);
    match std::fs::OpenOptions::new().create(true).append(true).open(history_log_path()) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(line.as_bytes()) {
                let _ = writeln!(err, "[ERROR: could not write history log - {e}]");
                std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
            }
        }
        Err(e) => {
            let _ = writeln!(err, "[ERROR: could not write history log - {e}]");
            std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
        }
    }
    std::process::exit(finish(Finish::Marked { status: SkillStatus::Ok, exit: 0 }, &mut out));
}
```

- [ ] **Step 8: Build and run the whole suite**

Run: `cargo build --release && cargo test -- --test-threads=1`
Expected: clean build, zero warnings, all tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/doughcon
git commit -m "feat(doughcon): Rust port of the pilot skill

Exit ownership sits in main: a hard delivery failure exits 1 BEFORE markers
(nullclaw's exit_code!=0 branch wins anyway), while an upstream fetch failure
degrades at exit 0 WITH markers even if delivering that warning also failed.

Arguments are hand-parsed rather than clap because --et-hour must stay
un-validated: the Python accepts -1 and 99 and turns them into permanent skips,
and a characterization test pins that."
```

---

### Task 9: Differential harness + Intentional Differences list

Two translations can share a misunderstanding, so the oracle is the running Python, not the ported tests.

**Files:**
- Create: `tools/differential/run.sh`, `tools/differential/cases.tsv`
- Create: `docs/specs/2026-07-28-phase1-intentional-differences.md`

- [ ] **Step 1: Write the case table**

`tools/differential/cases.tsv` — tab-separated: `name`, `argv`, `payload_fixture`, `job_id`, `expect_exit`:
```
full_deliver_stdout	--mode deliver	full.json	t-diff-1	0
nodata_deliver_stdout	--mode deliver	all_null.json	t-diff-2	0
zero_with_data	--mode deliver	zero_index_with_data.json	t-diff-3	0
no_timestamp	--mode deliver	no_timestamp.json	t-diff-4	0
record_mode	--mode record	full.json	t-diff-5	0
no_job_id	--mode deliver	full.json		0
gate_skip	--mode deliver --et-hour 99	full.json	t-diff-6	0
```

- [ ] **Step 2: Write the harness**

`tools/differential/run.sh`:
```bash
#!/usr/bin/env bash
# Runs the Python and the Rust implementation over identical fixtures and diffs
# exit code, stdout, and stderr. Any difference must be justified in
# docs/specs/2026-07-28-phase1-intentional-differences.md.
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
FIXTURES="$ROOT/doughcon/tests/fixtures"
BIN="$ROOT/target/release/doughcon"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

[ -x "$BIN" ] || { echo "build first: cargo build --release" >&2; exit 1; }

fail=0
while IFS=$'\t' read -r name argv fixture job_id expect_exit; do
  [ -z "${name:-}" ] && continue
  case "$name" in \#*) continue ;; esac

  # A local stub serves the fixture so neither implementation touches the network.
  python3 -c "
import http.server, json, sys, threading
body = open('$FIXTURES/$fixture','rb').read()
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass
srv = http.server.HTTPServer(('127.0.0.1', 0), H)
print(srv.server_port, flush=True)
srv.serve_forever()
" > "$STAGE/port.$name" &
  stub_pid=$!
  sleep 0.4
  port=$(head -1 "$STAGE/port.$name")
  url="http://127.0.0.1:$port/"

  env NULLCLAW_JOB_ID="$job_id" HOME="$STAGE/py" DOUGHCON_BASE_URL="$url" \
    python3 - "$argv" <<PY > "$STAGE/$name.py.out" 2> "$STAGE/$name.py.err"
import os, sys, runpy
os.makedirs(os.path.expanduser("~/.nullclaw"), exist_ok=True)
sys.argv = ["run.py"] + sys.argv[1].split()
import urllib.request
_orig = urllib.request.Request
def patched(url, *a, **k):
    return _orig(os.environ["DOUGHCON_BASE_URL"], *a, **k)
urllib.request.Request = patched
runpy.run_path("$ROOT/doughcon/scripts/run.py", run_name="__main__")
PY
  py_exit=$?

  env NULLCLAW_JOB_ID="$job_id" HOME="$STAGE/rs" DOUGHCON_BASE_URL="$url" \
    "$BIN" $argv > "$STAGE/$name.rs.out" 2> "$STAGE/$name.rs.err"
  rs_exit=$?

  kill $stub_pid 2>/dev/null

  if [ "$py_exit" != "$rs_exit" ]; then
    echo "DIFF $name: exit py=$py_exit rs=$rs_exit"; fail=1
  fi
  if ! diff -u "$STAGE/$name.py.out" "$STAGE/$name.rs.out" > "$STAGE/$name.out.diff"; then
    echo "DIFF $name: stdout"; cat "$STAGE/$name.out.diff"; fail=1
  fi
  if ! diff -u "$STAGE/$name.py.err" "$STAGE/$name.rs.err" > "$STAGE/$name.err.diff"; then
    echo "DIFF $name: stderr"; cat "$STAGE/$name.err.diff"; fail=1
  fi
  [ "$fail" = 0 ] && echo "ok   $name"
done < "$ROOT/tools/differential/cases.tsv"

# History-log comparison for record mode.
if ! diff -u "$STAGE/py/.nullclaw/doughcon-history.log" "$STAGE/rs/.nullclaw/doughcon-history.log" > /dev/null 2>&1; then
  echo "NOTE: history log lines differ only in the run timestamp — inspect manually:"
  tail -1 "$STAGE/py/.nullclaw/doughcon-history.log" 2>/dev/null
  tail -1 "$STAGE/rs/.nullclaw/doughcon-history.log" 2>/dev/null
fi

exit $fail
```

- [ ] **Step 3: Make it executable and run it**

```bash
chmod +x tools/differential/run.sh
cargo build --release
./tools/differential/run.sh
```
Expected: every case `ok`, except differences that are then justified in Step 4. Timestamp-bearing lines will differ by run time — that is expected and is why the history log is reported as a NOTE rather than a failure.

- [ ] **Step 4: Write the Intentional Differences list**

`docs/specs/2026-07-28-phase1-intentional-differences.md`:
```markdown
# Phase ① — intentional differences from the Python

Every entry is a deliberate decision. Anything not listed here is a bug.

1. **`emit_skill_status` cannot receive an invalid status.** Python raises
   `ValueError` before checking the env; Rust's typed `SkillStatus` makes it
   unrepresentable. The string boundary is still tested via `parse_status`.
2. **`deliver` returns an outcome instead of calling `sys.exit(1)`.** nullclaw
   classifies exit code and semantic status independently, so the binary owns
   the exit. Observable behaviour is identical.
3. **`SendOptions::base_url` and `DOUGHCON_BASE_URL` are test seams.** Both
   default to the production host, so scheduled runs are unaffected.
4. **A non-object PizzINT payload degrades instead of crashing.** Python would
   throw outside the fetch handler and exit hard; Rust rejects it at the adapter
   boundary and routes it to the same degraded path as a fetch failure. This is
   a compatibility fix, not a silent change.
5. **Telegram response bodies are still not parsed.** HTTP 200 is success. Rust
   must never start requiring `{"ok":true}`.
6. **Timestamps with a non-UTC offset** are accepted by both; a timezone-naive
   timestamp is rejected by `jiff` and falls back to run time, whereas Python
   would interpret it in the host's local zone. No PizzINT payload observed has
   ever been naive; recorded because it is unverified for all future payloads.
```

- [ ] **Step 5: Commit**

```bash
git add tools/differential docs/specs/2026-07-28-phase1-intentional-differences.md
git commit -m "test(phase1): differential harness against the Python oracle

Ported unit tests can preserve a shared misunderstanding, so the acceptance
oracle is the running Python. The harness serves each fixture from a local stub
and diffs exit code, stdout, and stderr for both implementations.

Every surviving difference is enumerated and justified; anything not on that
list is a bug."
```

---

### Task 10: nullclaw patch — supply the scheduled delivery budget (D9)

**Files:**
- Modify: `~/nullclaw/src/cron.zig` — add `SKILL_STARTED_ENV`, add `putSkillBudgetEnv`
- Modify: `~/nullclaw/src/gateway.zig` — call it on the scheduled skill spawn path

**⚠ This changes the behaviour of the existing Python skills**, activating a code path that has never run. It must land, and every affected job must fire at least once under it, **before** Task 12.

**⚠ CLOCK DOMAIN.** `NULLCLAW_SKILL_STARTED` must be `CLOCK_MONOTONIC` seconds — the same domain as Python's `time.monotonic()` and `claw_core::budget::monotonic_secs`. Writing a Unix epoch makes `elapsed` negative, which both consumers clamp to 0, so the failure is completely silent.

- [ ] **Step 1: Add the constant and helper in cron.zig**

Beside `const SKILL_TIMEOUT_ENV = "NULLCLAW_SKILL_TIMEOUT";` (`cron.zig:1520`):
```zig
const SKILL_STARTED_ENV = "NULLCLAW_SKILL_STARTED";

/// Publish the delivery budget to a skill child.
///
/// `started` MUST be CLOCK_MONOTONIC seconds — the same clock domain the Python
/// (`time.monotonic()`) and Rust (`clock_gettime(CLOCK_MONOTONIC)`) consumers
/// read. A wall-clock value makes their `now - started` hugely negative, which
/// both clamp to zero, so the budget silently degrades to "no elapsed time has
/// passed" and nothing looks wrong.
pub fn putSkillBudgetEnv(
    env_map: *std_compat.process.EnvMap,
    timeout_secs: u32,
    started_monotonic_s: f64,
) !void {
    var timeout_buf: [32]u8 = undefined;
    const timeout_str = std.fmt.bufPrint(&timeout_buf, "{d}", .{timeout_secs}) catch "120";
    try env_map.put(SKILL_TIMEOUT_ENV, timeout_str);

    var started_buf: [64]u8 = undefined;
    const started_str = std.fmt.bufPrint(&started_buf, "{d:.6}", .{started_monotonic_s}) catch "0";
    try env_map.put(SKILL_STARTED_ENV, started_str);
}

pub fn monotonicSeconds() f64 {
    const ts = std.posix.clock_gettime(std.posix.CLOCK.MONOTONIC) catch return 0;
    return @as(f64, @floatFromInt(ts.sec)) + @as(f64, @floatFromInt(ts.nsec)) / 1e9;
}
```

- [ ] **Step 2: Add a Zig test pinning the clock domain**

In `cron.zig`, beside the other tests:
```zig
test "monotonicSeconds is not a unix epoch timestamp" {
    const t = cron_mod.monotonicSeconds();
    try std.testing.expect(t > 0);
    // A wall-clock value in 2026 is ~1.78e9. CLOCK_MONOTONIC is seconds since
    // boot. If this fails, every delivery budget consumer is silently broken.
    try std.testing.expect(t < 1.0e9);
}

test "putSkillBudgetEnv writes both variables" {
    var env_map = std_compat.process.EnvMap.init(std.testing.allocator);
    defer env_map.deinit();
    try putSkillBudgetEnv(&env_map, 30, 12345.5);
    try std.testing.expectEqualStrings("30", env_map.get(SKILL_TIMEOUT_ENV).?);
    try std.testing.expectEqualStrings("12345.500000", env_map.get(SKILL_STARTED_ENV).?);
}
```

- [ ] **Step 3: Run the Zig tests to verify they pass**

Run: `cd ~/nullclaw && zig build test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 4: Call it on the scheduled skill path in gateway.zig**

At the scheduled skill spawn site — the one that calls `buildCronChildEnv` before launching `skill_cmd`, immediately above `classifySkillRun` (`gateway.zig` ~4524-4604) — capture the start time before spawning and set the budget:

```zig
const skill_started_s = cron_mod.monotonicSeconds();
var skill_env = try cron_mod.buildCronChildEnv(arena, .{
    .source = "cron_scheduler_skill",
    .trace_id = run_trace_id,
});
defer skill_env.deinit();
try cron_mod.putSkillBudgetEnv(&skill_env, timeout, skill_started_s);
```

Apply the same to the retry spawn, using a **fresh** `monotonicSeconds()` for the retry — the retry is a new run with its own budget, not a continuation.

- [ ] **Step 5: Build and verify the variables reach a child**

```bash
cd ~/nullclaw && zig build
# Point a scratch skill's ## Script at a shell script that dumps the env,
# schedule it one minute out, and read the captured stdout:
nullclaw cron list --all --json | head
```
Expected: the scheduled run's captured stdout contains both `NULLCLAW_SKILL_TIMEOUT` and `NULLCLAW_SKILL_STARTED`, and the started value is far below 1e9.

- [ ] **Step 6: Soak with the Python skills still live**

Do **not** proceed to Task 12 until every affected live job has fired at least once with no change in `last_status`. `inflation-con` (`0 6 3-5 * *`) is the slowest gate; a manual `nullclaw cron run` counts **only** if it goes through the scheduled env path, not `buildManualSkillChildEnv` — otherwise it does not exercise the change at all.

Record the before/after `last_status` for each job:
```bash
nullclaw cron list --all --json > /tmp/cron-before.json   # before deploying
# ... after each job has fired ...
nullclaw cron list --all --json > /tmp/cron-after.json
```

- [ ] **Step 7: Commit (in the nullclaw repo)**

```bash
cd ~/nullclaw
git add src/cron.zig src/gateway.zig
git commit -m "fix(cron): give scheduled skills the delivery budget they document

lib/delivery.py has always documented that the scheduler supplies
NULLCLAW_SKILL_TIMEOUT and NULLCLAW_SKILL_STARTED so the telegram retry loop
cannot starve a skill's own timeout. Only the manual path ever set the timeout,
and NULLCLAW_SKILL_STARTED was never set anywhere — so that branch has never
executed and scheduled deliveries silently fell back to a flat 30s cap.

STARTED is CLOCK_MONOTONIC seconds, matching what the Python and Rust consumers
read. A wall-clock value would make their elapsed calculation negative, both
clamp it to zero, and the budget would look healthy while doing nothing. Pinned
by a test."
```

---

### Task 11: Strict atomic installer

**Files:**
- Create: `tools/install-skill.sh`

`deploy.sh` never builds and nullclaw never checks that the script path exists, so a fresh clone would fail at fire time as `exec_error` — twice, because of `retry_once`. This installer is deliberately stricter than `deploy.sh`: it exits non-zero on any failure.

- [ ] **Step 1: Write the installer**

`tools/install-skill.sh`:
```bash
#!/usr/bin/env bash
# Build, verify, stage, and atomically publish a skill binary.
#
# Deliberately stricter than deploy.sh, which only does symlink bookkeeping and
# always exits 0. Nothing here is best-effort: if the artifact is missing or not
# executable, activation is REFUSED, because nullclaw does not check the path
# and would only discover it at fire time.
set -euo pipefail

skill=${1:?usage: install-skill.sh <skill-name>}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
dest_dir="$ROOT/$skill/bin"
dest="$dest_dir/$skill"
built="$ROOT/target/release/$skill"

echo "==> building $skill (locked)"
cargo build --release --locked -p "$skill"

[ -f "$built" ] || { echo "FAIL: $built not produced" >&2; exit 1; }
[ -x "$built" ] || { echo "FAIL: $built is not executable" >&2; exit 1; }

echo "==> staging"
mkdir -p "$dest_dir"
stage="$dest.staging.$$"
cp "$built" "$stage"
chmod +x "$stage"

echo "==> smoke-testing the staged artifact"
if ! "$stage" --mode record --et-hour 99 >/dev/null 2>&1; then
  rm -f "$stage"
  echo "FAIL: staged binary did not run cleanly (--et-hour 99 must be a no-op skip)" >&2
  exit 1
fi

echo "==> publishing atomically"
mv -f "$stage" "$dest"

echo "==> verifying the path nullclaw will resolve"
resolved="$HOME/.nullclaw/skills/$skill/bin/$skill"
[ -x "$resolved" ] || {
  echo "FAIL: $resolved is not executable — is the deploy.sh symlink in place?" >&2
  exit 1
}

echo "OK: $resolved"
echo "Activate by setting the SKILL.md '## Script' line to:"
echo "  ~/.nullclaw/skills/$skill/bin/$skill"
```

- [ ] **Step 2: Make executable and run it**

```bash
chmod +x tools/install-skill.sh
./tools/install-skill.sh doughcon
```
Expected: `OK: /home/yanggf/.nullclaw/skills/doughcon/bin/doughcon`

- [ ] **Step 3: Verify it refuses a missing artifact**

```bash
rm -f ~/a/claw-skills/doughcon/bin/doughcon target/release/doughcon
# Temporarily break the build to confirm refusal, e.g.:
mv crates/doughcon/src/main.rs /tmp/main.rs.bak
./tools/install-skill.sh doughcon; echo "exit=$?"
mv /tmp/main.rs.bak crates/doughcon/src/main.rs
```
Expected: non-zero exit, no file published, an explicit FAIL message.

- [ ] **Step 4: Commit**

```bash
git add tools/install-skill.sh
git commit -m "feat(tools): strict atomic skill-binary installer

deploy.sh only does symlink bookkeeping and always exits 0; nullclaw builds the
command string without checking the path exists. A fresh clone would therefore
fail at fire time as exec_error, twice, via retry_once.

This builds with --locked, verifies the artifact is executable, smoke-tests the
staged copy, publishes with an atomic rename, and re-checks the exact path
nullclaw will resolve. It refuses activation rather than leaving a broken job."
```

---

### Task 12: Cutover and live validation

**Files:**
- Modify: `doughcon/SKILL.md`

**Preconditions — do not start until all are true:** Task 9 passes with every difference justified; Task 10 has landed and soaked (Step 6); Task 11 published the binary successfully.

- [ ] **Step 1: Record the pre-cutover state**

```bash
nullclaw cron list --all --json > /tmp/doughcon-before.json
grep -c . ~/.nullclaw/doughcon-history.log
```

- [ ] **Step 2: Flip the Script line**

In `doughcon/SKILL.md`, replace:
```
~/.nullclaw/skills/doughcon/scripts/run.py
```
with:
```
~/.nullclaw/skills/doughcon/bin/doughcon
```

Leave `scripts/run.py` on disk — it is the rollback path.

- [ ] **Step 3: Verify nullclaw resolves the native command**

Run: `nullclaw cron list --all --json | grep -A2 doughcon | head -20`
Expected: the resolved command is the binary path with **no** `python3` prefix. If a prefix appears, the extension-inference rule did not apply — add `interpreter: native` to the SKILL.md frontmatter.

- [ ] **Step 4: Validate both firings and both job kinds**

doughcon has four jobs: two `--deliver-to … --et-hour 20` and two `--mode record --et-hour 20`, at 00:00 and 01:00. By design exactly one of each pair passes the gate; the other skips. Confirm **all four**:

```bash
nullclaw cron list --all --json > /tmp/doughcon-after.json
```
Expected, for each of the four jobs: `last_status=ok`. The two gated-out runs show `ok` with no body; the two that pass show the report.

- [ ] **Step 5: Confirm real-world effects**

- A Telegram message actually arrived for the deliver job.
- `~/.nullclaw/doughcon-history.log` gained **exactly one** line (record mode runs once per day after the gate).
- The trace id in the run's captured stdout matches that run.

- [ ] **Step 6: Verify rollback works**

Revert the `## Script` line to `scripts/run.py`, wait for one firing, confirm `last_status=ok`, then set it back to the binary. Rollback that has never been exercised is not a rollback.

- [ ] **Step 7: Commit**

```bash
git add doughcon/SKILL.md
git commit -m "feat(doughcon): cut over to the Rust binary

Validated on live cron: all four jobs report last_status=ok across both DST
candidate firings and both job kinds, Telegram delivery arrived, and the history
log gained exactly one line. Rollback was exercised, not assumed.

scripts/run.py stays on disk as the rollback path. Phase ① claims only that
native execution and the delivery/marker foundation are validated — the shared
contract is not proven until weather (Phase ②) exercises CLAW_ENV, partial
fallbacks, [skill-event], and semantic failed."
```

---

## Test Plan

**Layer 1 — ported unit tests (Tasks 1-6).** 47 Rust tests replacing the three Python test files, written before each implementation. Coverage the Python lacked and that these add: config-schema precedence and the mixed-schema fallback, empty-token handling, HTTP 408 being permanent, non-200 2xx, budget clock-domain, budget elapsed arithmetic, marker ordering, and the `Unmarked` path.

**Layer 2 — characterization tests (Task 7).** 15 Python tests written against the **existing** `run.py` before any Rust exists. They must pass against the Python first; if one fails, the test is wrong, not the Python.

**Layer 3 — differential harness (Task 9).** Seven fixture cases run through both implementations with a local stub server, diffing exit code, exact stdout, and exact stderr. This is the real oracle — Layers 1 and 2 can share a misunderstanding; this cannot.

**Layer 4 — Zig tests (Task 10).** Clock-domain guard plus the env-writing test.

**Layer 5 — live cron (Task 12).** All four doughcon jobs, both DST candidate firings, both job kinds, real Telegram receipt, history-log line count, and an exercised rollback.

**Commands:**
```bash
cargo test -- --test-threads=1                  # Layers 1, and doughcon report tests
python3 doughcon/tests/test_run_characterization.py  # Layer 2
cargo build --release && ./tools/differential/run.sh # Layer 3
cd ~/nullclaw && zig build test                 # Layer 4
```

`--test-threads=1` is required: several tests mutate process environment variables, which is global state.

## Acceptance Criteria

1. `cargo build --release` is clean — zero warnings.
2. All five test layers pass.
3. Every difference between the implementations appears in `docs/specs/2026-07-28-phase1-intentional-differences.md`.
4. The installer refuses to publish a missing or non-executable artifact (verified, Task 11 Step 3).
5. Task 10 landed and every affected live job fired at least once with unchanged `last_status`.
6. After cutover, all four doughcon jobs report `last_status=ok`, Telegram delivery arrived, and the history log gained exactly one line.
7. Rollback was exercised and observed working.
8. The Python `lib/` and `doughcon/scripts/run.py` are byte-identical to their pre-Phase-① state.

## Out of Scope

- Porting `skill_runner`, `heartbeat`, `news_quality`, `cover_image`, `oil_fetch`, `oil_store` — no Phase ① skill uses them.
- `CLAW_ENV` / dotenv resolution — doughcon does not read it; Phase ② (weather) needs it.
- Retiring the Python `lib/` — blocked on `cct` and `autocli`, which live outside this repo.
- Fixing `deploy.sh`'s unchecked `ln -s` and always-exit-0 behaviour.
- Any other skill.
