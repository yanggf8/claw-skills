//! Calling the `persona-core` CLI.
//!
//! Everything durable belongs to persona-core: the persona's voice, the publish
//! history, the editorial plan, the installment row, the body, the signature,
//! the delivery and the failure dump. This skill only decides *when* and *with
//! what*, so almost every step here is one subprocess call whose non-zero exit
//! must reach the caller intact.

use std::io::Write;
use std::process::Command;

/// persona-core's documented exit codes. Only the ones this skill can
/// distinguish usefully are named.
pub mod exit {
    /// The thing asked for does not exist — for `installments prepare`, that
    /// means no planned installment is left, which is an exhausted season
    /// rather than a broken skill.
    pub const NOT_FOUND: i32 = 4;
    /// Transient dependency failure. Safe to retry only for reads; a write
    /// that returns this may have partly applied.
    pub const TRANSIENT: i32 = 75;
}

#[derive(Debug)]
pub struct CallFailed {
    pub code: i32,
    pub args: Vec<String>,
}

impl std::fmt::Display for CallFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "persona-core {} failed with exit code {}",
            self.args.join(" "),
            self.code
        )
    }
}

/// Run one persona-core command and return its stdout.
///
/// stderr is always forwarded — persona-core puts its diagnostics there and
/// they are the only explanation an operator gets from a cron log. `echo`
/// additionally forwards stdout, for the steps whose output is a report rather
/// than a value this code consumes.
pub fn call(
    args: &[&str],
    echo: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<String, CallFailed> {
    let output = Command::new("persona-core").args(args).output();
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            let _ = writeln!(err, "persona-core could not be run: {e}");
            // 127 is the shell's "command not found"; keeping it distinct from
            // persona-core's own codes stops a missing binary being read as a
            // not-found row.
            return Err(CallFailed {
                code: 127,
                args: owned,
            });
        }
    };

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        let _ = write!(err, "{stderr}");
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if echo && !stdout.is_empty() {
        let _ = write!(out, "{stdout}");
    }

    match output.status.code() {
        Some(0) => Ok(stdout),
        code => Err(CallFailed {
            code: code.unwrap_or(1),
            args: owned,
        }),
    }
}

/// The six prompt blocks the writer template needs, in the order it uses them.
pub struct Blocks {
    pub persona_voice: String,
    pub style: String,
    pub signature: String,
    pub history: String,
    pub topic: String,
}

pub fn blocks(
    persona: &str,
    skill: &str,
    column: &str,
    history_limit: u32,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<Blocks, CallFailed> {
    let limit = history_limit.to_string();
    Ok(Blocks {
        persona_voice: call(&["personas", "show", persona, "--as-prompt-block"], false, out, err)?,
        style: call(&["style", "show", "default", "--as-prompt-block"], false, out, err)?,
        signature: call(&["personas", "show", persona, "--as-signature"], false, out, err)?,
        history: call(
            &["history", "list", "--persona", persona, "--as-prompt-block", "--limit", &limit],
            false,
            out,
            err,
        )?,
        topic: call(
            &["plans", "next", skill, column, "--as-prompt-block"],
            false,
            out,
            err,
        )?,
    })
}
