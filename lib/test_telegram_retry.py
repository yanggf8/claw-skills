"""Unit tests for lib/telegram.py retry + deadline behavior.

Run: python3 ~/.nullclaw/skills/lib/test_telegram_retry.py
"""
import io
import os
import sys
import time
import unittest
from unittest import mock

# Ensure lib/ is importable when run directly.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import telegram  # noqa: E402


class FakeResponse:
    def __init__(self, status: int):
        self.status = status

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


def _http_error(code: int) -> Exception:
    import urllib.error
    return urllib.error.HTTPError(
        url="https://api.telegram.org/", code=code, msg="x",
        hdrs=None, fp=io.BytesIO(b"err body"),
    )


class TelegramRetryTests(unittest.TestCase):
    def setUp(self):
        # Skip the real config read.
        self._token_patch = mock.patch.object(telegram, "get_bot_token", return_value="fake")
        self._token_patch.start()
        # Make backoff effectively instant so tests run fast.
        self._backoff_patch = mock.patch.object(telegram, "BACKOFFS", (0.0, 0.0))
        self._backoff_patch.start()
        # Silence the [telegram] stderr noise during normal runs.
        self._log_patch = mock.patch.object(telegram, "_log")
        self._log_patch.start()

    def tearDown(self):
        self._token_patch.stop()
        self._backoff_patch.stop()
        self._log_patch.stop()

    def test_success_first_try(self):
        with mock.patch("urllib.request.urlopen", return_value=FakeResponse(200)) as op:
            self.assertTrue(telegram.send("chat", "hi"))
        self.assertEqual(op.call_count, 1)

    def test_502_502_200(self):
        responses = [_http_error(502), _http_error(502), FakeResponse(200)]

        def fake(req, timeout):
            r = responses.pop(0)
            if isinstance(r, Exception):
                raise r
            return r

        with mock.patch("urllib.request.urlopen", side_effect=fake) as op:
            self.assertTrue(telegram.send("chat", "hi"))
        self.assertEqual(op.call_count, 3)

    def test_three_502_returns_false(self):
        with mock.patch("urllib.request.urlopen", side_effect=_http_error(502)) as op:
            self.assertFalse(telegram.send("chat", "hi"))
        self.assertEqual(op.call_count, 3)

    def test_403_no_retry(self):
        with mock.patch("urllib.request.urlopen", side_effect=_http_error(403)) as op:
            self.assertFalse(telegram.send("chat", "hi"))
        self.assertEqual(op.call_count, 1)

    def test_429_retries(self):
        responses = [_http_error(429), FakeResponse(200)]

        def fake(req, timeout):
            r = responses.pop(0)
            if isinstance(r, Exception):
                raise r
            return r

        with mock.patch("urllib.request.urlopen", side_effect=fake) as op:
            self.assertTrue(telegram.send("chat", "hi"))
        self.assertEqual(op.call_count, 2)

    def test_url_error_retries(self):
        import urllib.error
        responses = [urllib.error.URLError("conn refused"), FakeResponse(200)]

        def fake(req, timeout):
            r = responses.pop(0)
            if isinstance(r, Exception):
                raise r
            return r

        with mock.patch("urllib.request.urlopen", side_effect=fake) as op:
            self.assertTrue(telegram.send("chat", "hi"))
        self.assertEqual(op.call_count, 2)

    def test_deadline_blocks_second_attempt(self):
        # Simulate an attempt that consumes the entire budget, leaving
        # nothing for a retry.
        def slow_fail(req, timeout):
            time.sleep(0.05)
            raise _http_error(502)

        # backoff is 0 so the only thing keeping us from retrying is the deadline.
        with mock.patch("urllib.request.urlopen", side_effect=slow_fail) as op:
            self.assertFalse(telegram.send("chat", "hi", deadline_s=0.04))
        # First attempt fires (per_attempt clamped), then deadline check fails.
        self.assertEqual(op.call_count, 1)

    def test_zero_deadline_skips(self):
        with mock.patch("urllib.request.urlopen") as op:
            self.assertFalse(telegram.send("chat", "hi", deadline_s=0))
        self.assertEqual(op.call_count, 0)

    def test_no_token_returns_false_no_call(self):
        self._token_patch.stop()
        with mock.patch.object(telegram, "get_bot_token", return_value=None):
            with mock.patch("urllib.request.urlopen") as op:
                self.assertFalse(telegram.send("chat", "hi"))
            self.assertEqual(op.call_count, 0)
        # Re-arm so tearDown's stop() doesn't double-stop.
        self._token_patch = mock.patch.object(telegram, "get_bot_token", return_value="fake")
        self._token_patch.start()


if __name__ == "__main__":
    unittest.main()
