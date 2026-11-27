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

/// Put a blank line between the body's lines, so each one is its own paragraph.
///
/// The prompts require the body as plain lines — section headings alone on a
/// line, action lines with no bullet — because the persona-core validator and
/// its Telegram summary both read it line by line. But Markdown joins
/// consecutive lines into one paragraph, so on dev.to every issue up to
/// 2026-11-22 rendered as a single `<p>` held together by `<br>`: headings and
/// the three action lines ran together in one block.
///
/// A blank line is the only separator that leaves both readers intact: the
/// validator and the summary skip empty lines, whereas `##` would leave a
/// stray `##` line in the summary's signal list and `**…**` fails R2 on the
/// action section. Lines are trimmed too, because a line indented four spaces
/// after a blank line becomes a code block.
pub fn paragraphs(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The AI-assistance disclosure every issue opens with. dev.to's guidelines ask
/// AI-assisted posts to say so; the account was flagged on 2026-11-27 with no
/// issue carrying one. The English half is for moderators who don't read
/// Chinese.
///
/// It goes first, never last: the validator treats every line after
/// 本週檢視動作 as an action line, so a closing line would fail R2. Nothing
/// above 本週訊號 is validated or quoted in the Telegram summary. It must not
/// contain a section marker, because the validator locates sections with
/// `find`.
pub const AI_DISCLOSURE: &str = "本文由 AI 協助撰寫。(This post was written with AI assistance.)";

/// Put [`AI_DISCLOSURE`] at the top of `body`, once. A copy the model kept
/// from the repair prompt is dropped first, so repairing twice cannot stack
/// it. Expects a body already passed through [`paragraphs`].
pub fn with_disclosure(body: &str) -> String {
    let rest: Vec<&str> = body
        .split("\n\n")
        .filter(|para| para.trim() != AI_DISCLOSURE)
        .collect();
    format!("{AI_DISCLOSURE}\n\n{}", rest.join("\n\n"))
}
