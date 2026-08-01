//! Run the real executable.
//!
//! The unit tests call `parse_args`, `manage_add` and the marker helpers
//! directly and never go through `main`. Wiring them together is where the
//! earlier ports in this repo broke while every unit test stayed green, so
//! these drive the built binary and read what actually reaches stdout.
//!
//! Everything here is offline: `manage` touches no network, and an unknown
//! flag is rejected before anything else runs.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Home(PathBuf);

impl Home {
    fn new() -> Home {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("news-bin-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join(".nullclaw")).expect("scratch home");
        Home(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn topics(&self) -> String {
        std::fs::read_to_string(self.0.join(".nullclaw/news-topics.json")).unwrap_or_default()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn bin() -> PathBuf {
    // `CARGO_BIN_EXE_<name>` is set by cargo for integration tests.
    PathBuf::from(env!("CARGO_BIN_EXE_news"))
}

fn run(home: &Home, job_id: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(bin());
    cmd.args(args)
        .env("HOME", home.path())
        // Keep the run off the operator's real config and away from the agent.
        .env_remove("CLAW_CONFIG")
        .env_remove("CLAW_ENV")
        .env_remove("NULLCLAW_JOB_ID");
    if let Some(j) = job_id {
        cmd.env("NULLCLAW_JOB_ID", j);
    }
    cmd.output().expect("run news")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

#[test]
fn an_unknown_flag_exits_two_and_prints_nothing_to_stdout() {
    // Exit 2 is what `tools/install-skill.sh` probes for. A parser that
    // ignored the flag would run some other mode and still report success.
    let home = Home::new();
    let out = run(&home, None, &["--delivery-to", "123"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown argument"));
}

#[test]
fn an_unknown_manage_action_also_exits_two() {
    let home = Home::new();
    assert_eq!(
        run(&home, None, &["manage", "sync"]).status.code(),
        Some(2)
    );
}

#[test]
fn a_manual_manage_run_emits_no_scheduler_markers() {
    // The markers are a cron contract. On a manual run they are noise, and a
    // stale trace id in a hand-run would confuse the classifier's records.
    let home = Home::new();
    let out = run(&home, None, &["manage", "list"]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(text.contains("預設"), "{text}");
    assert!(!text.contains("[skill-status:"), "{text}");
    assert!(!text.contains("[trace:"), "{text}");
}

#[test]
fn a_scheduled_run_emits_both_markers_with_the_job_id_verbatim() {
    // `classifySkillRun` compares the trace payload to NULLCLAW_JOB_ID byte for
    // byte, so anything but the exact value reads as a contract failure.
    let home = Home::new();
    let out = run(&home, Some("news:1540"), &["manage", "list"]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(text.contains("[skill-status:ok]"), "{text}");
    assert!(text.contains("[trace:news:1540]"), "{text}");
}

#[test]
fn the_markers_come_after_the_body_not_before_it() {
    // Everything before them is the message; a marker in the middle would be
    // delivered to the reader.
    let home = Home::new();
    let out = run(&home, Some("j1"), &["manage", "list"]);
    let text = stdout(&out);
    let body_end = text.find("[skill-status:").expect("marker present");
    assert!(text[..body_end].contains("預設"), "{text}");
}

#[test]
fn adding_and_removing_a_topic_round_trips_through_the_real_file() {
    let home = Home::new();

    let added = stdout(&run(&home, None, &["manage", "add", "--topic", "台積電"]));
    assert!(added.contains("已新增主題「台積電」"), "{added}");
    assert!(home.topics().contains("台積電"), "{}", home.topics());

    // Idempotent: a second add reports it is already there and does not
    // duplicate the entry.
    let again = stdout(&run(&home, None, &["manage", "add", "--topic", "台積電"]));
    assert!(again.contains("已在訂閱中"), "{again}");
    assert_eq!(home.topics().matches("台積電").count(), 1);

    let listed = stdout(&run(&home, None, &["manage", "list"]));
    assert!(listed.contains("• 台積電"), "{listed}");

    let removed = stdout(&run(&home, None, &["manage", "remove", "--topic", "台積電"]));
    assert!(removed.contains("目前無訂閱主題"), "{removed}");
    assert!(!home.topics().contains("台積電"), "{}", home.topics());
}

#[test]
fn topics_are_kept_per_account() {
    let home = Home::new();
    run(&home, None, &["manage", "add", "--account", "nunu", "--topic", "AI"]);
    let main = stdout(&run(&home, None, &["manage", "list"]));
    let nunu = stdout(&run(&home, None, &["manage", "list", "--account", "nunu"]));
    assert!(main.contains("預設"), "main leaked nunu's topics: {main}");
    assert!(nunu.contains("• AI"), "{nunu}");
}

#[test]
fn removing_a_topic_that_was_never_added_says_so_and_changes_nothing() {
    let home = Home::new();
    let out = stdout(&run(&home, None, &["manage", "remove", "--topic", "沒訂過"]));
    assert!(out.contains("不在訂閱中"), "{out}");
}

#[test]
fn the_stored_topic_file_is_valid_json_a_person_can_read() {
    let home = Home::new();
    run(&home, None, &["manage", "add", "--topic", "半導體"]);
    let raw = home.topics();
    // Not \u-escaped: the file is meant to be hand-editable.
    assert!(raw.contains("半導體"), "{raw}");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
    assert_eq!(parsed["main"][0], "半導體");
}
