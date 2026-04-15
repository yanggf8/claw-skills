"""Unit tests for lib/oil_store.py."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import oil_store  # noqa: E402


class OilStoreTests(unittest.TestCase):
    def setUp(self):
        self.conn = oil_store.connect(":memory:")
        oil_store.ensure_schema(self.conn)

    def tearDown(self):
        self.conn.close()

    def test_ensure_schema_idempotent(self):
        oil_store.ensure_schema(self.conn)
        oil_store.ensure_schema(self.conn)
        row = self.conn.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'oil_daily'"
        ).fetchone()
        self.assertEqual(row[0], "oil_daily")

    def test_needs_backfill_true_when_empty_false_after_insert(self):
        self.assertTrue(oil_store.needs_backfill(self.conn, "CL=F"))
        oil_store.upsert(self.conn, "CL=F", "2026-04-15", 78.2)
        self.assertFalse(oil_store.needs_backfill(self.conn, "CL=F"))

    def test_window_returns_chronological_rows(self):
        oil_store.insert_many(
            self.conn,
            "CL=F",
            [
                ("2026-04-13", 77.0),
                ("2026-04-14", 78.0),
                ("2026-04-15", 79.0),
            ],
        )
        rows = oil_store.window(self.conn, "CL=F", 2)
        self.assertEqual(rows, [("2026-04-14", 78.0), ("2026-04-15", 79.0)])

    def test_upsert_replaces_existing_row(self):
        oil_store.upsert(self.conn, "CL=F", "2026-04-15", 78.2)
        oil_store.upsert(self.conn, "CL=F", "2026-04-15", 79.4)
        latest = oil_store.latest_row(self.conn, "CL=F")
        self.assertEqual(latest, ("2026-04-15", 79.4))


if __name__ == "__main__":
    unittest.main()
