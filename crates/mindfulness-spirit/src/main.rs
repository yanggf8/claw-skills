//! mindfulness-spirit — the weekly 身心靈 × AI column.
//!
//! Ports `mindfulness-spirit/scripts/run.py`. The skill itself is thin: it
//! gathers the week's material, renders two prompts from blocks persona-core
//! owns, runs the writer and the checklist, and hands the result back to
//! persona-core to store and publish.
//!
//! One thing the Python did not do: emit the scheduler markers. Its cron is on
//! `verification_mode = skill_contract`, which reads two literal stdout lines,
//! so even a run that published perfectly was recorded as `content_invalid`.
//! Everything this binary prints before the markers is diagnostic — the
//! article goes out through persona-core, never through stdout — so the
//! markers are safe to emit here.

use std::io::Write;
use std::path::PathBuf;

use claw_core::env::load_env;
use claw_core::marker::SkillStatus;
use claw_core::outcome::{finish, Finish};

use mindfulness_spirit::cli::{self, Command, USAGE};
use mindfulness_spirit::config::{self, Settings, HISTORY_LIMIT, SKILL_NAME};
use mindfulness_spirit::material;
use mindfulness_spirit::personacore::{self, exit as pc_exit, CallFailed};
use mindfulness_spirit::pipeline::{self, Draft};
use mindfulness_spirit::prompt;

/// Where `prompts/` lives, relative to the published binary at
/// `<skill>/bin/<name>`.
fn skill_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().and_then(|b| b.parent()).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse_args(&argv) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(err, "[ERROR: {e}]\n{USAGE}");
            std::process::exit(2);
        }
    };

    load_env(None);
    std::process::exit(run(command, &mut out, &mut err));
}

fn run(command: Command, out: &mut impl Write, err: &mut impl Write) -> i32 {
    let settings = match config::load_settings(err) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(err, "ERROR: {e}");
            return finish(Finish::Unmarked { exit: 1 }, out);
        }
    };

    let code = match command {
        Command::FixSignature { devto_id, dry_run } => {
            fix_signature(&settings, devto_id, dry_run, out, err)
        }
        Command::Write { dry_run } => write(&settings, dry_run, out, err),
    };

    // A non-zero exit wins over every marker in nullclaw's classification, so
    // a failed run says nothing and lets the exit code speak.
    if code != 0 {
        return finish(Finish::Unmarked { exit: code }, out);
    }
    finish(
        Finish::Marked {
            status: SkillStatus::Ok,
            exit: 0,
        },
        out,
    )
}

fn report(e: &CallFailed, err: &mut impl Write) -> i32 {
    let _ = writeln!(err, "ERROR: {e}");
    if e.code == pc_exit::NOT_FOUND {
        // The season ran out. Six weeks of Friday failures once looked like a
        // broken skill because nothing said this.
        let _ = writeln!(
            err,
            "HINT: no planned installment left — the column's season is finished. \
             Add the next one with `persona-core columns installments add`, or point \
             skills.mindfulness_spirit.column_slug at a new column."
        );
    }
    if e.code == pc_exit::TRANSIENT {
        let _ = writeln!(
            err,
            "HINT: transient dependency failure. A write that exits 75 may have \
             partly applied — check the installment before re-running."
        );
    }
    e.code
}

