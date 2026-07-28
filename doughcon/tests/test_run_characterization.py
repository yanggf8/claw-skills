"""Characterization tests for doughcon/scripts/run.py.

These pin CURRENT behaviour before the Rust port. They are the oracle: if one
of these changes, the port changed behaviour and that must be a recorded,
deliberate decision — not a surprise.

Run: python3 doughcon/tests/test_run_characterization.py
"""
import json
import os
import sys
import tempfile
import unittest
from contextlib import contextmanager
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "scripts"))
sys.path.insert(0, os.path.join(HERE, "..", "..", "lib"))

import run as doughcon  # noqa: E402


def fixture(name):
    with open(os.path.join(HERE, "fixtures", name)) as f:
        return json.load(f)


@contextmanager
def capture_fds():
    """Capture real fd 1 / fd 2, not sys.stdout / sys.stderr.

    lib/trace_marker.py binds ``stream=sys.stdout`` as a DEFAULT ARGUMENT,
    evaluated once at import time. contextlib.redirect_stdout rebinds
    ``sys.stdout`` but cannot touch that already-captured default, so the
    marker lines escape an in-process redirect entirely. Capturing at the file
    descriptor level records exactly the byte stream the scheduler reads —
    which is the whole point of the markers — and covers ``print(body)`` too.
    """
    sys.stdout.flush()
    sys.stderr.flush()
    out_f = tempfile.TemporaryFile()
    err_f = tempfile.TemporaryFile()
    saved_out, saved_err = os.dup(1), os.dup(2)
    try:
        os.dup2(out_f.fileno(), 1)
        os.dup2(err_f.fileno(), 2)
        yield out_f, err_f
    finally:
        sys.stdout.flush()
        sys.stderr.flush()
        os.dup2(saved_out, 1)
        os.dup2(saved_err, 2)
        os.close(saved_out)
        os.close(saved_err)
        out_f.close()
        err_f.close()


def read_fd_capture(f):
    f.seek(0)
    return f.read().decode("utf-8")


class IndexDerivationTests(unittest.TestCase):
    """The -1 sentinel is NOT 'index == 0'.

    These drive run.py's real main() and read the index off the rendered body.
    An earlier version of this class re-implemented the derivation inline and
    asserted against its own copy — it passed against a run.py whose derivation
    was deliberately broken, so it pinned nothing at all.
    """

    def _index_via_run(self, payload):
        """Return the 指數 value run.py actually printed, as a string."""
        code = 0
        with capture_fds() as (out_f, err_f):
            with mock.patch.object(doughcon, "fetch_doughcon", lambda: payload), \
                 mock.patch.object(sys, "argv", ["run.py"]):
                try:
                    doughcon.main()
                except SystemExit as e:
                    code = e.code or 0
            sys.stdout.flush()
            sys.stderr.flush()
            out = read_fd_capture(out_f)
            read_fd_capture(err_f)
        self.assertEqual(code, 0, f"run.py exited {code}; stdout was {out!r}")
        for line in out.splitlines():
            if line.startswith("指數："):
                return line[len("指數："):]
        self.fail(f"no 指數 line in run.py output: {out!r}")

    def test_normal_index_passes_through(self):
        self.assertEqual(self._index_via_run(fixture("full.json")), "42")

    def test_zero_with_all_null_is_minus_one(self):
        self.assertEqual(self._index_via_run(fixture("all_null.json")), "-1")

    def test_zero_with_real_data_stays_zero(self):
        # A genuine zero is NOT no-data. This is the subtle one.
        self.assertEqual(self._index_via_run(fixture("zero_index_with_data.json")), "0")

    def test_missing_index_is_minus_one(self):
        self.assertEqual(self._index_via_run({"data": [{"current_popularity": 5}]}), "-1")

    def test_empty_places_counts_as_all_null(self):
        self.assertEqual(self._index_via_run({"overall_index": 0, "data": []}), "-1")

    def test_mixed_null_and_present_popularity_is_not_all_null(self):
        # The only input that distinguishes all() from any(). Without it the
        # single most subtle rule in the port is unpinned on BOTH sides.
        self.assertEqual(
            self._index_via_run({"overall_index": 0, "data": [
                {"current_popularity": None}, {"current_popularity": 7}]}),
            "0")

    def test_python_zero_semantics_cover_float_and_false(self):
        # Python's `raw_index == 0` is true for 0.0, -0.0 and False.
        for value in (0.0, -0.0, False):
            self.assertEqual(
                self._index_via_run({"overall_index": value,
                                     "data": [{"current_popularity": None}]}),
                "-1", f"{value!r} must be treated as zero")
        # True == 0 is False, so it renders instead of collapsing.
        self.assertEqual(
            self._index_via_run({"overall_index": True,
                                 "data": [{"current_popularity": None}]}),
            "True")

    def test_non_numeric_popularity_is_not_null(self):
        # `is None` only — the string "x" is NOT null, so a zero index stays 0.
        self.assertEqual(
            self._index_via_run({"overall_index": 0,
                                 "data": [{"current_popularity": "x"}]}),
            "0")


