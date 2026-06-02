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

    def test_moving_average_basic(self):
        rows = [("d1", 10.0), ("d2", 20.0), ("d3", 30.0), ("d4", 40.0), ("d5", 50.0)]
        self.assertEqual(oilcon_run.moving_average(rows, 3), 40.0)

    def test_moving_average_insufficient_rows(self):
        rows = [("d1", 10.0), ("d2", 20.0)]
        with self.assertRaises(ValueError):
            oilcon_run.moving_average(rows, 3)

    def test_ma_rising_true(self):
        # 50MA should be rising when recent prices are higher than 20 days ago
        rows = [(f"d{i}", float(50 + i)) for i in range(70)]
        self.assertTrue(oilcon_run.ma_rising(rows, 50, 20))

    def test_ma_rising_false(self):
        # 50MA should not be rising when recent prices are lower
        rows = [(f"d{i}", float(100 - i)) for i in range(70)]
        self.assertFalse(oilcon_run.ma_rising(rows, 50, 20))

    def test_ma_rising_insufficient_history(self):
        rows = [(f"d{i}", float(i)) for i in range(30)]
        self.assertFalse(oilcon_run.ma_rising(rows, 50, 20))

    def test_pct_below_60d_high_basic(self):
        rows = [(f"d{i}", float(90 + i)) for i in range(60)]
        # Current is 149, high is 149, so 0% below
        self.assertAlmostEqual(oilcon_run.pct_below_60d_high(rows), 0.0, places=1)

    def test_pct_below_60d_high_below_high(self):
        rows = [(f"d{i}", float(100 + i * 0.5)) for i in range(55)]
        rows.append(("d55", 120.0))  # high at 127.0, current 120
        # Need to build proper rows - let's use simpler: high 100, current 90
        rows2 = [(f"d{i}", float(50 + min(i, 50))) for i in range(55)]
        rows2.append(("d55", 90.0))  # current 90, high was 100
        self.assertGreater(oilcon_run.pct_below_60d_high(rows2), 0.0)

    def test_classify_oil_trend_uptrend(self):
        # Price > 50MA, 50MA rising, <=10% below 60d high
        rows = [(f"d{i}", float(80 + i * 0.3)) for i in range(70)]
        state = oilcon_run.classify_oil_trend(rows)
        self.assertEqual(state, "uptrend")

    def test_classify_oil_trend_weakening_uptrend(self):
        # Price > 50MA, 50MA rising, >10% below 60d high
        # Build: sustained rise (80+ rows), then recent drop >10% from high
        rows = [(f"d{i}", float(30 + i * 0.9)) for i in range(75)]
        # High ~97, drop to 85 (>12% below) but still above rising 50MA
        rows.append(("d75", 85.0))
        state = oilcon_run.classify_oil_trend(rows)
        self.assertEqual(state, "weakening-uptrend")

    def test_classify_oil_trend_rollover_price_below_ma(self):
        # Price < 50MA but 50MA still rising -> rollover (price crossed, MA not yet)
        # Build: rise steadily, then sharp drop below MA
        rows = [(f"d{i}", float(50 + i * 0.8)) for i in range(60)]
        # Add 10 more rows that drop below the MA
        for i in range(10):
            rows.append((f"d{60+i}", 90.0 - i * 2.0))
        state = oilcon_run.classify_oil_trend(rows)
        self.assertEqual(state, "rollover")

    def test_classify_oil_trend_no_uptrend(self):
        # Price < 50MA AND 50MA flat/falling -> no-uptrend
        rows = [(f"d{i}", float(100 - i)) for i in range(70)]
        state = oilcon_run.classify_oil_trend(rows)
        self.assertEqual(state, "no-uptrend")

    def test_classify_oil_trend_insufficient_history(self):
        rows = [(f"d{i}", float(i)) for i in range(30)]
        state = oilcon_run.classify_oil_trend(rows)
        self.assertIn("insufficient", state.lower())

    def test_classify_oil_trend_price_exactly_equal_ma_not_uptrend(self):
        # Price exactly == 50MA with rising MA → NOT "uptrend" (strict > required)
        rows = [(f"d{i}", float(50 + i * 0.5)) for i in range(70)]
        # Force last price to equal the 50MA
        ma50 = oilcon_run.moving_average(rows, 50)
        rows[-1] = (rows[-1][0], ma50)
        state = oilcon_run.classify_oil_trend(rows)
        self.assertNotEqual(state, "uptrend")

    def test_classify_oil_trend_69_rows_insufficient_70_rows_sufficient(self):
        # 69 rows (steadily rising) → insufficient-history
        rows_69 = [(f"d{i}", float(30 + i * 0.5)) for i in range(69)]
        state_69 = oilcon_run.classify_oil_trend(rows_69)
        self.assertIn("insufficient", state_69.lower())
        # 70 rows (steadily rising) → uptrend
        rows_70 = [(f"d{i}", float(30 + i * 0.5)) for i in range(70)]
        state_70 = oilcon_run.classify_oil_trend(rows_70)
        self.assertEqual(state_70, "uptrend")

    def test_format_message_emits_oil_trend_no_jets_verdict(self):
        rows = [(f"d{i}", float(30 + i * 0.9)) for i in range(75)]
        rows.append(("d75", 85.0))
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
        self.assertIn("OIL-TREND:", message)
        self.assertNotIn("JETS", message)
        self.assertNotIn("review reduce", message.lower())

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
