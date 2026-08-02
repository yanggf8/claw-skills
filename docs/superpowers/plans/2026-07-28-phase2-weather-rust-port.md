> **已完成並上線。**weather 的移植已在生產環境執行。
>
> 下面的核取方塊**在執行過程中從未被勾選**，所以它們不帶任何資訊 —— 讀作
> 「當時規劃的步驟」，不是「尚未完成的工作」。實際落地的內容以 `git log` 為準，
> 蒸餾後的常駐參考在 `docs/specs/*-intentional-differences.md`。
>
> 保留的理由是設計推理，不是待辦清單。

# claw-skills Rust Port — Phase ② Implementation Plan (weather)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port `weather` to Rust, grow `claw-core` with `env`/`agent`/`sanitize`, and close the three Phase ① test gaps that mutation testing proved real.

**Architecture:** Three new `claw-core` modules shared with the later `traffic` and `mindfulness-spirit` ports, plus a `weather` binary crate split so the fallback state machine is reachable from `cargo test` without a network — the correction to Phase ①, whose `main.rs` no unit test could touch.

**Tech Stack:** Rust 2021, `ureq` 2, `serde_json` (adapter boundary only), `regex` 1, `libc`. Python 3 stays the behaviour oracle.

**Spec:** `docs/specs/2026-07-28-phase2-weather-rust-port-design.md` — **read it first.** Its clauses **B1–B15** are the behaviour contract; tasks below reference them by number rather than restating them.

**Lessons:** `docs/specs/2026-07-28-phase1-lessons.md` — mandatory. Sections 1 and 4 are the ones this plan is shaped by.

## Global Constraints

- **`lib/*.py` and `weather/scripts/run.py` are FROZEN.** Do not edit, refactor, or delete. `cct` and `autocli` import `lib/` from outside this repo.
- **Behaviour parity is the bar.** Every difference goes in `docs/specs/2026-07-28-phase1-intentional-differences.md` (extend it; do not replace) or it is a bug.
- **B1–B15 are the contract.** Anything in the Rust that looks like it is fixing a bug in the Python is almost certainly one of them. Check before "improving".
- **`cargo test` must be sound WITHOUT `--test-threads=1`.** Phase ① shipped a race; per-call temp files and a lock over env mutation are the fix, and they are not optional.
- Zero compiler warnings. Agents must not run `git commit` — a human gates every commit.

### Why this plan specifies tests exhaustively and implementation sparsely

Phase ① put full implementations in the plan. Agents transcribed them faithfully — **including three bugs of mine**, which then shipped and had to be found by adversarial verification afterwards. Tests are the part that must be exact, because a wrong test is invisible; an implementation that is wrong fails a test that is right. So: **test code below is literal and must be used verbatim. Implementation is described by contract and B-clause, and the implementer writes it.**

---

## File Structure

**Create:**

| Path | Responsibility |
|---|---|
| `crates/claw-core/src/sanitize.rs` | `strip_agent_artifacts` — four substitutions + trim |
| `crates/claw-core/src/env.rs` | dotenv load (B11 char-set quote strip, B12 no-override) |
| `crates/claw-core/src/agent.rs` | agent subprocess (B13 exit code ignored), resolved via `HOME` |
| `crates/weather/src/routing.rs` | HK-vs-TW split, argument defaults |
| `crates/weather/src/sources/{hko,cwa,open_meteo}.rs` | three adapters; JSON stops here |
| `crates/weather/src/orchestrate.rs` | the fallback state machine (B1, B2) — no network |
| `crates/weather/src/main.rs` | thin: args → orchestrate → deliver → finish |
| `crates/doughcon/src/cli.rs` | doughcon's arg parsing + branch selection, extracted so `cargo test` reaches it |
| `tools/differential/sanitize_corpus/` | corpus files for the sanitizer differential |
| `tools/differential/weather.sh` | end-to-end differential harness |

**Modify:** `Cargo.toml` (add `crates/weather`), `crates/claw-core/src/lib.rs`, `crates/claw-core/tests/config.rs`, `crates/claw-core/tests/delivery.rs`, `crates/doughcon/src/main.rs`, `.gitignore`.

---

### Task 1: `claw-core::sanitize`

The highest-risk surface in Phase ②: 42 lines of regex shared by three skills, which exists because agent artifacts once leaked into Telegram. **Byte-exact or nothing.**

**Files:**
- Create: `crates/claw-core/src/sanitize.rs`, `crates/claw-core/tests/sanitize.rs`
- Modify: `crates/claw-core/src/lib.rs`, `crates/claw-core/Cargo.toml` (add `regex = "1"`)

**Interfaces:**
- Produces: `pub fn strip_agent_artifacts(text: &str, collapse_blank_lines: bool) -> String`

**Contract — the Python, in order.** Read `lib/skill_runner.py:211-252` before writing anything.

1. Remove paired `<ncchoices>…</ncchoices>` — **lazy**, dotall, case-insensitive.
2. Remove unclosed `<ncchoices>` through end of input — dotall, case-insensitive.
3. Remove whole lines matching `^\[(?:skill-status|trace|skill-event)[:\]].*$` — multiline. Note the character class is `:` **or** `]`, so a bare `[trace]` line also goes.
4. Remove whole lines matching `^\s*skill-[0-9a-f-]{8,}(?:-[0-9a-f]+)*:\d+\s*$` — multiline, case-insensitive.
5. If `collapse_blank_lines`, replace runs `\n\n+` with a single `\n`.
6. Return `.strip()`ed.

**Only the literal `ncchoices` tag is stripped.** `<25分鐘` and `>40分鐘` are legitimate advice and must survive — that is stated in the Python's own docstring.

- [ ] **Step 1: Write the failing tests**

