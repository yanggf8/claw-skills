import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import skill_runner as sr


class RunCmdTolerantTests(unittest.TestCase):
    """run_cmd_tolerant returns the raw (rc, stdout, stderr) triple and
    never raises — for commands whose non-zero exit is a signal, not an
    error (e.g. persona-core validate-body)."""

    def setUp(self):
        # run_cmd_tolerant logs via _display_args, which needs an init'd skill.
        sr.init("test-skill-runner")

    def test_zero_exit_returns_stdout(self):
        rc, out, err = sr.run_cmd_tolerant(
            [sys.executable, "-c", "print('hello')"]
        )
        self.assertEqual(rc, 0)
        self.assertEqual(out.strip(), "hello")
        self.assertEqual(err, "")

    def test_nonzero_exit_does_not_raise(self):
        rc, out, err = sr.run_cmd_tolerant(
            [sys.executable, "-c", "import sys; sys.exit(2)"]
        )
        self.assertEqual(rc, 2)

    def test_nonzero_exit_still_returns_stdout_and_stderr(self):
        # validate-body's failure mode: non-zero exit WITH the report on
        # stdout. The caller must still get that text back.
        rc, out, err = sr.run_cmd_tolerant(
            [
                sys.executable,
                "-c",
                "import sys; print('violation: bad'); "
                "print('detail', file=sys.stderr); sys.exit(2)",
            ]
        )
        self.assertEqual(rc, 2)
        self.assertEqual(out.strip(), "violation: bad")
        self.assertEqual(err.strip(), "detail")


class ExtractBodyTests(unittest.TestCase):
    """_extract_body pulls the real body out of an LLM's marker-wrapped
    stdout. The markers let the model speak freely (preamble/epilogue)
    around the payload; extraction must be robust to the model being
    sloppy with them (a common MiniMax-M3 failure), because the whole
    point of the markers is to survive exactly that."""

    MARKER = ("BEGIN_REWRITE", "END_REWRITE")

    def test_both_markers_present_extracts_inner(self):
        out = "chatter\nBEGIN_REWRITE\n---\ntitle: x\n---\nbody\nEND_REWRITE\ntrailing"
        self.assertEqual(
            sr._extract_body(out, self.MARKER),
            "---\ntitle: x\n---\nbody",
        )

    def test_no_marker_returns_stripped_output(self):
        # No body_marker configured at all -> whole output (trimmed).
        self.assertEqual(sr._extract_body("  hello  ", None), "hello")

    def test_missing_end_marker_extracts_begin_to_eof(self):
        # THE BUG: the rewrite LLM emitted a preamble, then BEGIN_REWRITE
        # (glued to the preamble), then the real article with frontmatter,
        # but NEVER emitted END_REWRITE. The old code fell back to the whole
        # output (preamble included) whose first line was not `---`, so the
        # verify gate rejected it as "no frontmatter". Extraction must take
        # everything AFTER the start marker to end-of-output.
        out = (
            "I'll perform the rewrite based on the critique.\n"
            "1. fix hook 2. fix table ...引用BEGIN_REWRITE\n"
            "---\ntitle: \"AI 週報\"\npublished: true\n---\n"
            "> body\n```ainews-meta\nkey_links:\n  - https://x\n```"
        )
        body = sr._extract_body(out, self.MARKER)
        self.assertTrue(
            body.startswith("---"),
            f"body must start with frontmatter, got: {body[:40]!r}",
        )
        self.assertNotIn("I'll perform the rewrite", body)
        self.assertIn("title:", body)

    def test_preamble_before_begin_is_stripped_even_with_both_markers(self):
        out = "let me help.\nBEGIN_REWRITE\nreal\nEND_REWRITE"
        self.assertEqual(sr._extract_body(out, self.MARKER), "real")

    def test_start_marker_absent_falls_back_to_whole_output(self):
        # If the model never emitted the start marker at all, we cannot
        # locate a payload — fall back to the trimmed whole output (the
        # downstream verify gate is the real backstop).
        self.assertEqual(
            sr._extract_body("  plain body no markers  ", self.MARKER),
            "plain body no markers",
        )


