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


if __name__ == "__main__":
    unittest.main()