`crates/claw-core/tests/sanitize.rs`:
```rust
use claw_core::sanitize::strip_agent_artifacts;

fn s(t: &str) -> String { strip_agent_artifacts(t, true) }

#[test]
fn removes_paired_ncchoices_case_insensitively() {
    assert_eq!(s("before<ncchoices>a\nb</ncchoices>after"), "beforeafter");
    assert_eq!(s("before<NCChoices>x</NCCHOICES>after"), "beforeafter");
}

#[test]
fn paired_match_is_lazy_not_greedy() {
    // Greedy would swallow everything between the FIRST open and the LAST close,
    // deleting "keep". Lazy keeps it.
    assert_eq!(
        s("<ncchoices>a</ncchoices>keep<ncchoices>b</ncchoices>"),
        "keep"
    );
}

#[test]
fn removes_unclosed_ncchoices_through_end_of_input() {
    // The model routinely drops the closing tag — this is the whole reason
    // rule 2 exists.
    assert_eq!(s("advice text\n<ncchoices>\n{\"a\":1}\nmore junk"), "advice text");
}

#[test]
fn leaves_other_angle_brackets_alone() {
    // TOKEN-SPECIFIC per the Python docstring: only `ncchoices` is a tag.
    assert_eq!(s("步行 <25分鐘，開車 >40分鐘"), "步行 <25分鐘，開車 >40分鐘");
    assert_eq!(s("<b>bold</b>"), "<b>bold</b>");
}

#[test]
fn removes_marker_lines_with_colon_or_bracket() {
    // The class is [:\]] — a bare "[trace]" line matches too.
    assert_eq!(s("keep\n[skill-status:ok]\n[trace:abc:1]\nkeep2"), "keep\nkeep2");
    assert_eq!(s("keep\n[trace]\nkeep2"), "keep\nkeep2");
    assert_eq!(s("keep\n[skill-event] fell back\nkeep2"), "keep\nkeep2");
}

#[test]
fn marker_removal_is_whole_line_only() {
    // Not anchored at line start => must NOT be removed.
    assert_eq!(s("prefix [skill-status:ok]"), "prefix [skill-status:ok]");
}

#[test]
fn removes_bare_job_id_lines() {
    assert_eq!(s("keep\nskill-b8993369-96fd-4890:3801\nkeep2"), "keep\nkeep2");
    assert_eq!(s("keep\n  SKILL-B8993369-96FD:12  \nkeep2"), "keep\nkeep2");
}

#[test]
fn short_hex_is_not_a_job_id() {
    // {8,} — seven hex chars must not match.
    assert_eq!(s("keep\nskill-abc1234:1\nkeep2"), "keep\nskill-abc1234:1\nkeep2");
}

#[test]
fn collapses_blank_line_runs_when_asked() {
    assert_eq!(s("a\n\n\n\nb"), "a\nb");
}

#[test]
fn preserves_blank_lines_when_not_collapsing() {
    // markdown-safe mode for article bodies
    assert_eq!(strip_agent_artifacts("a\n\n\nb", false), "a\n\n\nb");
}

#[test]
fn trims_edges() {
    assert_eq!(s("\n\n  advice  \n\n"), "advice");
}

#[test]
fn empty_and_artifact_only_inputs_yield_empty() {
    // This is what makes the advice line disappear — the caller checks `if advice:`.
    assert_eq!(s(""), "");
    assert_eq!(s("<ncchoices>only junk</ncchoices>"), "");
    assert_eq!(s("[skill-status:ok]\n[trace:x:1]"), "");
}

#[test]
fn cjk_and_emoji_survive_intact() {
    assert_eq!(s("👔 記得帶傘，早晚偏涼"), "👔 記得帶傘，早晚偏涼");
}

#[test]
fn python_strip_removes_more_than_unicode_whitespace() {
    // Python's str.strip() removes \x1c-\x1f (file/group/record/unit separators);
    // Rust's trim() uses the Unicode White_Space property, which does NOT include
    // them. If this test fails, the port used trim() where Python used strip()
    // and edge bytes will survive that the oracle removes.
    assert_eq!(s("\u{1c}advice\u{1f}"), "advice");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd ~/a/claw-skills && cargo test -p claw-core --test sanitize`
Expected: FAIL — `unresolved import claw_core::sanitize`

- [ ] **Step 3: Implement `sanitize.rs` to the contract above**

Write it yourself from the six numbered rules. Guidance, not code:
- `regex::Regex` with inline flags: `(?is)` for rules 1–2, `(?m)` for rule 3, `(?im)` for rule 4.
- Compile the five regexes once (`std::sync::OnceLock`), not per call.
- The last step must reproduce Python's `str.strip()`, **not** `trim()` — see the final test. Trim the Unicode-whitespace set *plus* `\u{1c}`–`\u{1f}` and `\u{85}`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p claw-core --test sanitize`
Expected: PASS, 14 tests.

- [ ] **Step 5: Prove the tests can fail**

Apply each mutation, confirm the named test goes red, then revert:
1. Change `.*?` to `.*` in rule 1 → `paired_match_is_lazy_not_greedy` must fail.
2. Delete rule 2 → `removes_unclosed_ncchoices_through_end_of_input` must fail.
3. Change `{8,}` to `{3,}` in rule 4 → `short_hex_is_not_a_job_id` must fail.
4. Replace the final strip with `.trim()` → `python_strip_removes_more_than_unicode_whitespace` must fail.

Record the four results. A mutation that does **not** turn a test red is a missing test, not a passing implementation.

- [ ] **Step 6: Report** (do not commit — a human gates commits)

---

### Task 2: `claw-core::env`

**Files:**
- Create: `crates/claw-core/src/env.rs`, `crates/claw-core/tests/env.rs`
- Modify: `crates/claw-core/src/lib.rs`

**Interfaces:**
- Produces: `pub fn load_env(explicit: Option<&std::path::Path>) -> ()` — sets process env vars as a side effect, matching Python's `os.environ` mutation.

**Contract:** `weather/scripts/run.py:62-75`. Path is `$CLAW_ENV` else `~/.nullclaw/.env`; a missing file is a silent no-op. Per line: `strip()`, then skip if empty, if it starts with `#`, or if it has no `=`. Split on the **first** `=`. Key: `strip()`. Value: `strip()` then `strip('"')` then `strip("'")` — **B11: successive character-set stripping, not paired-quote removal.** Set only if the key is absent from the environment — **B12**.

- [ ] **Step 1: Write the failing tests**

