"""Unit tests for lib/trace_marker.py.

Run: python3 ~/.nullclaw/skills/lib/test_trace_marker.py
"""
import io
import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import trace_marker  # noqa: E402


class EmitSkillStatusTests(unittest.TestCase):
    def _capture(self, fn):
        out = io.StringIO()
        fn(out)
        return out.getvalue()

    def test_noop_when_job_id_unset(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            out = self._capture(lambda s: trace_marker.emit_skill_status("ok", stream=s))
        self.assertEqual(out, "")

    def test_emits_when_job_id_set(self):
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "job-123:7"}, clear=True):
            out = self._capture(lambda s: trace_marker.emit_skill_status("ok", stream=s))
        self.assertEqual(out, "[skill-status:ok]\n")

    def test_emits_degraded(self):
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "job-123:7"}, clear=True):
            out = self._capture(lambda s: trace_marker.emit_skill_status("degraded", stream=s))
        self.assertEqual(out, "[skill-status:degraded]\n")

    def test_emits_failed(self):
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "job-123:7"}, clear=True):
            out = self._capture(lambda s: trace_marker.emit_skill_status("failed", stream=s))
        self.assertEqual(out, "[skill-status:failed]\n")

    def test_invalid_status_raises_regardless_of_env(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(ValueError):
                trace_marker.emit_skill_status("bogus")
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "x"}, clear=True):
            with self.assertRaises(ValueError):
                trace_marker.emit_skill_status("bogus")


class EmitTraceTests(unittest.TestCase):
    def _capture(self, fn):
        out = io.StringIO()
        fn(out)
        return out.getvalue()

    def test_noop_when_job_id_unset(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            out = self._capture(lambda s: trace_marker.emit_trace(stream=s))
        self.assertEqual(out, "")

    def test_emits_when_job_id_set(self):
        with mock.patch.dict(os.environ, {"NULLCLAW_JOB_ID": "job-abc:42"}, clear=True):
            out = self._capture(lambda s: trace_marker.emit_trace(stream=s))
        self.assertEqual(out, "[trace:job-abc:42]\n")


if __name__ == "__main__":
    unittest.main()
