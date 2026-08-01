//! liko-finance-weekly — draft, validate and publish the weekly issue.
//!
//! Ports liko-finance-weekly/scripts/run.py. Almost all of this skill is
//! orchestration: persona-core owns the data and the validator, an agent writes
//! the prose, and this binary sequences them and reports to the scheduler.
//!
//! The sequence, unchanged:
//!
//! ```text
//! doctor → prepare issue → already published? stop
//!        → load context → draft → validate
//!        → failed? one repair pass → validate again
//!        → still failed? mark skipped, report failed
//!        → passed? update-body → publish
//! ```
//!
//! One repair pass, not a loop: a validator that rejects the same body twice is
//! reporting something the model cannot fix by trying harder.

use std::io::Write;
use std::time::Duration;

use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};
use liko_finance_weekly::issues::{already_done, status_of};
use liko_finance_weekly::proc::{agent, body_between, log, run, run_ok};
use liko_finance_weekly::prompts;
use liko_finance_weekly::schedule::next_sunday_taipei;

const STREAM: &str = "weekly-intl-wealth-signals";
const PERSONA: &str = "liko-finance";
const SKILL: &str = "liko-finance-weekly";
const SOURCE_DOC: &str = "docs/superpowers/specs/2026-04-29-liko-finance-weekly-design/sources.md";