`crates/claw-core/tests/env.rs`:
```rust
use claw_core::env::load_env;
use std::io::Write;
use std::path::PathBuf;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn write(body: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("claw-core-env-{}-{}.env", std::process::id(), n));
    std::fs::File::create(&p).unwrap().write_all(body.as_bytes()).unwrap();
    p
}

#[test]
fn sets_keys_from_file() {
    let _g = guard();
    std::env::remove_var("CLAW_T_A");
    load_env(Some(&write("CLAW_T_A=hello\n")));
    assert_eq!(std::env::var("CLAW_T_A").unwrap(), "hello");
    std::env::remove_var("CLAW_T_A");
}

#[test]
fn never_overrides_an_existing_variable() {
    // B12. A value already in the environment (set by cron) must win.
    let _g = guard();
    std::env::set_var("CLAW_T_B", "from-cron");
    load_env(Some(&write("CLAW_T_B=from-file\n")));
    assert_eq!(std::env::var("CLAW_T_B").unwrap(), "from-cron");
    std::env::remove_var("CLAW_T_B");
}

#[test]
fn strips_quote_characters_successively_not_as_pairs() {
    // B11. Verified against Python: '"value' -> 'value' (UNPAIRED),
    // and '"\'value\'"' -> 'value' (both layers).
    let _g = guard();
    for (raw, want) in [
        ("CLAW_T_C=\"value\"", "value"),
        ("CLAW_T_C='value'", "value"),
        ("CLAW_T_C=\"value", "value"),
        ("CLAW_T_C=value\"", "value"),
        ("CLAW_T_C=\"'value'\"", "value"),
        ("CLAW_T_C=va\"lue", "va\"lue"),
    ] {
        std::env::remove_var("CLAW_T_C");
        load_env(Some(&write(&format!("{raw}\n"))));
        assert_eq!(std::env::var("CLAW_T_C").unwrap(), want, "input was {raw}");
    }
    std::env::remove_var("CLAW_T_C");
}

#[test]
fn skips_blank_comment_and_keyless_lines() {
    let _g = guard();
    std::env::remove_var("CLAW_T_D");
    load_env(Some(&write("\n   \n# CLAW_T_D=commented\nnoequalshere\nCLAW_T_D=real\n")));
    assert_eq!(std::env::var("CLAW_T_D").unwrap(), "real");
    std::env::remove_var("CLAW_T_D");
}

#[test]
fn splits_on_the_first_equals_only() {
    let _g = guard();
    std::env::remove_var("CLAW_T_E");
    load_env(Some(&write("CLAW_T_E=a=b=c\n")));
    assert_eq!(std::env::var("CLAW_T_E").unwrap(), "a=b=c");
    std::env::remove_var("CLAW_T_E");
}

#[test]
fn missing_file_is_a_silent_noop() {
    let _g = guard();
    load_env(Some(&PathBuf::from("/nonexistent/claw-core/none.env")));
}

#[test]
fn claw_env_variable_selects_the_path() {
    let _g = guard();
    let p = write("CLAW_T_F=via-env\n");
    std::env::set_var("CLAW_ENV", &p);
    std::env::remove_var("CLAW_T_F");
    load_env(None);
    assert_eq!(std::env::var("CLAW_T_F").unwrap(), "via-env");
    std::env::remove_var("CLAW_ENV");
    std::env::remove_var("CLAW_T_F");
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p claw-core --test env`, expect an unresolved import.
- [ ] **Step 3: Implement `env.rs` to the contract.** The quote handling is the whole trick: three successive `trim_matches` passes over single character sets, in the order whitespace → `"` → `'`.
- [ ] **Step 4: Run to verify it passes** — 7 tests.
- [ ] **Step 5: Prove they can fail.** Mutate to paired-quote removal → `strips_quote_characters_successively_not_as_pairs` red. Mutate to unconditional set → `never_overrides_an_existing_variable` red. Revert both.
- [ ] **Step 6: Report.**

---

### Task 3: `claw-core::agent`

**Files:**
- Create: `crates/claw-core/src/agent.rs`, `crates/claw-core/tests/agent.rs`
- Modify: `crates/claw-core/src/lib.rs`

**Interfaces:**
- Consumes: `sanitize::strip_agent_artifacts`
- Produces: `pub fn agent_binary_path() -> std::path::PathBuf` and `pub fn call_agent(prompt: &str, timeout: std::time::Duration) -> String` (empty string means "no advice").

**Contract:** `weather/scripts/run.py:210-236`.
- The binary is `<HOME>/nullclaw/zig-out/bin/nullclaw`, argv `["agent", "-m", prompt]`. **Resolving through `HOME` is load-bearing** — it is the shared injection seam the differential harness depends on. Do not replace it with a constant or a bespoke env var.
- Timeout 30s.
- **B13: the exit code is ignored.** Python reads `result.stdout` regardless of `returncode`.
- Output goes through `strip_agent_artifacts(.., true)`.
- Any failure — spawn error, timeout — prints `[WARN] LLM clothing advice failed: {e}` to **stderr** and yields an empty string.

- [ ] **Step 1: Write the failing tests**

`crates/claw-core/tests/agent.rs`:
```rust
use claw_core::agent::{agent_binary_path, call_agent};
use std::io::Write;
use std::time::Duration;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Plant a fake agent under a temporary HOME. This is exactly the seam the
/// differential harness uses, so exercising it here keeps it honest.
fn fake_home(script: &str) -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut home = std::env::temp_dir();
    home.push(format!("claw-core-home-{}-{}", std::process::id(), n));
    let bin = home.join("nullclaw/zig-out/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let p = bin.join("nullclaw");
    std::fs::File::create(&p).unwrap().write_all(script.as_bytes()).unwrap();
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    home
}

#[test]
fn resolves_the_binary_through_home() {
    let _g = guard();
    std::env::set_var("HOME", "/tmp/fake-home-probe");
    assert_eq!(
        agent_binary_path(),
        std::path::PathBuf::from("/tmp/fake-home-probe/nullclaw/zig-out/bin/nullclaw")
    );
}

#[test]
fn returns_sanitized_stdout() {
    let _g = guard();
    let home = fake_home("#!/bin/sh\nprintf 'take an umbrella<ncchoices>junk'\n");
    std::env::set_var("HOME", &home);
    assert_eq!(call_agent("p", Duration::from_secs(5)), "take an umbrella");
}

#[test]
fn ignores_a_nonzero_exit_code() {
    // B13. Python never checks returncode — stdout is used regardless. A Rust
    // port that treats exit != 0 as failure silently drops real advice.
    let _g = guard();
    let home = fake_home("#!/bin/sh\nprintf 'advice anyway'\nexit 3\n");
    std::env::set_var("HOME", &home);
    assert_eq!(call_agent("p", Duration::from_secs(5)), "advice anyway");
}

#[test]
fn empty_stdout_yields_empty_advice() {
    let _g = guard();
    let home = fake_home("#!/bin/sh\nexit 0\n");
    std::env::set_var("HOME", &home);
    assert_eq!(call_agent("p", Duration::from_secs(5)), "");
}

#[test]
fn missing_binary_yields_empty_advice_not_a_panic() {
    let _g = guard();
    std::env::set_var("HOME", "/nonexistent/claw-core-home");
    assert_eq!(call_agent("p", Duration::from_secs(5)), "");
}

#[test]
fn timeout_yields_empty_advice() {
    let _g = guard();
    let home = fake_home("#!/bin/sh\nsleep 30\n");
    std::env::set_var("HOME", &home);
    let t0 = std::time::Instant::now();
    assert_eq!(call_agent("p", Duration::from_millis(500)), "");
    assert!(t0.elapsed().as_secs() < 5, "must not wait for the child");
}

#[test]
fn prompt_reaches_the_child_as_a_single_argv_entry() {
    let _g = guard();
    let home = fake_home("#!/bin/sh\nprintf '%s' \"$3\"\n");
    std::env::set_var("HOME", &home);
    // argv is ["agent", "-m", prompt] => $3 is the prompt, intact with spaces.
    assert_eq!(call_agent("a b  c", Duration::from_secs(5)), "a b  c");
}
```

