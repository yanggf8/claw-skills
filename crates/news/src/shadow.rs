//! `news shadow-ai`: the AI section built from the direct feeds, written to
//! `~/.nullclaw/.news-shadow/<YYYY-MM-DD>.md`, never delivered.
//!
//! Spec `docs/specs/2026-11-17-news-ai-direct-feeds-design.md` §6. The
//! shadow is its own process under its own cron job, so its timeout, crash
//! or exit code is classified against that job only [R3]. Nothing here
//! reaches Telegram: this module does not use `claw_core::delivery`, and the
//! one other path that could — `alert_failure` — is handed a silenced
//! context [R2]. The trace stamp [R9] is set by `main` before this runs.
//!
//! Stdout is the cron capture and carries only the markers.
//!
//! - `ok` — a section with at least one story was written.
//! - `degraded` — the file was written, but the section is empty (no usable
//!   config, every feed empty, nothing picked) or could not be produced at
//!   all (Level-3 exhaustion). The shadow job is on `repair_policy = none`,
//!   and the shadow delivers nothing, so `degraded` re-delivers nothing.
//! - exit 1, unmarked — the file could not be written.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};
use serde_json::json;

use crate::alert::AlertContext;
use crate::cache::safe_date;
use crate::config::{section, shadow_dir};
use crate::digest::summarize_ai_section;
use crate::precheck::{tier1_filter_items, SharedCache};
use crate::run::{direct_ai_items, FeedSource};
use crate::sources::AiSources;
use crate::text::dedup;
use crate::trace::{job_id, log_trace};

/// The file's only line under the header when the section could not be
/// produced. Not a `- ` bullet, so it is never counted as a story; the
/// reason is the alert `summarize_default_ai_substaged_for` raised.
const EXHAUSTED_LINE: &str = "（AI 段未產生：ai_substage_level3_failed）";

/// The day's file. `date_str` is the run's `"YYYY/MM/DD (Mon)"` Taipei date,
/// turned into a name exactly as the day cache does, so the shadow file and
/// the cache directory it filled always name the same day.
pub fn shadow_path(date_str: &str) -> PathBuf {
    shadow_dir().join(format!("{}.md", safe_date(date_str)))
}

/// Temp file in the same directory, then rename: a reader never sees half a
/// file, and a rerun replaces the day's file whole.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(std::io::Error::other("no file name"));
    };
    if name.to_string_lossy() == ".md" {
        return Err(std::io::Error::other("no date to name the file with"));
    }
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp.{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// What `main` runs: no chat id, alerts silenced.
pub fn run_shadow_ai(
    feeds: &dyn FeedSource,
    date_str: &str,
    cache_handle: &SharedCache,
    out: &mut impl Write,
) -> i32 {
    let ctx = AlertContext::new(None, String::new(), std::env::var("NULLCLAW_JOB_ID").ok());
    run_shadow_ai_with(ctx, feeds, date_str, cache_handle, out)
}

/// [`run_shadow_ai`] with the alert context supplied. Whatever context it is
/// handed is silenced first, so a context that would page someone cannot.
pub fn run_shadow_ai_with(
    ctx: AlertContext,
    feeds: &dyn FeedSource,
    date_str: &str,
    cache_handle: &SharedCache,
    out: &mut impl Write,
) -> i32 {
    let ctx = ctx.silenced();
    let started = Instant::now();
    let path = shadow_path(date_str);

    // As the delivering run treats its AI feed: dedup, then Tier 1.
    let items = tier1_filter_items(dedup(&direct_ai_items(&ctx, feeds)));
    let built = summarize_ai_section(&items, &ctx, date_str, cache_handle, AiSources::Direct);
    let (text, outcome, stories, links_total, links_google) = match built {
        Ok(s) => {
            let outcome = if s.stories > 0 { "ok" } else { "empty" };
            (s.text, outcome, s.stories, s.links_total, s.links_google)
        }
        Err(_) => {
            let header = section("ai").expect("ai section").header;
            (format!("{header}\n{EXHAUSTED_LINE}\n"), "exhausted", 0, 0, 0)
        }
    };

    let written = write_atomically(&path, &text);
    let mut summary = json!({
        "path": path.display().to_string(),
        "outcome": outcome,
        "stories": stories,
        "links_total": links_total,
        "links_google": links_google,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    });
    match written {
        Err(e) => {
            summary["outcome"] = json!("write_failed");
            summary["error"] = json!(e.to_string());
            log_trace("ai_sources_shadow", summary);
            eprintln!(
                "[news shadow-ai] job={} could not write {}: {e}",
                job_id(),
                path.display()
            );
            finish(Finish::Unmarked { exit: 1 }, out)
        }
        Ok(()) => {
            log_trace("ai_sources_shadow", summary);
            eprintln!(
                "[news shadow-ai] wrote {} ({outcome}, {stories} stories)",
                path.display()
            );
            let status = if outcome == "ok" {
                SkillStatus::Ok
            } else {
                SkillStatus::Degraded
            };
            finish(Finish::Marked { status, exit: 0 }, out)
        }
    }
}
