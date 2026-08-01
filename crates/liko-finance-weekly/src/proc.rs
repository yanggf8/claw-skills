//! Running persona-core and the drafting agent.

use std::process::Command;
use std::time::Duration;

pub fn repo() -> String {
    std::env::var("PERSONA_CORE_REPO").unwrap_or_else(|_| "/home/yanggf/a/persona-core".into())
}

pub fn log(msg: &str) {
    eprintln!("[liko-finance-weekly] {msg}");
}

#[derive(Debug)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Run a command in the persona-core repo.
///
/// No timeout argument. std::process has no portable per-call timeout, and the
/// Python's `timeout=` values (60, 120, 180) were never the thing that bounded
/// a run — the scheduler's own NULLCLAW_SKILL_TIMEOUT is. Adding a thread and a
/// kill here to reproduce a number that never fired would be machinery
/// pretending to be a guarantee. The one call that genuinely runs long, the
/// drafting agent, keeps an explicit bound via `agent_timeout`.
pub fn run(args: &[&str]) -> Output {
    match Command::new(args[0]).args(&args[1..]).current_dir(repo()).output() {
        Ok(o) => Output {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        },
        Err(e) => Output {
            code: -1,
            stdout: String::new(),
            stderr: e.to_string(),
        },
    }
}

/// Run and require success, returning trimmed stdout.
pub fn run_ok(args: &[&str]) -> Result<String, String> {
    let o = run(args);
    if o.ok() {
        Ok(o.stdout.trim().to_string())
    } else {
        Err(format!(
            "{} exited {}: {}",
            args.join(" "),
            o.code,
            o.stderr.trim().chars().take(200).collect::<String>()
        ))
    }
}

/// Ask the nullclaw agent to write, with a wall-clock bound.
///
/// The bound is real: this is the call that can hang for a quarter of an hour,
/// and the whole reason the Python passed `--agent-timeout` down to it.
pub fn agent(prompt: &str, timeout: Duration) -> Result<String, String> {
    // call_agent sanitizes: it strips <ncchoices> blocks and harness marker
    // lines. That is wanted here — this output becomes the published body, and
    // agent protocol noise must not reach a reader.
    let out = claw_core::agent::call_agent(prompt, timeout);
    if out.trim().is_empty() {
        Err("agent returned nothing".into())
    } else {
        Ok(out)
    }
}

/// Take what sits between the agent's body markers.
///
/// The drafting prompt asks for the issue wrapped in `BEGIN_ISSUE_BODY` /
/// `END_ISSUE_BODY`, because a model that has been told to research will often
/// narrate before it writes. Without the markers that narration is published.
///
/// A reply with no markers is returned whole rather than discarded: the model
/// answered, and the validator downstream is the thing that decides whether the
/// answer is usable. Silently dropping a marker-less reply would turn a
/// formatting slip into a missing week.
pub fn body_between(reply: &str, begin: &str, end: &str) -> String {
    let after = match reply.find(begin) {
        Some(i) => &reply[i + begin.len()..],
        None => return reply.trim().to_string(),
    };
    match after.find(end) {
        Some(j) => after[..j].trim().to_string(),
        // Opened but never closed — the model was cut off. Keep what came
        // after the marker; it is the start of the body, not narration.
        None => after.trim().to_string(),
    }
}