- [ ] **Step 2: Run to verify it fails.**
- [ ] **Step 3: Implement `agent.rs`.** `std::process::Command` has no timeout; spawn, poll `try_wait` with a short sleep until the deadline, then `kill`. Read stdout after the child ends.
- [ ] **Step 4: Run to verify it passes** — 7 tests.
- [ ] **Step 5: Prove they can fail.** Add an exit-code check → `ignores_a_nonzero_exit_code` red. Hardcode the path instead of using `HOME` → `resolves_the_binary_through_home` red. Revert.
- [ ] **Step 6: Report.**

---

### Task 4: Close the three Phase ① gaps

Independent of everything else — dispatchable in parallel with Tasks 1–3.

**Files:**
- Create: `crates/doughcon/src/cli.rs`, `crates/doughcon/tests/cli.rs`
- Modify: `crates/doughcon/src/main.rs`, `crates/doughcon/src/lib.rs`, `crates/claw-core/tests/config.rs`, `crates/claw-core/tests/delivery.rs`

Mutation testing proved all three gaps: deleting each implementation left the suite green.

- [ ] **Step 1: Add the `CLAW_CONFIG` test**

Append to `crates/claw-core/tests/config.rs` (it needs the same env lock the other suites use — add one if absent):
```rust
#[test]
fn claw_config_env_var_is_used_when_no_explicit_path() {
    let _g = guard();
    let p = write_tmp("via-env", r#"{"channels":{"telegram":{"botToken":"FROM_ENV"}}}"#);
    std::env::set_var("CLAW_CONFIG", &p);
    assert_eq!(get_bot_token("main", None).as_deref(), Some("FROM_ENV"));
    assert_eq!(resolve_config_path(None), p);
    std::env::remove_var("CLAW_CONFIG");
}
```

- [ ] **Step 2: Add the `parse_mode` plumbing test**

Append to `crates/claw-core/tests/delivery.rs`:
```rust
#[test]
fn parse_mode_actually_reaches_the_request() {
    // Deleting `parse_mode: opts.parse_mode.clone()` from deliver() previously
    // passed every test while silently removing Markdown from every message.
    let s = stub_server::start(vec![Some(200), Some(200)], 0);
    let mut o = opts(&s.base_url);
    let _ = run(Some("chat"), "body", &o);
    o.parse_mode = None;
    let _ = run(Some("chat"), "body", &o);
    assert!(s.body(0).contains("\"parse_mode\":\"Markdown\""), "default must survive deliver()");
    assert!(!s.body(1).contains("parse_mode"), "None must omit the key entirely");
}
```

- [ ] **Step 3: Extract doughcon's CLI into a testable module**

Move `parse_args`, its `Args` struct, and the DST-gate decision out of `main.rs` into `cli.rs`, exposing:
- `pub struct Args { pub mode: String, pub deliver_to: Option<String>, pub account: String, pub et_hour: Option<i32> }`
- `pub fn parse_args(argv: &[String]) -> Result<Args, String>` — takes argv rather than reading `std::env::args`, so it is testable.
- `pub enum Gate { Run, Skip { current_hour: i32, abbrev: String } }` and `pub fn gate(now_hour: i32, abbrev: &str, target: Option<i32>) -> Gate` — pure, no clock.

`main.rs` keeps only: read argv, read the clock, call `gate`, dispatch. Add `pub mod cli;` to `crates/doughcon/src/lib.rs`.

- [ ] **Step 4: Write the CLI tests**

`crates/doughcon/tests/cli.rs`:
```rust
use doughcon::cli::{gate, parse_args, Gate};

fn argv(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

#[test]
fn defaults_are_deliver_mode_and_main_account() {
    let a = parse_args(&argv(&[])).unwrap();
    assert_eq!(a.mode, "deliver");
    assert_eq!(a.account, "main");
    assert!(a.deliver_to.is_none());
    assert!(a.et_hour.is_none());
}

#[test]
fn parses_every_flag() {
    let a = parse_args(&argv(&["--mode", "record", "--deliver-to", "42", "--account", "nunu", "--et-hour", "20"])).unwrap();
    assert_eq!(a.mode, "record");
    assert_eq!(a.deliver_to.as_deref(), Some("42"));
    assert_eq!(a.account, "nunu");
    assert_eq!(a.et_hour, Some(20));
}

#[test]
fn et_hour_is_deliberately_not_range_checked() {
    // The Python's argparse does not validate 0-23; -1 and 99 are accepted and
    // become permanent skips. clap would "fix" this by default.
    assert_eq!(parse_args(&argv(&["--et-hour", "99"])).unwrap().et_hour, Some(99));
    assert_eq!(parse_args(&argv(&["--et-hour", "-1"])).unwrap().et_hour, Some(-1));
}

#[test]
fn rejects_unknown_flags_and_missing_values() {
    assert!(parse_args(&argv(&["--nope"])).is_err());
    assert!(parse_args(&argv(&["--mode"])).is_err());
    assert!(parse_args(&argv(&["--mode", "sideways"])).is_err());
}

#[test]
fn gate_runs_when_hour_matches_or_no_target() {
    assert!(matches!(gate(20, "EDT", Some(20)), Gate::Run));
    assert!(matches!(gate(4, "EDT", None), Gate::Run));
}

#[test]
fn gate_skip_carries_the_hour_and_abbreviation() {
    // The abbreviation is in the stderr line the Python emits; dropping it was
    // a real parity bug in Phase ①.
    match gate(4, "EDT", Some(20)) {
        Gate::Skip { current_hour, abbrev } => { assert_eq!(current_hour, 4); assert_eq!(abbrev, "EDT"); }
        Gate::Run => panic!("expected Skip"),
    }
}
```

