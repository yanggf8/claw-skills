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

/// Shortest and longest headline accepted, in characters (not bytes: CJK).
pub const TITLE_MIN_CHARS: usize = 6;
pub const TITLE_MAX_CHARS: usize = 40;

/// Trading verbs a headline must not carry — the same list the action section
/// bans, as whole words. Single-character 買 / 賣 are left out on purpose:
/// they appear inside ordinary nouns (賣壓, 買氣) that describe a market
/// rather than tell a reader to trade.
const TITLE_BANNED: [&str; 9] = [
    "買進", "賣出", "申購", "贖回", "轉倉", "加碼", "減碼", "進場", "出場",
];

/// The issue's own headline, from between `BEGIN_ISSUE_TITLE` /
/// `END_ISSUE_TITLE` in the draft reply.
///
/// Unlike [`body_between`], a missing or malformed title is an `Err`, not a
/// best-effort guess: the title is optional to the publish (persona-core falls
/// back to "<stream title> <target date>"), so the caller logs the reason and
/// ships without one rather than publishing narration as a headline. The
/// returned value is one trimmed line with surrounding quotes and Markdown
/// heading or emphasis marks removed.
pub fn issue_title(reply: &str) -> Result<String, String> {
    const BEGIN: &str = "BEGIN_ISSUE_TITLE";
    const END: &str = "END_ISSUE_TITLE";
    let start = reply.find(BEGIN).ok_or("no BEGIN_ISSUE_TITLE marker")? + BEGIN.len();
    let len = reply[start..].find(END).ok_or("no END_ISSUE_TITLE marker")?;
    let lines: Vec<&str> = reply[start..start + len]
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let [line] = lines.as_slice() else {
        return Err(format!("title must be one line, got {}", lines.len()));
    };
    let title = unwrap_title(line.trim_start_matches('#').trim()).to_string();
    let n = title.chars().count();
    if !(TITLE_MIN_CHARS..=TITLE_MAX_CHARS).contains(&n) {
        return Err(format!(
            "title is {n} chars, want {TITLE_MIN_CHARS}-{TITLE_MAX_CHARS}"
        ));
    }
    if title.chars().any(char::is_control) {
        return Err("title contains a control character".into());
    }
    if let Some(w) = TITLE_BANNED.iter().find(|w| title.contains(*w)) {
        return Err(format!("title uses trading verb {w}"));
    }
    Ok(title)
}

/// Drop one wrapper the model put around the whole headline — `**…**`,
/// `"…"`, `「…」` — but only when it wraps all of it: 「信託」成形 keeps its
/// brackets, because stripping each end on its own would leave them unpaired.
fn unwrap_title(s: &str) -> &str {
    for (open, close) in [("**", "**"), ("\"", "\""), ("「", "」"), ("『", "』")] {
        if let Some(inner) = s.strip_prefix(open).and_then(|r| r.strip_suffix(close)) {
            if !inner.contains(open) && !inner.contains(close) {
                return inner.trim();
            }
        }
    }
    s
}
