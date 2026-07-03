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


if __name__ == "__main__":
    unittest.main()