- [ ] **Step 5: Run everything** — `cargo test` (no thread flag). Expect all suites green, doughcon `cli` 6 tests.
- [ ] **Step 6: Prove the three gaps are closed.** Apply and revert: delete the `CLAW_CONFIG` branch → the new config test red; drop `parse_mode` in `deliver()` → the new delivery test red; invert `gate`'s comparison → `gate_runs_when_hour_matches_or_no_target` red. **All three previously left the suite green — record the before/after.**
- [ ] **Step 7: Report.**

---

### Task 5: `weather::routing`

**Files:** Create `crates/weather/Cargo.toml`, `crates/weather/src/lib.rs`, `crates/weather/src/routing.rs`, `crates/weather/tests/routing.rs`. Modify root `Cargo.toml` (add the member **after** creating the directory — cargo hard-errors on a missing member).

**Interfaces:**
- Produces: `pub fn is_hk(loc: &str) -> bool`, `pub fn split(locations: &[String]) -> (Vec<String>, Vec<String>)` returning `(hk, tw)` **preserving input order and duplicates**, and `pub fn with_default(locations: Vec<String>) -> Vec<String>`.

**Contract:** `run.py:77-81` and the `locations = args.locations or ["臺北市"]` line. The HK set is exactly `{"香港", "hong kong", "hk", "九龍", "新界", "港島"}`, matched on `loc.to_lowercase().trim()`. Everything else is TW. Both `None` and an **empty list** take the default (B: empty is falsy in Python).

- [ ] **Step 1: Write the failing tests**

`crates/weather/tests/routing.rs`:
```rust
use weather::routing::{is_hk, split, with_default};

fn v(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

#[test]
fn hk_membership_is_the_closed_set_after_lowercase_trim() {
    for s in ["香港", "hong kong", "HK", " hk ", "九龍", "新界", "港島"] {
        assert!(is_hk(s), "{s} should be HK");
    }
    for s in ["臺北市", "台北", "Hong Kong Island", "hk1", "九龍城"] {
        assert!(!is_hk(s), "{s} should be TW");
    }
}

#[test]
fn split_preserves_order_and_duplicates() {
    // B4: repeated HK aliases each produce their own line later, so the split
    // must not deduplicate.
    let (hk, tw) = split(&v(&["香港", "臺北市", "hk", "香港", "新北市"]));
    assert_eq!(hk, v(&["香港", "hk", "香港"]));
    assert_eq!(tw, v(&["臺北市", "新北市"]));
}

#[test]
fn empty_input_defaults_to_taipei() {
    assert_eq!(with_default(vec![]), v(&["臺北市"]));
    assert_eq!(with_default(v(&["高雄市"])), v(&["高雄市"]));
}
```

- [ ] **Step 2–4:** run red, implement, run green (3 tests).
- [ ] **Step 5: Prove they can fail.** Deduplicate in `split` → `split_preserves_order_and_duplicates` red. Revert.
- [ ] **Step 6: Report.**

---

### Task 6: `weather::sources`

**Files:** Create `crates/weather/src/sources/{mod,hko,cwa,open_meteo}.rs` and `crates/weather/tests/sources.rs`.

**Interfaces:**
- Produces, per source: `parse_*(body: &str) -> Result<..., String>` and `format_*(loc_name: &str, parsed: &..) -> (String, Option<Row>)`, plus `fetch_*(base_url: Option<&str>, ..) -> Result<.., String>` with a **base-url test seam**. `Row` is the advice-summary record: `pub struct Row { pub location: String, pub wx: String, pub min_t: String, pub max_t: String, pub pop: String }`.
- `None` for the row means "not counted toward `weather_data`" — HKO and Open-Meteo return `None` on the WARN paths (B7).

**Contract:** B3, B5, B6, B9, B10, B14, B15, and `run.py:83-208`. **Read those lines.** Highlights the implementer will get wrong otherwise: HKO always names the row `香港` regardless of `loc_name` (B3); rain formatting differs three ways (B5); Open-Meteo rounds **half-to-even** (B6); timeouts are 20/8/8 (B10); `"0"` is truthy (B14); CWA picks the slot nearest `now` in UTC+8 (B15).

- [ ] **Step 1: Write the failing tests**