fn fix_signature(
    settings: &Settings,
    devto_id: i64,
    dry_run: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> i32 {
    let id = devto_id.to_string();
    let mut args = vec![
        "columns",
        "installments",
        "repair-signature",
        &id,
        "--persona",
        &settings.persona_slug,
    ];
    if dry_run {
        args.push("--dry-run");
    }
    match personacore::call(&args, true, out, err) {
        Ok(_) => 0,
        Err(e) => report(&e, err),
    }
}

fn write(settings: &Settings, dry_run: bool, out: &mut impl Write, err: &mut impl Write) -> i32 {
    let items = material::collect(None, err);
    if items.is_empty() {
        let _ = writeln!(err, "ERROR: No RSS results found.");
        return 1;
    }

    let dir = prompt::prompts_dir(&skill_dir());
    let writer_tmpl = match std::fs::read_to_string(dir.join("writer.md.tmpl")) {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(err, "ERROR: cannot read writer template: {e}");
            return 1;
        }
    };
    let checklist_tmpl = match std::fs::read_to_string(dir.join("checklist.md.tmpl")) {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(err, "ERROR: cannot read checklist template: {e}");
            return 1;
        }
    };

    let blocks = match personacore::blocks(
        &settings.persona_slug,
        SKILL_NAME,
        &settings.column_slug,
        HISTORY_LIMIT,
        out,
        err,
    ) {
        Ok(b) => b,
        Err(e) => return report(&e, err),
    };

    let mut values = std::collections::BTreeMap::new();
    values.insert("persona_voice_block", blocks.persona_voice);
    values.insert("style_block", blocks.style);
    values.insert("signature_block", blocks.signature);
    values.insert("history_block", blocks.history);
    values.insert("topic_block", blocks.topic);
    values.insert("prompt_items", material::prompt_items(&items));
    let writer_prompt = match prompt::render(&writer_tmpl, &values) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(err, "ERROR: {e}");
            return 1;
        }
    };

    let work = match scratch_dir() {
        Ok(d) => d,
        Err(e) => {
            let _ = writeln!(err, "ERROR: cannot create work directory: {e}");
            return 1;
        }
    };
    let writer_path = work.join("writer.md");
    let checklist_path = work.join("checklist.md");
    let material_path = work.join("material.tsv");
    for (path, body) in [
        (&writer_path, &writer_prompt),
        (&checklist_path, &checklist_tmpl),
        (&material_path, &material::material_text(&items)),
    ] {
        if let Err(e) = std::fs::write(path, body) {
            let _ = writeln!(err, "ERROR: cannot write {}: {e}", path.display());
            return 1;
        }
    }

    let _ = writeln!(out, "Writer prompt: {}", writer_path.display());
    let _ = writeln!(out, "Checklist prompt: {}", checklist_path.display());
    let _ = writeln!(out, "Material file: {}", material_path.display());
    let _ = writeln!(out, "RSS items: {}", items.len());
    let _ = writeln!(out, "Persona: {}", settings.persona_slug);
    let _ = writeln!(out, "Column: {}", settings.column_slug);
    let _ = writeln!(
        out,
        "Legacy publish config: {}; main_image_url: {}",
        settings.publish,
        settings.main_image_url.as_deref().unwrap_or("(none)")
    );
    if dry_run {
        let _ = writeln!(out, "[Dry-run] Skip agent, prepare, update-body, and publish.");
        return 0;
    }

    let draft = pipeline::write_and_review(
        &writer_prompt,
        &checklist_tmpl,
        &pipeline::run_agent,
        out,
        err,
    );
    let (body, validation_summary) = match draft {
        Draft::Reviewed {
            body,
            validation_summary,
        } => (body, validation_summary),
        // Nothing is prepared, stored or published — the installment stays
        // planned, so next week's run picks up the same one.
        Draft::Failed { code, .. } => return code,
    };

    let body_path = work.join("body.md");
    if let Err(e) = std::fs::write(&body_path, &body) {
        let _ = writeln!(err, "ERROR: cannot write body: {e}");
        return 1;
    }

    let id = match personacore::call(
        &[
            "columns",
            "installments",
            "prepare",
            &settings.column_slug,
            "--print-id",
        ],
        false,
        out,
        err,
    ) {
        Ok(s) => s.trim().to_string(),
        Err(e) => return report(&e, err),
    };
    let id = match id.parse::<i64>() {
        Ok(n) => n.to_string(),
        Err(_) => {
            let _ = writeln!(err, "ERROR: prepare returned a non-numeric id: {id:?}");
            return 1;
        }
    };
    let _ = writeln!(out, "Prepared installment: {id}");

    let body_arg = format!("@{}", body_path.display());
    let material_arg = format!("@{}", material_path.display());
    if let Err(e) = personacore::call(
        &[
            "columns",
            "installments",
            "update-body",
            &id,
            "--body",
            &body_arg,
            "--material",
            &material_arg,
            "--restore-source-links",
            "--derive-key-links",
            "--validation-ok",
            "--validation-summary",
            &validation_summary,
        ],
        true,
        out,
        err,
    ) {
        return report(&e, err);
    }

    if let Err(e) = personacore::call(
        &[
            "columns",
            "installments",
            "publish",
            &id,
            "--skill-id",
            SKILL_NAME,
        ],
        true,
        out,
        err,
    ) {
        return report(&e, err);
    }
    0
}

/// A per-run directory under the system temp dir, matching `mkdtemp`'s prefix
/// so an operator can still find last week's prompts by name.
fn scratch_dir() -> std::io::Result<PathBuf> {
    let base = std::env::temp_dir();
    let stamp = jiff_nanos();
    for n in 0..64 {
        let dir = base.join(format!("mindfulness-spirit-{}{n}", stamp));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("could not create a unique work directory"))
}

fn jiff_nanos() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{n:x}-", std::process::id())
}
