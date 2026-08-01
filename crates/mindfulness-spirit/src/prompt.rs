//! Filling the two prompt templates.
//!
//! Both live in `prompts/` and are edited by hand far more often than this
//! code is, so the substitution stays deliberately dumb: a fixed set of named
//! slots, replaced literally, with an error if the template asks for one that
//! does not exist.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The marker the checklist template uses for the draft.
///
/// A different shape from the writer template's `{slot}` on purpose: this one
/// is substituted with model output, which routinely contains braces, and a
/// `str.format`-style pass over it would try to interpret them.
pub const WRITER_OUTPUT: &str = "{{WRITER_OUTPUT}}";

pub fn prompts_dir(skill_dir: &Path) -> PathBuf {
    skill_dir.join("prompts")
}

/// Replace every `{name}` for which a value was supplied.
///
/// An unknown `{name}` is an error rather than a passthrough: the template is
/// how the article gets its voice, its history and its editorial plan, and a
/// silently-empty slot produces an article that reads fine and is missing the
/// thing that makes it part of a series.
pub fn render(template: &str, values: &BTreeMap<&str, String>) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            return Err("unbalanced '{' in template".into());
        };
        let name = &after[..close];
        match values.get(name) {
            Some(v) => out.push_str(v),
            None => return Err(format!("template asks for unknown slot {{{name}}}")),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The checklist prompt, with the draft spliced in.
pub fn checklist(template: &str, writer_output: &str) -> String {
    template.replace(WRITER_OUTPUT, writer_output)
}
