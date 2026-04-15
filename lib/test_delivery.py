"""Unit tests for lib/delivery.py.

Run: python3 ~/.nullclaw/skills/lib/test_delivery.py
"""
import io
import os
import sys
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import delivery  # noqa: E402
import telegram  # noqa: E402


class DeliveryTests(unittest.TestCase):
    def _capture(self, fn):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            try:
                rv = fn()
                exit_code = None
            except SystemExit as e:
                rv = None
                exit_code = e.code
        return rv, exit_code, out.getvalue(), err.getvalue()

    def test_empty_chat_prints_to_stdout(self):
        rv, code, stdout, stderr = self._capture(
            lambda: delivery.deliver_or_fail("", "the body")
        )
        self.assertEqual(rv, True)
        self.assertIsNone(code)
        self.assertEqual(stdout.strip(), "the body")
        self.assertEqual(stderr, "")

    def test_none_chat_prints_to_stdout(self):
        rv, code, stdout, stderr = self._capture(
            lambda: delivery.deliver_or_fail(None, "the body")
        )
        self.assertEqual(rv, True)
        self.assertEqual(stdout.strip(), "the body")

    def test_send_success_no_output(self):
        with mock.patch.object(telegram, "send", return_value=True) as m:
            rv, code, stdout, stderr = self._capture(
                lambda: delivery.deliver_or_fail("chat-1", "body", account="alt")
            )
        self.assertEqual(rv, True)
        self.assertEqual(stdout, "")
        self.assertEqual(stderr, "")
        m.assert_called_once()
        # account passthrough
        self.assertEqual(m.call_args.kwargs["account"], "alt")

    def test_send_failure_default_exits(self):
        with mock.patch.object(telegram, "send", return_value=False):
            rv, code, stdout, stderr = self._capture(
                lambda: delivery.deliver_or_fail("chat-1", "body")
            )
        self.assertEqual(code, 1)
        # body on stdout (data goes to stdout for cron capture)
        self.assertIn("body", stdout)
        # error line on stderr
        self.assertIn("[delivery]", stderr)
        self.assertIn("chat=chat-1", stderr)

    def test_send_failure_opt_out_returns_false(self):
        with mock.patch.object(telegram, "send", return_value=False):
            rv, code, stdout, stderr = self._capture(
                lambda: delivery.deliver_or_fail(
                    "chat-1", "body", fail_on_delivery_error=False
                )
            )
        self.assertEqual(rv, False)
        self.assertIsNone(code)
        self.assertIn("body", stdout)
        self.assertIn("[delivery]", stderr)

    def test_deadline_resolved_from_env(self):
        captured = {}

        def fake_send(chat_id, text, account="main", *, deadline_s=None):
            captured["deadline_s"] = deadline_s
            return True

        with mock.patch.object(telegram, "send", side_effect=fake_send):
            with mock.patch.dict(os.environ, {"NULLCLAW_SKILL_TIMEOUT": "60"}, clear=False):
                self._capture(lambda: delivery.deliver_or_fail("chat", "body"))
        # 60 - 1s safety margin
        self.assertAlmostEqual(captured["deadline_s"], 59.0, places=1)

    def test_no_env_passes_none_deadline(self):
        captured = {}

        def fake_send(chat_id, text, account="main", *, deadline_s=None):
            captured["deadline_s"] = deadline_s
            return True

        env_clean = {k: v for k, v in os.environ.items()
                     if k not in ("NULLCLAW_SKILL_TIMEOUT", "NULLCLAW_SKILL_STARTED")}
        with mock.patch.object(telegram, "send", side_effect=fake_send):
            with mock.patch.dict(os.environ, env_clean, clear=True):
                self._capture(lambda: delivery.deliver_or_fail("chat", "body"))
        self.assertIsNone(captured["deadline_s"])

    def test_invalid_timeout_env_falls_back(self):
        captured = {}

        def fake_send(chat_id, text, account="main", *, deadline_s=None):
            captured["deadline_s"] = deadline_s
            return True

        with mock.patch.object(telegram, "send", side_effect=fake_send):
            with mock.patch.dict(os.environ, {"NULLCLAW_SKILL_TIMEOUT": "not-a-number"}, clear=False):
                self._capture(lambda: delivery.deliver_or_fail("chat", "body"))
        self.assertIsNone(captured["deadline_s"])


if __name__ == "__main__":
    unittest.main()
