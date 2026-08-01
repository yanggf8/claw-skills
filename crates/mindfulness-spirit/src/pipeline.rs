//! Draft, review, store, publish.

use crate::agent::{self, Output};
use claw_core::sanitize::strip_agent_artifacts;

/// The outcome of the two agent passes.
pub enum Draft {
    /// A body worth storing, plus the line recorded as `validation_summary`.
    Reviewed {
        body: String,
        validation_summary: String,
    },
    /// Neither pass produced something publishable.
    Failed { code: i32, reason: String },
}

/// Write, then review.
///
/// A failed checklist aborts. It is tempting to publish the unreviewed draft
/// and mark it degraded — an earlier version of this skill did, and its own
/// tests were written to forbid it. The checklist is the only thing standing
/// between a first draft and the reader; skipping it and labelling the result
/// makes the label the only difference between reviewed and not.
///
/// Both passes are sanitised. The writer's output is cleaned *before* it goes
/// into the checklist prompt, so a stray `<ncchoices>` block cannot become
/// something the reviewer is asked to reason about; and the checklist's own
/// output is cleaned again before it is stored. Neither pass collapses blank
/// lines: this is an article body, where they are paragraph breaks.
pub fn write_and_review(
    writer_prompt: &str,
    checklist_template: &str,
    run: &dyn Fn(&str) -> Output,
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
) -> Draft {
    let writer = run(writer_prompt);
    if writer.code != 0 {
        let _ = write!(out, "{}", writer.stdout);
        let _ = write!(err, "{}", writer.stderr);
        let reason = writer.failure_reason();
        let _ = writeln!(err, "ERROR: writer agent failed: {reason}");
        return Draft::Failed { code: 1, reason };
    }

    let draft = strip_agent_artifacts(&writer.stdout, false);
    let checklist = run(&crate::prompt::checklist(checklist_template, &draft));
    if checklist.code != 0 {
        let reason = checklist.failure_reason();
        let _ = writeln!(err, "[checklist] degraded: {reason}");
        return Draft::Failed {
            code: checklist.code,
            reason: format!("checklist phase degraded: {reason}"),
        };
    }

    Draft::Reviewed {
        body: strip_agent_artifacts(&checklist.stdout, false),
        validation_summary: "checklist passed".to_string(),
    }
}

pub fn run_agent(prompt: &str) -> Output {
    agent::run(prompt, agent::TIMEOUT)
}