class FormatUpdatedTests(unittest.TestCase):
    def test_uses_api_timestamp_not_run_time(self):
        out = doughcon.format_updated(fixture("full.json"))
        self.assertIn("2026-06-03", out)
        self.assertIn("CST", out)
        self.assertIn("美東", out)

    def test_formats_to_minutes_not_seconds(self):
        out = doughcon.format_updated(fixture("full.json"))
        self.assertNotIn(":38", out, "API path is minute-resolution")

    def test_missing_timestamp_falls_back_to_run_time_with_seconds(self):
        out = doughcon.format_updated(fixture("no_timestamp.json"))
        self.assertTrue(out.endswith("CST"))
        # cst_now() is second-resolution: HH:MM:SS
        self.assertRegex(out, r"\d{2}:\d{2}:\d{2} CST$")

    def test_unparseable_timestamp_falls_back_silently(self):
        out = doughcon.format_updated({"timestamp": "not-a-date"})
        self.assertRegex(out, r"\d{2}:\d{2}:\d{2} CST$")


class DeliverModeTests(unittest.TestCase):
    def _run(self, argv, fetch_result=None, fetch_raises=None):
        code = 0

        def fake_fetch():
            if fetch_raises:
                raise fetch_raises
            return fetch_result

        with capture_fds() as (out_f, err_f):
            with mock.patch.object(doughcon, "fetch_doughcon", fake_fetch), \
                 mock.patch.object(sys, "argv", ["run.py"] + argv):
                try:
                    doughcon.main()
                except SystemExit as e:
                    code = e.code or 0
            sys.stdout.flush()
            sys.stderr.flush()
            out, err = read_fd_capture(out_f), read_fd_capture(err_f)
        return code, out, err

    def test_deliver_no_chat_prints_body_and_marks_ok(self):
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "t-1"}, clear=True):
            code, out, err = self._run([], fetch_result=fixture("full.json"))
        self.assertEqual(code, 0)
        self.assertIn("🍕 DOUGHCON 情報", out)
        self.assertIn("目前等級：DOUGHCON 3", out)
        self.assertIn("指數：42", out)
        self.assertIn("`t-1`", out, "job id is appended to the body in deliver mode")
        self.assertIn("[skill-status:ok]", out)
        self.assertIn("[trace:t-1]", out)

    def test_no_data_marks_degraded_not_ok(self):
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "t-2"}, clear=True):
            code, out, _ = self._run([], fetch_result=fixture("all_null.json"))
        self.assertEqual(code, 0)
        self.assertIn("指數：-1", out)
        self.assertIn("[skill-status:degraded]", out)

    def test_upstream_failure_is_degraded_exit_zero(self):
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "t-3"}, clear=True):
            code, out, _ = self._run([], fetch_raises=RuntimeError("boom"))
        self.assertEqual(code, 0, "upstream failure is a soft degrade, not exit 1")
        self.assertIn("[WARN: doughcon unavailable", out)
        self.assertIn("[skill-status:degraded]", out)

    def test_markers_absent_without_job_id(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            code, out, _ = self._run([], fetch_result=fixture("full.json"))
        self.assertEqual(code, 0)
        self.assertNotIn("[skill-status:", out)
        self.assertNotIn("[trace:", out)
        self.assertNotIn("`", out, "no job-id suffix on the body either")


class DstGateTests(unittest.TestCase):
    def _run_gate(self, et_hour, job_id="t-g"):
        code = 0
        with capture_fds() as (out_f, err_f):
            with mock.patch.object(sys, "argv", ["run.py", "--et-hour", str(et_hour)]), \
                 mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": job_id}, clear=True):
                try:
                    doughcon.main()
                except SystemExit as e:
                    code = e.code or 0
            sys.stdout.flush()
            sys.stderr.flush()
            out, err = read_fd_capture(out_f), read_fd_capture(err_f)
        return code, out, err

    def test_gate_mismatch_is_ok_with_markers_and_no_body(self):
        from datetime import datetime
        wrong = (datetime.now(doughcon._NY).hour + 5) % 24
        code, out, err = self._run_gate(wrong)
        self.assertEqual(code, 0)
        self.assertIn("[skip: US-Eastern hour", err)
        self.assertIn("[skill-status:ok]", out)
        self.assertIn("[trace:t-g]", out)
        self.assertNotIn("DOUGHCON 情報", out)

    def test_out_of_range_hour_is_accepted_and_skips(self):
        # argparse does NOT validate 0-23. -1 and 99 are permanent skips.
        code, out, err = self._run_gate(99)
        self.assertEqual(code, 0)
        self.assertIn("[skip:", err)


if __name__ == "__main__":
    unittest.main(verbosity=2)
