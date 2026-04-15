"""Unit tests for oilcon/scripts/run.py formatting helpers."""
import importlib.util
import os
import sys
import unittest
from unittest import mock

LIB_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_DIR = os.path.dirname(LIB_DIR)
RUN_PATH = os.path.join(REPO_DIR, "oilcon", "scripts", "run.py")

sys.path.insert(0, LIB_DIR)

spec = importlib.util.spec_from_file_location("oilcon_run", RUN_PATH)
oilcon_run = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(oilcon_run)


class OilconRunTests(unittest.TestCase):
    def test_format_message_ok(self):
        snapshot = oilcon_run.Snapshot(
            symbols={
                "WTI": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 77.0),
                        ("2026-04-15", 78.2),
                    ]
                ),
                "Brent": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 80.0),
                        ("2026-04-15", 80.64),
                    ]
                ),
                "HO": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 2.40),
                        ("2026-04-15", 2.45),
                    ]
                ),
            }
        )
        message, status = oilcon_run.format_message(snapshot)
        self.assertEqual(status, "ok")
        self.assertIn("WTI: $78.20 (+1.6%)", message)
        self.assertIn("確認：Brent ✓ (+0.8%)   HO ✓ (+2.1%)", message)

    def test_format_message_degraded_warning(self):
        snapshot = oilcon_run.Snapshot(
            symbols={
                "WTI": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 77.0),
                        ("2026-04-15", 78.2),
                    ]
                ),
                "Brent": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 80.0),
                        ("2026-04-15", 79.6),
                    ]
                ),
                "HO": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 2.40),
                        ("2026-04-15", 2.45),
                    ]
                ),
            },
            warning="latest quote unavailable",
        )
        message, status = oilcon_run.format_message(snapshot)
        self.assertEqual(status, "degraded")
        self.assertIn("[WARN: latest quote unavailable]", message)
        self.assertIn("確認：Brent ✗ (-0.5%)   HO ✓ (+2.1%)", message)

    def test_flat_confirmation_renders_en_dash(self):
        snapshot = oilcon_run.Snapshot(
            symbols={
                "WTI": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 77.0),
                        ("2026-04-15", 77.0),
                    ]
                ),
                "Brent": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 80.0),
                        ("2026-04-15", 80.5),
                    ]
                ),
                "HO": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 2.40),
                        ("2026-04-15", 2.40),
                    ]
                ),
            }
        )
        message, status = oilcon_run.format_message(snapshot)
        self.assertEqual(status, "ok")
        self.assertIn("確認：Brent – (+0.6%)   HO – (+0.0%)", message)

    def test_confirmation_symbol_with_short_history_renders_na(self):
        snapshot = oilcon_run.Snapshot(
            symbols={
                "WTI": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 77.0),
                        ("2026-04-15", 78.2),
                    ]
                ),
                "Brent": oilcon_run.SymbolSnapshot(rows=None),
                "HO": oilcon_run.SymbolSnapshot(
                    rows=[
                        ("2026-04-14", 2.40),
                        ("2026-04-15", 2.45),
                    ],
                    stale=True,
                ),
            }
        )
        message, status = oilcon_run.format_message(snapshot)
        self.assertEqual(status, "ok")
        self.assertIn("確認：Brent n/a   HO ✓ (+2.1%) (stale)", message)

    def test_format_record_line_requires_fresh_data(self):
        snapshot = oilcon_run.Snapshot(
            symbols={
                "WTI": oilcon_run.SymbolSnapshot(rows=[("2026-04-14", 77.0), ("2026-04-15", 78.2)]),
                "Brent": oilcon_run.SymbolSnapshot(rows=[("2026-04-14", 80.0), ("2026-04-15", 80.64)]),
                "HO": oilcon_run.SymbolSnapshot(rows=[("2026-04-14", 2.40), ("2026-04-15", 2.45)], stale=True),
            }
        )
        with self.assertRaises(ValueError):
            oilcon_run.format_record_line(snapshot)

    def test_build_snapshot_marks_latest_fetch_failure_as_stale(self):
        class FakeConn:
            def close(self):
                return None

        conn = FakeConn()

        def fake_window(_, symbol, limit):
            rows = {
                "CL=F": [("2026-04-14", 77.0), ("2026-04-15", 78.2)] * 10,
                "BZ=F": [("2026-04-14", 80.0), ("2026-04-15", 80.64)] * 10,
                "HO=F": [("2026-04-14", 2.40), ("2026-04-15", 2.45)] * 10,
            }
            return rows[symbol][:limit]

        with mock.patch.object(oilcon_run.oil_store, "connect_from_env", return_value=conn), \
                mock.patch.object(oilcon_run.oil_store, "ensure_schema"), \
                mock.patch.object(oilcon_run.oil_store, "needs_backfill", return_value=False), \
                mock.patch.object(oilcon_run.oil_store, "window", side_effect=fake_window), \
                mock.patch.object(oilcon_run.oil_store, "upsert"), \
                mock.patch.object(oilcon_run.oil_fetch, "fetch_latest", side_effect=[RuntimeError("boom"), ("2026-04-15", 80.64), ("2026-04-15", 2.45)]):
            snapshot = oilcon_run.build_snapshot()

        self.assertIsNone(snapshot.warning)
        self.assertTrue(snapshot.symbols["WTI"].stale)
        self.assertFalse(snapshot.symbols["Brent"].stale)


if __name__ == "__main__":
    unittest.main()