class AgentArgvTests(unittest.TestCase):
    """_agent_argv builds the `nullclaw agent` command line. Optional
    provider/model let a caller pin a model per-call (nullclaw natively
    supports --provider/--model); omitting them must reproduce the exact
    prior argv so existing callers are unaffected."""

    def test_no_provider_or_model_is_bare_argv(self):
        self.assertEqual(
            sr._agent_argv("hello", provider=None, model=None),
            ["nullclaw", "agent", "-m", "hello"],
        )

    def test_model_only_appends_model_flag(self):
        self.assertEqual(
            sr._agent_argv("hi", provider=None, model="GLM-5.2"),
            ["nullclaw", "agent", "-m", "hi", "--model", "GLM-5.2"],
        )

    def test_provider_only_appends_provider_flag(self):
        self.assertEqual(
            sr._agent_argv("hi", provider="anthropic-custom:https://x/anthropic", model=None),
            ["nullclaw", "agent", "-m", "hi",
             "--provider", "anthropic-custom:https://x/anthropic"],
        )

    def test_provider_and_model_append_both(self):
        self.assertEqual(
            sr._agent_argv("hi", provider="prov", model="mod"),
            ["nullclaw", "agent", "-m", "hi", "--provider", "prov", "--model", "mod"],
        )


class StripAgentArtifactsTests(unittest.TestCase):
    """strip_agent_artifacts cleans agent stdout before Telegram delivery.

    MiniMax-M3 (and similar harness models) routinely append interactive
    <ncchoices> JSON blocks, drop the closing tag, and leak cron/skill
    harness markers into the user-facing body. The traffic/weather skills
    deliver agent advice as-is; without stripping, users see raw JSON and
    [skill-status:ok] lines. This function is the last gate: remove only
    known artifact patterns, keep legitimate angle-bracket advice
    (e.g. <25分鐘), collapse blank runs, and stay idempotent."""

    def test_incident_regression_ncchoices_block_removed(self):
        # Real agent stdout that leaked into Telegram: advice + paired
        # <ncchoices> JSON. Users must see advice only.
        raw = (
            "路況順暢，維持平常路線即可，行車平安。\n\n"
            '<ncchoices>{"v":1,"options":[{"id":"ok","label":"收到，繼續出發",'
            '"submittext":"收到"}]}</ncchoices>'
        )
        self.assertEqual(
            sr.strip_agent_artifacts(raw),
            "路況順暢，維持平常路線即可，行車平安。",
        )

    def test_unclosed_ncchoices_strips_to_eof(self):
        # MiniMax-M3 routinely drops the end marker — strip from open tag
        # through end-of-string so trailing garbage never reaches the user.
        raw = (
            "路況順暢，維持平常路線即可。\n"
            '<ncchoices>{"v":1,"options":[{"id":"ok","label":"收到"}]}'
        )
        self.assertEqual(
            sr.strip_agent_artifacts(raw),
            "路況順暢，維持平常路線即可。",
        )

    def test_case_insensitive_ncchoices_tag(self):
        raw = (
            "advice here\n"
            '<NCChoices>{"v":1}</NCChoices>\n'
            "more advice"
        )
        self.assertEqual(
            sr.strip_agent_artifacts(raw),
            "advice here\nmore advice",
        )

    def test_multiline_json_inside_ncchoices_removed(self):
        raw = (
            "before\n"
            "<ncchoices>\n"
            "{\n"
            '  "v": 1,\n'
            '  "options": [{"id": "ok"}]\n'
            "}\n"
            "</ncchoices>\n"
            "after"
        )
        self.assertEqual(
            sr.strip_agent_artifacts(raw),
            "before\nafter",
        )

    def test_ncchoices_in_middle_keeps_advice_before_and_after(self):
        raw = (
            "前半段建議，路況尚可。\n"
            '<ncchoices>{"v":1,"options":[]}</ncchoices>\n'
            "後半段建議，提早出門。"
        )
        self.assertEqual(
            sr.strip_agent_artifacts(raw),
            "前半段建議，路況尚可。\n後半段建議，提早出門。",
        )

    def test_legit_angle_brackets_in_advice_passthrough(self):
        # Token-specific: only the literal ncchoices tag is stripped.
        # Comparison advice with < / > must survive byte-identical.
        text = "小於 <25分鐘 算順暢，>40分鐘 要提早出發"
        self.assertEqual(sr.strip_agent_artifacts(text), text)

    def test_skill_uuid_trace_line_removed(self):
        raw = (
            "路況順暢。\n"
            "skill-563f90ef-d26d-4afb-a4c4-a333472e97bf:2596\n"
            "行車平安。"
        )
        self.assertEqual(
            sr.strip_agent_artifacts(raw),
            "路況順暢。\n行車平安。",
        )

    def test_skill_status_and_trace_and_event_lines_removed(self):
        raw = (
            "建議維持原路線。\n"
            "[skill-status:ok]\n"
            "[skill-status:degraded]\n"
            "[trace:anything]\n"
            "[skill-event] started delivery pipeline\n"
            "注意天氣。"
        )
        self.assertEqual(
            sr.strip_agent_artifacts(raw),
            "建議維持原路線。\n注意天氣。",
        )

    def test_artifacts_only_returns_empty_string(self):
        raw = (
            '<ncchoices>{"v":1}</ncchoices>\n'
            "[skill-status:ok]\n"
            "skill-563f90ef-d26d-4afb-a4c4-a333472e97bf:2596\n"
            "[trace:x]\n"
            "[skill-event] noop"
        )
        self.assertEqual(sr.strip_agent_artifacts(raw), "")

    def test_empty_string_returns_empty(self):
        self.assertEqual(sr.strip_agent_artifacts(""), "")

    def test_idempotent_on_incident_regression(self):
        raw = (
            "路況順暢，維持平常路線即可，行車平安。\n\n"
            '<ncchoices>{"v":1,"options":[{"id":"ok","label":"收到，繼續出發",'
            '"submittext":"收到"}]}</ncchoices>'
        )
        once = sr.strip_agent_artifacts(raw)
        twice = sr.strip_agent_artifacts(once)
        self.assertEqual(once, twice)
        self.assertEqual(once, "路況順暢，維持平常路線即可，行車平安。")

    def test_clean_chinese_advice_unchanged(self):
        text = "路況順暢，維持平常路線即可，行車平安。"
        self.assertEqual(sr.strip_agent_artifacts(text), text)

    def test_leading_artifacts_before_advice_removed(self):
        # Artifact removal is position-independent — leading harness lines
        # before real advice must not survive either.
        raw = (
            "[skill-status:ok]\n"
            "skill-563f90ef-d26d-4afb-a4c4-a333472e97bf:2596\n"
            "實際建議內容"
        )
        self.assertEqual(sr.strip_agent_artifacts(raw), "實際建議內容")

    def test_whitespace_only_returns_empty(self):
        self.assertEqual(sr.strip_agent_artifacts("  \n\n  "), "")