`crates/weather/tests/sources.rs`:
```rust
use weather::sources::{cwa, hko, open_meteo};

#[test]
fn hko_line_always_says_hong_kong() {
    // B3: the requested name is ignored, in BOTH the line and the row.
    let body = r#"{"weatherForecast":[{"forecastWeather":"多雲","forecastMintemp":{"value":24},"forecastMaxtemp":{"value":30},"PSR":"高"}]}"#;
    let parsed = hko::parse(body).unwrap();
    let (line, row) = hko::format("九龍", &parsed);
    assert!(line.starts_with("🌤 香港："), "line was {line}");
    assert_eq!(row.as_ref().unwrap().location, "香港");
}

#[test]
fn hko_rain_has_no_percent_sign() {
    // B5: HKO uses 降雨概率{psr} with NO %, because PSR is qualitative.
    let body = r#"{"weatherForecast":[{"forecastWeather":"多雲","forecastMintemp":{"value":24},"forecastMaxtemp":{"value":30},"PSR":"高"}]}"#;
    let (line, _) = hko::format("香港", &hko::parse(body).unwrap());
    assert!(line.contains("降雨概率高"), "line was {line}");
    assert!(!line.contains("降雨概率高%"), "must not add a percent sign");
}

#[test]
fn hko_missing_temps_render_as_question_marks() {
    let body = r#"{"weatherForecast":[{"forecastWeather":"晴"}]}"#;
    let (line, row) = hko::format("香港", &hko::parse(body).unwrap());
    assert!(line.contains("低溫?°C"), "line was {line}");
    assert_eq!(row.unwrap().min_t, "?");
}

#[test]
fn hko_empty_forecast_warns_and_yields_no_row() {
    // B7: no row => does not count toward weather_data => can drive status to failed.
    let (line, row) = hko::format("香港", &hko::parse(r#"{"weatherForecast":[]}"#).unwrap());
    assert_eq!(line, "[WARN: HKO forecast unavailable for 香港]");
    assert!(row.is_none());
}

#[test]
fn cwa_null_location_key_is_an_empty_list_not_an_error() {
    // B9: `.get("location", []) or []` — present-but-null behaves as missing.
    assert_eq!(cwa::records(r#"{"records":{"location":null}}"#).unwrap().len(), 0);
    assert_eq!(cwa::records(r#"{"records":{}}"#).unwrap().len(), 0);
}

#[test]
fn open_meteo_rounds_half_to_even_like_python() {
    // B6. Verified: python round(24.5)==24 and round(26.5)==26, while Rust's
    // f64::round gives 25 and 27. A one-degree divergence on every .5 temp.
    assert_eq!(open_meteo::round_like_python(24.5), 24);
    assert_eq!(open_meteo::round_like_python(25.5), 26);
    assert_eq!(open_meteo::round_like_python(26.5), 26);
    assert_eq!(open_meteo::round_like_python(-24.5), -24);
    assert_eq!(open_meteo::round_like_python(24.4), 24);
    assert_eq!(open_meteo::round_like_python(24.6), 25);
}

#[test]
fn open_meteo_keeps_a_zero_rain_probability() {
    // B14: "0" is truthy in Python, so the field is rendered.
    let body = r#"{"daily":{"weather_code":[1],"temperature_2m_max":[30.0],"temperature_2m_min":[24.0],"precipitation_probability_max":[0]}}"#;
    let (line, _) = open_meteo::format("臺北市", &open_meteo::parse(body).unwrap());
    assert!(line.contains("降雨機率0%"), "line was {line}");
}

#[test]
fn open_meteo_success_line_is_suffixed_as_fallback() {
    let body = r#"{"daily":{"weather_code":[1],"temperature_2m_max":[30.0],"temperature_2m_min":[24.0],"precipitation_probability_max":[10]}}"#;
    let (line, row) = open_meteo::format("臺北市", &open_meteo::parse(body).unwrap());
    assert!(line.ends_with("（備援）"), "line was {line}");
    assert!(row.is_some());
}

#[test]
fn open_meteo_missing_arrays_warn_and_yield_no_row() {
    let (line, row) = open_meteo::format("臺北市", &open_meteo::parse(r#"{"daily":{}}"#).unwrap());
    assert_eq!(line, "[WARN: Open-Meteo forecast unavailable for 臺北市]");
    assert!(row.is_none());
}

#[test]
fn cwa_slot_selection_is_stable_for_a_past_only_fixture() {
    // B15: the picker reads the wall clock. Fixtures must be built so the
    // choice is time-invariant; this asserts the fixture shape the differential
    // harness depends on. If this test starts flaking, the fixture is wrong,
    // not the code.
    let body = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/differential/fixtures/cwa_past_only.json")
    ).unwrap();
    let recs = cwa::records(&body).unwrap();
    let (line_a, _) = cwa::format("臺北市", &recs[0]);
    let (line_b, _) = cwa::format("臺北市", &recs[0]);
    assert_eq!(line_a, line_b);
    assert!(line_a.contains("臺北市"), "line was {line_a}");
}
```

- [ ] **Step 2–4:** run red, implement each source to its `run.py` lines, run green (10 tests). Create `tools/differential/fixtures/cwa_past_only.json` with every `startTime` in 2020 so slot selection cannot depend on today's date.
- [ ] **Step 5: Prove they can fail.** Use `loc_name` in the HKO line → `hko_line_always_says_hong_kong` red. Use `f64::round` → `open_meteo_rounds_half_to_even_like_python` red. Add `%` to the HKO rain → `hko_rain_has_no_percent_sign` red. Revert all three.
- [ ] **Step 6: Report.**

---

### Task 7: `weather::orchestrate` — the fallback state machine

**The task this whole phase exists to get right.** B1 and B2 both live here, and both are behaviours a competent Rust developer would remove on sight.

**Files:** Create `crates/weather/src/orchestrate.rs`, `crates/weather/tests/orchestrate.rs`.

**Interfaces:**
- Produces:
  ```rust
  pub struct Outcome { pub lines: Vec<String>, pub rows: Vec<Row>, pub fallback_used: bool, pub fallback_event: Option<FallbackEvent> }
  pub struct FallbackEvent { pub reason: String, pub scope: String }
  pub trait Sources {                       // injected so tests need no network
      fn hko(&self) -> Result<HkoData, String>;
      fn cwa(&self, locs: &[String]) -> Result<String, String>;   // raw body
      fn open_meteo(&self, loc: &str) -> Result<OmData, String>;
  }
  pub fn run(hk: &[String], tw: &[String], api_key: &str, src: &dyn Sources) -> Outcome
  ```
- `status_of(&Outcome) -> SkillStatus` lives here too, so B2's ordering is unit-testable.

**Contract — read `run.py:262-336` line by line.** The three that matter:

- **B8** — an empty `CWA_API_KEY` is the same as unset; both set `cwa_failed_reason = "CWA_API_KEY is not set in the environment"`.
- **B1** — the per-location format loop runs *inside* the same fallible region as the fetch. Lines and rows already pushed **stay**, and on error `targets` becomes **all** of `tw`, not just the unmatched ones. That produces duplicate lines for locations that already succeeded, and that is correct.
- **B2** — `fallback_used` is set whenever `targets` is non-empty, **regardless of whether Open-Meteo produced anything**; and `status_of` checks "no rows → failed" **before** "fallback_used → degraded".

- [ ] **Step 1: Write the failing tests**

