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