class StripAgentArtifactsMarkdownTests(unittest.TestCase):
    """dev.to markdown article bodies need artifact stripping WITHOUT collapsing
    paragraph blank lines — the chat-mode collapse destroys markdown layout.
    """

    def test_default_collapses_blank_lines(self):
        self.assertEqual(sr.strip_agent_artifacts("a\n\n\nb"), "a\nb")  # documents current default

    def test_markdown_mode_preserves_paragraph_blanks(self):
        out = sr.strip_agent_artifacts("# T\n\n第一段。\n\n第二段。", collapse_blank_lines=False)
        self.assertIn("\n\n第一段。", out)
        self.assertIn("\n\n第二段。", out)

    def test_markdown_mode_strips_ncchoices_paired(self):
        raw = "- 標題\n\n<ncchoices>{\"v\":1}</ncchoices>\n\n下一段"
        out = sr.strip_agent_artifacts(raw, collapse_blank_lines=False)
        self.assertEqual(out, "- 標題\n\n\n\n下一段")  # ncchoices gone, BOTH paragraph blank lines preserved

    def test_markdown_mode_strips_unclosed_ncchoices_to_eof(self):
        self.assertEqual(
            sr.strip_agent_artifacts("正文\n\n<ncchoices>{partial", collapse_blank_lines=False),
            "正文",
        )

    def test_markdown_mode_strips_harness_marker_lines(self):
        raw = "內容。\n[skill-status:ok]\nskill-563f90ef-d26d-4afb-a4c4-a333472e97bf:123\n結語。"
        out = sr.strip_agent_artifacts(raw, collapse_blank_lines=False)
        self.assertNotIn("skill-status", out)
        self.assertNotIn("skill-563f90ef", out)
        self.assertIn("內容。", out)
        self.assertIn("結語。", out)
        self.assertIn("\n\n", out)  # blank lines NOT collapsed

    def test_markdown_mode_preserves_source_markers(self):
        text = "[來源 #1] 引用文字"
        self.assertEqual(sr.strip_agent_artifacts(text, collapse_blank_lines=False), text)  # strip rules must NOT touch [來源 #N]

    def test_default_mode_unchanged_for_existing_callers(self):
        raw = "建議維持原路線。\n\n<ncchoices>{\"v\":1}</ncchoices>\n[skill-status:ok]"
        expected = "建議維持原路線。"
        self.assertEqual(sr.strip_agent_artifacts(raw), expected)
        self.assertEqual(sr.strip_agent_artifacts(raw, collapse_blank_lines=True), expected)  # explicit True == default


if __name__ == "__main__":
    unittest.main()