`crates/weather/tests/orchestrate.rs`:
```rust
use weather::orchestrate::{run, status_of, Sources};
use claw_core::marker::SkillStatus;

fn v(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

/// Scriptable fake. `cwa_body` None means the fetch itself fails.
struct Fake {
    cwa_body: Option<String>,
    om_ok: bool,
    /// When set, formatting this location panics the way a malformed record does.
    cwa_poison: Option<String>,
}
// (the implementer wires Fake to the Sources trait; `cwa_poison` must surface as
//  an Err from the format step INSIDE the same region as the fetch, per B1)

#[test]
fn happy_path_is_ok_with_no_fallback() {
    let out = run(&[], &v(&["臺北市"]), "key", &Fake::cwa_with(&["臺北市"]));
    assert_eq!(out.lines.len(), 1);
    assert_eq!(out.rows.len(), 1);
    assert!(!out.fallback_used);
    assert!(out.fallback_event.is_none());
    assert_eq!(status_of(&out), SkillStatus::Ok);
}

#[test]
fn empty_api_key_is_treated_as_unset() {
    // B8
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_ok());
    assert!(out.fallback_used);
    assert_eq!(
        out.fallback_event.unwrap().reason,
        "CWA_API_KEY is not set in the environment"
    );
}

#[test]
fn partial_match_falls_back_only_for_unmatched() {
    let out = run(&[], &v(&["臺北市", "高雄市"]), "key", &Fake::cwa_with(&["臺北市"]));
    assert!(out.fallback_used);
    assert_eq!(
        out.fallback_event.unwrap().reason,
        "CWA did not return data for 1 of 2 locations"
    );
    assert_eq!(out.lines.len(), 2, "one CWA line + one Open-Meteo line");
}

#[test]
fn partial_progress_then_error_keeps_the_lines_and_refetches_everything() {
    // B1 — THE case. 臺北市 formats fine, 高雄市 raises. The Python keeps the
    // 臺北市 CWA line AND falls back for BOTH locations, so 臺北市 appears twice.
    // A Rust `?` would drop the first line; falling back only for unmatched
    // would drop the duplicate. Both are wrong.
    let out = run(&[], &v(&["臺北市", "高雄市"]), "key", &Fake::cwa_poison_on("高雄市"));
    let taipei_lines = out.lines.iter().filter(|l| l.contains("臺北市")).count();
    assert_eq!(taipei_lines, 2, "expected the CWA line AND its Open-Meteo duplicate");
    assert!(out.fallback_event.unwrap().reason.starts_with("CWA request failed with"));
}

#[test]
fn fallback_used_is_set_even_when_every_open_meteo_call_fails() {
    // B2, first half.
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_all_fail());
    assert!(out.fallback_used, "the attempt counts, not its success");
    assert!(out.fallback_event.is_some(), "the [skill-event] is still emitted");
    assert!(out.rows.is_empty());
}

#[test]
fn failed_outranks_degraded() {
    // B2, second half. No rows => failed, even though fallback_used is true.
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_all_fail());
    assert_eq!(status_of(&out), SkillStatus::Failed);
}

#[test]
fn degraded_when_the_fallback_produced_something() {
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_ok());
    assert_eq!(status_of(&out), SkillStatus::Degraded);
}

#[test]
fn scope_is_singular_for_one_location() {
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_ok());
    assert_eq!(out.fallback_event.unwrap().scope, "1 Taiwan location");
    let out2 = run(&[], &v(&["臺北市", "高雄市"]), "", &Fake::om_ok());
    assert_eq!(out2.fallback_event.unwrap().scope, "2 Taiwan locations");
}

#[test]
fn hk_locations_never_trigger_a_fallback_event() {
    let out = run(&v(&["香港"]), &[], "key", &Fake::hko_ok());
    assert!(!out.fallback_used);
    assert!(out.fallback_event.is_none());
}

#[test]
fn repeated_hk_aliases_produce_one_line_each_from_one_fetch() {
    // B4
    let out = run(&v(&["香港", "九龍", "香港"]), &[], "key", &Fake::hko_ok());
    assert_eq!(out.lines.len(), 3);
    assert_eq!(out.rows.len(), 3);
}

#[test]
fn empty_records_uses_the_empty_list_reason() {
    let out = run(&[], &v(&["臺北市"]), "key", &Fake::cwa_empty_records());
    assert_eq!(out.fallback_event.unwrap().reason, "CWA returned an empty record list");
}
```

- [ ] **Step 2: Run to verify it fails.**
- [ ] **Step 3: Implement `orchestrate.rs`.** Mirror the Python's control flow shape — accumulate into `lines`/`rows` as you go, and let the error path set a reason without unwinding what was already pushed. Do **not** use `?` across the format loop.
- [ ] **Step 4: Run to verify it passes** — 11 tests.
- [ ] **Step 5: Prove they can fail.** Three mutations, each must turn exactly the named test red, then revert:
  1. Use `?` so a format error abandons the CWA branch → `partial_progress_then_error_keeps_the_lines_and_refetches_everything` red.
  2. Set `fallback_used` only when Open-Meteo returned rows → `fallback_used_is_set_even_when_every_open_meteo_call_fails` red.
  3. Check `fallback_used` before "no rows" in `status_of` → `failed_outranks_degraded` red.
- [ ] **Step 6: Report.**

---

### Task 8: `weather::main`

**Files:** Create `crates/weather/src/main.rs`, `crates/weather/tests/cli.rs`.

**Interfaces:** Consumes everything above. `main.rs` stays thin: parse argv, `load_env`, read `CWA_API_KEY`, build the real `Sources`, call `orchestrate::run`, append the advice line, append the job-id footer, deliver, `finish`.

**Contract:** `run.py:238-341`. The ordering is load-bearing:
1. `[WARN: no valid locations provided]` if `lines` is empty.
2. Advice **only if** `rows` is non-empty; then only if the sanitized advice is non-empty (B13).
3. Job-id footer `\n\n`+backticks appended to the **whole body**, after the advice.
4. `deliver`, then `status_of`, then markers.
5. A `FailedFatal` delivery exits 1 **before** markers — same rule as doughcon.

Argument parsing mirrors Task 4's shape: `pub fn parse_args(argv: &[String]) -> Result<Args, String>` in a `cli.rs`, so `cargo test` reaches it. `--location` is repeatable.

