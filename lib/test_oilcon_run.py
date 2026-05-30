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

    def test_is_rising_rising_series(self):
        rows = [("d1", 60.0), ("d2", 62.0), ("d3", 64.0), ("d4", 66.0), ("d5", 68.0)]
        self.assertTrue(oilcon_run.is_rising(rows))

    def test_is_rising_falling_series(self):
        rows = [("d1", 68.0), ("d2", 66.0), ("d3", 64.0), ("d4", 62.0), ("d5", 60.0)]
        self.assertFalse(oilcon_run.is_rising(rows))

    def test_is_rising_flat_series(self):
        rows = [("d1", 65.0), ("d2", 65.0), ("d3", 65.0), ("d4", 65.0), ("d5", 65.0)]
        self.assertFalse(oilcon_run.is_rising(rows))

    def test_is_rising_fewer_than_two_rows(self):
        self.assertFalse(oilcon_run.is_rising([("d1", 65.0)]))

    def test_is_rising_fewer_than_window(self):
        rows = [("d1", 60.0), ("d2", 65.0)]
        self.assertTrue(oilcon_run.is_rising(rows))

    def _make_jets_rows(self, low=60.0, current=72.0, n=50, low_index=0):
        """Build synthetic rows where low is unique minimum at low_index."""
        rows = []
        base = low + 1.0  # all other rows are at least low+1
        step = (current - base) / max(n - 1, 1)
        for i in range(n):
            price = base + step * i
            rows.append((f"2026-01-{i+1:02d}", round(price, 2)))
        # force the unique low at low_index
        rows[low_index] = (rows[low_index][0], low)
        return rows

    def test_jets_oil_signal_met(self):
        rows = self._make_jets_rows(low=60.0, current=72.0, n=50, low_index=0)
        extremes = oilcon_run.compute_extremes(rows)
        self.assertTrue(oilcon_run.jets_oil_signal(extremes, rows))

    def test_jets_oil_signal_not_off_low_enough(self):
        rows = self._make_jets_rows(low=60.0, current=63.0, n=50, low_index=0)
        extremes = oilcon_run.compute_extremes(rows)
        self.assertFalse(oilcon_run.jets_oil_signal(extremes, rows))

    def test_jets_oil_signal_low_too_recent(self):
        rows = self._make_jets_rows(low=60.0, current=72.0, n=50, low_index=45)
        extremes = oilcon_run.compute_extremes(rows)
        self.assertFalse(oilcon_run.jets_oil_signal(extremes, rows))

    def test_jets_oil_signal_price_falling(self):
        n = 50
        rows = []
        for i in range(n - 5):
            rows.append((f"2026-01-{i+1:02d}", 60.0 + i * 0.3))
        # last 5 bars declining
        peak = rows[-1][1]
        for i in range(5):
            rows.append((f"2026-01-{n-5+i+1:02d}", round(peak - (i + 1) * 1.0, 2)))
        extremes = oilcon_run.compute_extremes(rows)
        # distance_off_low_pct and days_since_low should pass, but is_rising fails
        self.assertFalse(oilcon_run.jets_oil_signal(extremes, rows))

    def test_format_message_includes_jets_line_when_met(self):
        rows = self._make_jets_rows(low=60.0, current=72.0, n=50, low_index=0)
        snapshot = oilcon_run.Snapshot(
            symbols={
                "WTI": oilcon_run.SymbolSnapshot(rows=rows),
                "Brent": oilcon_run.SymbolSnapshot(
                    rows=[("2026-04-14", 80.0), ("2026-04-15", 80.5)]
                ),
                "HO": oilcon_run.SymbolSnapshot(
                    rows=[("2026-04-14", 2.40), ("2026-04-15", 2.45)]
                ),
            }
        )
        message, status = oilcon_run.format_message(snapshot)
        self.assertIn("JETS: oil in sustained uptrend", message)
        self.assertIn("review entry-exit-rules.md JETS Reduce Rule", message)

    def test_format_message_omits_jets_line_when_not_met(self):
        snapshot = oilcon_run.Snapshot(
            symbols={
                "WTI": oilcon_run.SymbolSnapshot(
                    rows=[("2026-04-14", 77.0), ("2026-04-15", 78.2)]
                ),
                "Brent": oilcon_run.SymbolSnapshot(
                    rows=[("2026-04-14", 80.0), ("2026-04-15", 80.5)]
                ),
                "HO": oilcon_run.SymbolSnapshot(
                    rows=[("2026-04-14", 2.40), ("2026-04-15", 2.45)]
                ),
            }
        )
        message, status = oilcon_run.format_message(snapshot)
        self.assertNotIn("JETS", message)

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