struct Args {
    dry_run: bool,
    check: bool,
    agent_timeout: u64,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        dry_run: false,
        check: false,
        agent_timeout: 900,
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--dry-run" => {
                a.dry_run = true;
                i += 1;
            }
            "--check" => {
                a.check = true;
                i += 1;
            }
            "--agent-timeout" => {
                a.agent_timeout = argv
                    .get(i + 1)
                    .ok_or("--agent-timeout requires a value")?
                    .parse()
                    .map_err(|_| "--agent-timeout must be a number".to_string())?;
                i += 2;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

fn load_context(issue_id: &str, target_date: &str) -> Result<String, String> {
    let persona = run_ok(&["persona-core", "personas", "get", PERSONA])?;
    let stream = run_ok(&["persona-core", "streams", "get", STREAM])?;

    // History is best-effort: a week should not be lost because the archive is
    // briefly unreadable. The agent is told so rather than shown a silent gap.
    let history = run_ok(&[
        "persona-core", "history", "list", "--skill", SKILL, "--stream", STREAM, "--limit", "8",
    ])
    .unwrap_or_else(|e| format!("(history unavailable: {e})"));

    let style = run_ok(&["persona-core", "style", "show", "default", "--as-prompt-block"])?;

    let source_path = format!("{}/{SOURCE_DOC}", liko_finance_weekly::proc::repo());
    let source_policy = std::fs::read_to_string(&source_path)
        .map_err(|e| format!("cannot read {source_path}: {e}"))?;

    Ok([
        format!("ISSUE_ID:\n{issue_id}"),
        format!("TARGET_DATE:\n{target_date}"),
        format!("PERSONA:\n{persona}"),
        format!("STREAM:\n{stream}"),
        format!("RECENT_HISTORY:\n{history}"),
        format!("WRITING_STYLE:\n{style}"),
        format!("SOURCE_POLICY_DOC:\n{source_policy}"),
    ]
    .join("\n\n"))
}

/// Write the body somewhere persona-core can read back with `@path`.
fn write_temp(prefix: &str, body: &str) -> Result<String, String> {
    let path = format!(
        "{}/{prefix}{}.md",
        std::env::temp_dir().display(),
        std::process::id()
    );
    std::fs::write(&path, body).map_err(|e| format!("cannot write {path}: {e}"))?;
    Ok(path)
}

/// `validate-body` signals pass/fail by exit code and prints the violations.
fn validate(path: &str) -> (bool, String) {
    let at = format!("@{path}");
    let o = run(&["persona-core", "streams", "issues", "validate-body", &at]);
    let report = if o.stdout.trim().is_empty() {
        o.stderr.trim().to_string()
    } else {
        o.stdout.trim().to_string()
    };
    (o.ok(), report)
}

fn run_skill(args: &Args) -> Result<(), String> {
    run_ok(&["persona-core", "doctor"])?;
    run_ok(&["persona-core", "streams", "issues", "--help"])?;
    if args.check {
        return Ok(());
    }

    let target_date = next_sunday_taipei(jiff::Timestamp::now());
    // `prepare --print-id` emits the bare id. target_date is the value just
    // passed in, so it is reused rather than parsed back out.
    let issue_id = run_ok(&[
        "persona-core", "streams", "issues", "prepare", STREAM,
        "--target-date", &target_date, "--print-id",
    ])?;

    let listing = run_ok(&["persona-core", "streams", "issues", "list", STREAM])?;
    let status = status_of(&listing, &issue_id);
    log(&format!(
        "issue_id={issue_id} target_date={target_date} status={status:?}"
    ));

    if already_done(status.as_deref()) {
        log("issue already published or delivered; no-op");
        return Ok(());
    }

    log("stage: load_context start");
    let context = load_context(&issue_id, &target_date)?;

    log("stage: draft start (agent call)");
    let timeout = Duration::from_secs(args.agent_timeout);
    let reply = agent(&prompts::DRAFT.replace("{context}", &context), timeout)?;
    let mut body = body_between(&reply, "BEGIN_ISSUE_BODY", "END_ISSUE_BODY");
    log(&format!("stage: draft done ({} chars)", body.len()));

    let mut path = write_temp(&format!("liko-{target_date}-"), &body)?;
    log("stage: validate start");
    let (mut passed, mut report) = validate(&path);
    log(&format!("stage: validate done (passed={passed})"));

    if !passed {
        log("stage: repair start (initial validation failed, one repair pass)");
        let prompt = prompts::REPAIR
            .replace("{body}", &body)
            .replace("{validation_report}", &report);
        let reply = agent(&prompt, timeout)?;
        body = body_between(&reply, "BEGIN_ISSUE_BODY", "END_ISSUE_BODY");
        log(&format!("stage: repair done ({} chars)", body.len()));
        path = write_temp(&format!("liko-{target_date}-repair-"), &body)?;
        log("stage: re-validate start");
        let second = validate(&path);
        passed = second.0;
        report = second.1;
        log(&format!("stage: re-validate done (passed={passed})"));
    }

    if !passed {
        log("validation failed after repair pass");
        log(&report);
        if !args.dry_run {
            run_ok(&[
                "persona-core", "streams", "issues", "update-body", &issue_id,
                "--no-validation-ok",
                "--validation-summary",
                "R1/R2/R3 validation failed after one repair pass; issue skipped",
                "--status", "skipped",
            ])?;
        }
        return Err("validation failed after repair pass".into());
    }

    if args.dry_run {
        println!("dry_run: yes");
        println!("issue_id: {issue_id}");
        println!("target_date: {target_date}");
        println!("body_path: {path}");
        println!("validation: passed");
        println!("would_update_body: yes");
        println!("would_publish: yes");
        return Ok(());
    }

    log("stage: update-body start");
    let body_arg = format!("@{path}");
    run_ok(&[
        "persona-core", "streams", "issues", "update-body", &issue_id,
        "--body", &body_arg,
        "--validation-ok",
        "--validation-summary", "R1/R2/R3 validation passed",
        "--status", "validated",
    ])?;

    log("stage: publish start");
    let out = run_ok(&[
        "persona-core", "streams", "issues", "publish", &issue_id,
        "--kind", "issue", "--target", "both",
    ])?;
    log("stage: publish done");
    println!("{out}");
    Ok(())
}

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    claw_core::env::load_env(None);

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            let _ = writeln!(err, "[ERROR: {e}]");
            std::process::exit(2);
        }
    };

    match run_skill(&args) {
        Ok(()) => std::process::exit(finish(
            Finish::Marked {
                status: SkillStatus::Ok,
                exit: 0,
            },
            &mut out,
        )),
        Err(e) => {
            log(&format!("ERROR {e}"));
            // Marked failed with exit 0, not a non-zero exit: nullclaw's
            // exit_code != 0 branch overrides marker parsing, and the marker is
            // the more precise signal. This skill publishes nothing on failure,
            // so there is no delivery to suppress.
            std::process::exit(finish(
                Finish::Marked {
                    status: SkillStatus::Failed,
                    exit: 0,
                },
                &mut out,
            ))
        }
    }
}