- [ ] **Step 1: Write the failing tests** — `crates/weather/tests/cli.rs` covering: repeated `--location` accumulates in order; `--deliver-to` / `--account` parse; unknown flag and missing value error; empty argv yields an empty location vec (the default is applied later by `with_default`, matching the Python's `or`).
- [ ] **Step 2–4:** run red, implement, run green.
- [ ] **Step 5: Report.**

---

### Task 9: sanitizer corpus differential

**Files:** Create `tools/differential/sanitize_corpus/*.txt` and `tools/differential/sanitize.sh`.

The single highest-value test in this phase: 42 lines of regex, three consumers, and a failure mode users see directly.

- [ ] **Step 1: Build the corpus.** At least 20 files, each one input. Must include: closed `<ncchoices>`; **unclosed** `<ncchoices>`; two `<ncchoices>` blocks in one input; mixed case tags; `<25分鐘` / `>40分鐘`; `<b>` HTML; all three marker line forms; a bare `[trace]`; a bare job-id line; a 7-hex-char near-miss; runs of 2/3/5 blank lines; leading and trailing whitespace; `\x1c`/`\x1f` edge bytes; CJK; emoji; empty file; whitespace-only file; a realistic multi-line clothing advice with a trailing marker block.
- [ ] **Step 2: Write the harness.** For each corpus file, run it through Python (`python3 -c` importing `lib/skill_runner.py`) and through a tiny Rust bin (`cargo run -p claw-core --example sanitize_stdin` — add that example), and `diff` the two outputs byte for byte. Run each file twice, once per `collapse_blank_lines` value.
- [ ] **Step 3: Run it.** Expected: zero diffs. Any diff is a finding — report it, do not adjust the corpus to hide it.
- [ ] **Step 4: Report.**

---

### Task 10: end-to-end differential harness

**Files:** Create `tools/differential/weather.sh`, `tools/differential/weather_cases.tsv`, `tools/differential/fixtures/*.json`.

Reuses the Phase ① pattern with three changes: three HTTP stubs instead of one, a fake agent planted under a temporary `HOME`, and two masks.

- [ ] **Step 1: Build the stub layer.** One local HTTP server per source, each scriptable to success / failure / partial. The Python side reuses the Phase ① trick — monkeypatch `urllib.request.Request` — but must now route by URL substring (`hko`, `opendata.cwa`, `open-meteo`) rather than redirecting everything to one stub.
- [ ] **Step 2: Plant the fake agent.** `$STAGE/nullclaw/zig-out/bin/nullclaw` printing a fixed string, with `HOME=$STAGE`. **Verify both sides actually use it** — assert the fixed string appears in both outputs before trusting any case. If it does not, the seam is broken and every later "pass" is meaningless.
- [ ] **Step 3: Apply the two masks before diffing.** Replace `and took \d+ms` with `and took <MS>ms`, and any `CWA request failed with \w+: .*` / `[WARN: Open-Meteo unavailable for … - .*]` tail with a placeholder — exception text differs between runtimes and is a declared intentional difference.
- [ ] **Step 4: Write the cases.** HK only; TW only; mixed; repeated HK aliases (B4); CWA total failure; CWA partial (B); **CWA partial-then-error (B1)**; Open-Meteo total failure (B2); empty `CWA_API_KEY` (B8); `location: null` (B9); a `.5` temperature (B6); a zero rain probability (B14); no `NULLCLAW_JOB_ID`; a delivery-failure exit path.
- [ ] **Step 5: Run and report every diff.** Do not silence a diff by editing a fixture.
- [ ] **Step 6: Extend the intentional-differences doc** with anything that survives, and report.

---

### Task 11: cutover and live validation — HUMAN GATED

**Do not execute this task in a workflow.** It changes live behaviour.

- [ ] Build and publish via `tools/install-skill.sh weather`.
- [ ] Record `nullclaw cron list --all --json` before.
- [ ] Flip `weather/SKILL.md`'s `## Script` to `~/.nullclaw/skills/weather/bin/weather`.
- [ ] Verify nullclaw resolves the native command with no `python3` prefix.
- [ ] Watch all four active jobs plus one temporarily-unpaused `--account nunu` job — the first real exercise of non-`main` account resolution.
- [ ] Confirm a real Telegram message arrives, with the clothing advice line present and free of artifacts.
- [ ] Exercise rollback by reverting the one line, observing a clean Python run, then restoring.

---

## Test Plan

| Layer | What | Where | Gate |
|---|---|---|---|
| L1 | Unit, tests-first, per component | `crates/*/tests/*.rs` | Every task's Step 5 proves the tests can fail by mutation |
| L2 | Sanitizer corpus differential | `tools/differential/sanitize.sh` | Zero byte diffs across ≥20 inputs × 2 modes |
| L3 | End-to-end differential | `tools/differential/weather.sh` | Zero diffs after the two declared masks |
| L4 | Live cron | Task 11 | Four jobs + one `nunu` job `ok`; real Telegram; rollback exercised |

**Commands:**
```bash
cd ~/a/claw-skills
cargo test                                   # L1 — MUST be green without --test-threads=1
./tools/differential/sanitize.sh             # L2
cargo build --release && ./tools/differential/weather.sh   # L3
```

**Mutation gate.** Every task lists specific mutations and the test each must turn red. Phase ① shipped a suite where `.all()` → `.any()` passed everything; the mutation step is what catches that, and it is not optional. A mutation that leaves the suite green means the test is missing — write it before moving on.

## Acceptance Criteria

1. `cargo test` green **without** `--test-threads=1`, zero warnings on a forced full rebuild.
2. Every mutation listed in Steps 5 turns exactly the named test red; results recorded.
3. L2 zero diffs; L3 zero diffs after the two declared masks.
4. The three Phase ① gaps demonstrably closed — each mutation now red where it was previously green.
5. B1 and B2 each covered by a test that fails under the obvious Rust rewrite.
6. The frozen files are byte-identical to their pre-Phase-② state.

## Out of Scope

`traffic` and `mindfulness-spirit` (they reuse `claw-core::sanitize` later); the legacy nullclaw scheduler spawn path; retiring the Python `lib/`; the remaining Phase ① NITs (data-array-of-scalars divergence, i64-overflow index, `/tmp` test litter).
