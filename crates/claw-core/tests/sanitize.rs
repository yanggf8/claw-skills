use claw_core::sanitize::strip_agent_artifacts;

fn s(t: &str) -> String { strip_agent_artifacts(t, true) }

#[test]
fn removes_paired_ncchoices_case_insensitively() {
    assert_eq!(s("before<ncchoices>a\nb</ncchoices>after"), "beforeafter");
    assert_eq!(s("before<NCChoices>x</NCCHOICES>after"), "beforeafter");
}

#[test]
fn paired_match_is_lazy_not_greedy() {
    // Greedy would swallow everything between the FIRST open and the LAST close,
    // deleting "keep". Lazy keeps it.
    assert_eq!(
        s("<ncchoices>a</ncchoices>keep<ncchoices>b</ncchoices>"),
        "keep"
    );
}

#[test]
fn removes_unclosed_ncchoices_through_end_of_input() {
    // The model routinely drops the closing tag — this is the whole reason
    // rule 2 exists.
    assert_eq!(s("advice text\n<ncchoices>\n{\"a\":1}\nmore junk"), "advice text");
}

#[test]
fn leaves_other_angle_brackets_alone() {
    // TOKEN-SPECIFIC per the Python docstring: only `ncchoices` is a tag.
    assert_eq!(s("步行 <25分鐘，開車 >40分鐘"), "步行 <25分鐘，開車 >40分鐘");
    assert_eq!(s("<b>bold</b>"), "<b>bold</b>");
}

#[test]
fn removes_marker_lines_with_colon_or_bracket() {
    // The class is [:\]] — a bare "[trace]" line matches too.
    assert_eq!(s("keep\n[skill-status:ok]\n[trace:abc:1]\nkeep2"), "keep\nkeep2");
    assert_eq!(s("keep\n[trace]\nkeep2"), "keep\nkeep2");
    assert_eq!(s("keep\n[skill-event] fell back\nkeep2"), "keep\nkeep2");
}

#[test]
fn marker_removal_is_whole_line_only() {
    // Not anchored at line start => must NOT be removed.
    assert_eq!(s("prefix [skill-status:ok]"), "prefix [skill-status:ok]");
}

#[test]
fn removes_bare_job_id_lines() {
    assert_eq!(s("keep\nskill-b8993369-96fd-4890:3801\nkeep2"), "keep\nkeep2");
    assert_eq!(s("keep\n  SKILL-B8993369-96FD:12  \nkeep2"), "keep\nkeep2");
}

#[test]
fn short_hex_is_not_a_job_id() {
    // {8,} — seven hex chars must not match.
    assert_eq!(s("keep\nskill-abc1234:1\nkeep2"), "keep\nskill-abc1234:1\nkeep2");
}

#[test]
fn collapses_blank_line_runs_when_asked() {
    assert_eq!(s("a\n\n\n\nb"), "a\nb");
}

#[test]
fn preserves_blank_lines_when_not_collapsing() {
    // markdown-safe mode for article bodies
    assert_eq!(strip_agent_artifacts("a\n\n\nb", false), "a\n\n\nb");
}

#[test]
fn trims_edges() {
    assert_eq!(s("\n\n  advice  \n\n"), "advice");
}

#[test]
fn empty_and_artifact_only_inputs_yield_empty() {
    // This is what makes the advice line disappear — the caller checks `if advice:`.
    assert_eq!(s(""), "");
    assert_eq!(s("<ncchoices>only junk</ncchoices>"), "");
    assert_eq!(s("[skill-status:ok]\n[trace:x:1]"), "");
}

#[test]
fn cjk_and_emoji_survive_intact() {
    assert_eq!(s("👔 記得帶傘，早晚偏涼"), "👔 記得帶傘，早晚偏涼");
}

#[test]
fn python_strip_removes_more_than_unicode_whitespace() {
    // Python's str.strip() removes \x1c-\x1f (file/group/record/unit separators);
    // Rust's trim() uses the Unicode White_Space property, which does NOT include
    // them. If this test fails, the port used trim() where Python used strip()
    // and edge bytes will survive that the oracle removes.
    assert_eq!(s("\u{1c}advice\u{1f}"), "advice");
}
