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
