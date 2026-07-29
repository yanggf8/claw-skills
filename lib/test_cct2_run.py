"""Unit tests for cct2/scripts/run.py delivery gating.

Regression cover for the 2026-07-29 duplicate-delivery incident: cct2
pre-market put two Telegram messages in front of the user a minute apart, both
stamped with the same trace id :3818. The first attempt got nothing from the
dual LLM, delivered "⚠️ 無法取得任何分析結果", and only then reported
[skill-status:failed]; nullclaw retried the run (cron.zig:5622) with an
identical environment, and the rescued attempt delivered a second time.

A skill cannot detect that it is the retry — the scheduler re-execs with
`retry_child.env_map = &skill_env`, so NULLCLAW_JOB_ID is byte-identical and
there is no attempt counter. The only fix available skill-side is to not
deliver on the hard-failure path (option A, recorded in CLAUDE.md).

Run directly, no pytest needed:

    python3 lib/test_cct2_run.py
"""
import importlib.util
import os
import sys
import unittest
from unittest import mock

LIB_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_DIR = os.path.dirname(LIB_DIR)
RUN_PATH = os.path.join(REPO_DIR, "cct2", "scripts", "run.py")

sys.path.insert(0, LIB_DIR)

spec = importlib.util.spec_from_file_location("cct2_run", RUN_PATH)
cct2_run = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(cct2_run)

CHAT = "7972814626"
ROW = {"symbol": "AAPL", "sentiment": "bullish", "confidence": 0.9}


class DeliveryTargetTests(unittest.TestCase):
    """The rule in isolation: no analysis rows means no Telegram target."""

    def test_rows_present_delivers_to_chat(self):
        self.assertEqual(cct2_run.delivery_target(CHAT, [ROW]), CHAT)

    def test_no_rows_suppresses_telegram(self):
        self.assertIsNone(cct2_run.delivery_target(CHAT, []))

    def test_absent_chat_id_stays_none(self):
        """Manual runs (no --deliver-to) keep echoing to stdout as before."""
        self.assertIsNone(cct2_run.delivery_target(None, [ROW]))


class MainDeliveryGateTests(unittest.TestCase):
    """The gate has to be wired into main(), not merely defined."""

    def _run_main(self, rows):
        patches = {
            "load_skill_config": mock.patch.object(cct2_run, "load_skill_config", return_value={}),
            "load_tickers": mock.patch.object(cct2_run, "load_tickers", return_value=["AAPL"]),
            "fetch_all": mock.patch.object(cct2_run, "fetch_all", return_value={}),
            "build_ticker_summary": mock.patch.object(cct2_run, "build_ticker_summary", return_value=""),
            "run_dual_llm": mock.patch.object(cct2_run, "run_dual_llm", return_value=(None, None)),
            "merge_results": mock.patch.object(cct2_run, "merge_results", return_value=rows),
            "format_report": mock.patch.object(cct2_run, "format_report", return_value="body"),
            "emit_trace": mock.patch.object(cct2_run, "emit_trace"),
        }
        started = {k: p.start() for k, p in patches.items()}
        self.addCleanup(lambda: [p.stop() for p in patches.values()])

        with mock.patch.object(cct2_run, "deliver_or_fail") as deliver, \
             mock.patch.object(cct2_run, "emit_skill_status") as status, \
             mock.patch.object(
                 sys, "argv",
                 ["run.py", "--mode", "pre-market", "--deliver-to", CHAT],
             ):
            cct2_run.main()
        del started
        return deliver, status

    def test_empty_rows_never_reaches_telegram(self):
        deliver, _ = self._run_main([])
        deliver.assert_called_once()
        self.assertIsNone(
            deliver.call_args.args[0],
            "an empty result must not be delivered — the scheduler retry duplicates it",
        )

    def test_rows_present_still_reaches_telegram(self):
        deliver, _ = self._run_main([ROW])
        deliver.assert_called_once()
        self.assertEqual(deliver.call_args.args[0], CHAT)

    def test_suppressing_delivery_does_not_suppress_the_marker(self):
        """Silence on Telegram, but the scheduler must still see the contract."""
        _, status = self._run_main([])
        status.assert_called_once_with("failed")

    def test_ok_marker_unchanged_when_rows_present(self):
        _, status = self._run_main([ROW])
        status.assert_called_once_with("ok")


if __name__ == "__main__":
    unittest.main(verbosity=2)
