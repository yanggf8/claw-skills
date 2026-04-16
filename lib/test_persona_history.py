"""Unit tests for lib/persona_history.py."""
import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import persona_history  # noqa: E402


class PersonaHistoryTests(unittest.TestCase):
    def setUp(self):
        self.conn = persona_history.connect(":memory:")
        persona_history.ensure_schema(self.conn)

    def tearDown(self):
        self.conn.close()

    # ---------- schema ----------

    def test_ensure_schema_idempotent(self):
        persona_history.ensure_schema(self.conn)
        persona_history.ensure_schema(self.conn)
        row = self.conn.execute(
            "SELECT name FROM sqlite_master "
            "WHERE type = 'table' AND name = 'persona_history'"
        ).fetchone()
        self.assertEqual(row[0], "persona_history")

    def test_ensure_schema_bumps_user_version(self):
        row = self.conn.execute(
            "SELECT version FROM schema_version WHERE name = ?",
            (persona_history.SCHEMA_NAME,),
        ).fetchone()
        self.assertEqual(int(row[0]), persona_history.SCHEMA_VERSION)

    def test_ensure_schema_skips_already_applied_migrations(self):
        # A fresh connection already at the latest version should not re-run
        # migrations. We simulate by dropping the table manually and
        # confirming ensure_schema does not recreate it when user_version is
        # already current.
        self.conn.execute("DROP TABLE persona_history")
        self.conn.commit()
        persona_history.ensure_schema(self.conn)
        row = self.conn.execute(
            "SELECT name FROM sqlite_master "
            "WHERE type = 'table' AND name = 'persona_history'"
        ).fetchone()
        self.assertIsNone(row)

    # ---------- record + recent round-trip ----------

    def _record_sample(self, **overrides):
        args = dict(
            skill="mindfulness-spirit",
            stream="mindfulness",
            persona_slug="ping-w",
            date="2026-04-16",
            title="晨光與沈默",
            stance="在 AI 加速的日常裡，保留一段手寫般的寧靜。",
            key_links=["https://example.com/a", "https://example.com/b"],
            writer_hash="sha256:abc1234567890def",
        )
        args.update(overrides)
        return persona_history.record(self.conn, **args)

    def test_record_returns_row_id(self):
        row_id = self._record_sample()
        self.assertIsInstance(row_id, int)
        self.assertGreaterEqual(row_id, 1)

    def test_record_and_recent_roundtrip(self):
        self._record_sample()
        rows = persona_history.recent(self.conn, persona_slug="ping-w")
        self.assertEqual(len(rows), 1)
        got = rows[0]
        self.assertEqual(got.skill, "mindfulness-spirit")
        self.assertEqual(got.stream, "mindfulness")
        self.assertEqual(got.persona_slug, "ping-w")
        self.assertEqual(got.editor_slug, None)
        self.assertEqual(got.date, "2026-04-16")
        self.assertEqual(got.title, "晨光與沈默")
        self.assertEqual(
            got.stance,
            "在 AI 加速的日常裡，保留一段手寫般的寧靜。",
        )
        self.assertEqual(got.key_links, [
            "https://example.com/a",
            "https://example.com/b",
        ])
        self.assertFalse(got.git_dirty)
        self.assertEqual(got.writer_hash, "sha256:abc1234567890def")
        self.assertIsNone(got.devto_id)

    def test_record_key_links_stored_as_json(self):
        self._record_sample(key_links=["https://x/1"])
        raw = self.conn.execute(
            "SELECT key_links FROM persona_history LIMIT 1"
        ).fetchone()[0]
        self.assertEqual(json.loads(raw), ["https://x/1"])

    def test_record_rejects_non_list_key_links(self):
        with self.assertRaises(TypeError):
            persona_history.record(
                self.conn,
                skill="mindfulness-spirit",
                stream="mindfulness",
                persona_slug="ping-w",
                date="2026-04-16",
                title="t",
                stance="s",
                key_links="not-a-list",  # type: ignore[arg-type]
                writer_hash="sha256:deadbeefdeadbeef",
            )

    def test_record_preserves_git_dirty_flag(self):
        self._record_sample(git_dirty=True)
        row = persona_history.recent(self.conn, persona_slug="ping-w")[0]
        self.assertTrue(row.git_dirty)

    # ---------- recent: filtering, ordering, limits ----------

    def test_recent_requires_at_least_one_filter(self):
        with self.assertRaises(ValueError):
            persona_history.recent(self.conn)

    def test_recent_rejects_nonpositive_limit(self):
        with self.assertRaises(ValueError):
            persona_history.recent(self.conn, persona_slug="ping-w", limit=0)

    def test_recent_orders_date_desc_then_id_desc(self):
        # Two rows same date: id tiebreaker must put the later-inserted first.
        self._record_sample(date="2026-04-15", title="first")
        self._record_sample(date="2026-04-15", title="second")
        self._record_sample(date="2026-04-14", title="older")

        rows = persona_history.recent(self.conn, persona_slug="ping-w")
        self.assertEqual(
            [r.title for r in rows],
            ["second", "first", "older"],
        )

    def test_recent_respects_limit(self):
        for i in range(5):
            self._record_sample(date=f"2026-04-{10 + i:02d}", title=f"t{i}")
        rows = persona_history.recent(self.conn, persona_slug="ping-w", limit=2)
        self.assertEqual(len(rows), 2)

    def test_recent_default_limit_is_eight(self):
        for i in range(12):
            self._record_sample(date=f"2026-04-{i + 1:02d}", title=f"t{i}")
        rows = persona_history.recent(self.conn, persona_slug="ping-w")
        self.assertEqual(len(rows), 8)

    def test_recent_filters_by_skill(self):
        self._record_sample(skill="mindfulness-spirit", title="mind-1")
        self._record_sample(skill="ainews", stream="ai-weekly", title="ai-1")
        rows = persona_history.recent(self.conn, skill="ainews")
        self.assertEqual([r.title for r in rows], ["ai-1"])

    def test_recent_filters_by_stream(self):
        self._record_sample(skill="ainews", stream="ai-weekly", title="weekly")
        self._record_sample(skill="ainews", stream="turboquant", title="tq")
        rows = persona_history.recent(
            self.conn, skill="ainews", stream="turboquant"
        )
        self.assertEqual([r.title for r in rows], ["tq"])

    def test_recent_cross_skill_persona_query(self):
        # Same persona, two skills — persona query returns both, skill-scoped
        # query returns only the matching one. This is the headline use case.
        self._record_sample(skill="mindfulness-spirit", title="mind")
        self._record_sample(skill="ainews", stream="ai-weekly", title="ai")

        everything = persona_history.recent(self.conn, persona_slug="ping-w")
        self.assertEqual({r.title for r in everything}, {"mind", "ai"})

        only_ainews = persona_history.recent(
            self.conn, persona_slug="ping-w", skill="ainews"
        )
        self.assertEqual([r.title for r in only_ainews], ["ai"])

    # ---------- set_devto_result ----------

    def test_set_devto_result_updates_only_target_row(self):
        first = self._record_sample(title="first")
        second = self._record_sample(title="second", date="2026-04-17")

        persona_history.set_devto_result(
            self.conn,
            row_id=second,
            devto_id=9999,
            devto_url="https://dev.to/foo/second-abcd",
        )

        rows = {
            r.id: r
            for r in persona_history.recent(self.conn, persona_slug="ping-w")
        }
        self.assertEqual(rows[second].devto_id, 9999)
        self.assertEqual(rows[second].devto_url, "https://dev.to/foo/second-abcd")
        self.assertIsNone(rows[first].devto_id)
        self.assertIsNone(rows[first].devto_url)

    # ---------- persona_hash ----------

    def test_persona_hash_stable_across_key_order(self):
        a = {"slug": "ping-w", "role": "心行者", "name": "Ping W."}
        b = {"name": "Ping W.", "role": "心行者", "slug": "ping-w"}
        self.assertEqual(
            persona_history.persona_hash(a),
            persona_history.persona_hash(b),
        )

    def test_persona_hash_changes_with_content(self):
        a = {"slug": "ping-w", "role": "心行者"}
        b = {"slug": "ping-w", "role": "觀行者"}
        self.assertNotEqual(
            persona_history.persona_hash(a),
            persona_history.persona_hash(b),
        )

    def test_persona_hash_format(self):
        h = persona_history.persona_hash({"slug": "x", "role": "y"})
        self.assertTrue(h.startswith("sha256:"))
        self.assertEqual(len(h), len("sha256:") + 16)

    # ---------- connect_from_env ----------

    def _save_env(self):
        keys = [
            persona_history.DB_URL_ENV,
            persona_history.DB_TOKEN_ENV,
            persona_history._DEPRECATED_URL_ENV,
            persona_history._DEPRECATED_TOKEN_ENV,
        ]
        return {k: os.environ.get(k) for k in keys}

    def _restore_env(self, saved):
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        persona_history._deprecation_warned = False

    def test_connect_from_env_rejects_remote_without_token(self):
        saved = self._save_env()
        try:
            os.environ[persona_history.DB_URL_ENV] = "libsql://example.turso.io"
            os.environ.pop(persona_history.DB_TOKEN_ENV, None)
            os.environ.pop(persona_history._DEPRECATED_URL_ENV, None)
            with self.assertRaises(persona_history.MissingCredentialsError):
                persona_history.connect_from_env()
        finally:
            self._restore_env(saved)

    def test_connect_from_env_raises_when_url_unset(self):
        saved = self._save_env()
        try:
            os.environ.pop(persona_history.DB_URL_ENV, None)
            os.environ.pop(persona_history._DEPRECATED_URL_ENV, None)
            with self.assertRaises(persona_history.MissingCredentialsError):
                persona_history.connect_from_env()
        finally:
            self._restore_env(saved)

    def test_connect_from_env_deprecated_alias_works(self):
        saved = self._save_env()
        try:
            os.environ.pop(persona_history.DB_URL_ENV, None)
            os.environ.pop(persona_history.DB_TOKEN_ENV, None)
            os.environ[persona_history._DEPRECATED_URL_ENV] = ":memory:"
            os.environ.pop(persona_history._DEPRECATED_TOKEN_ENV, None)
            persona_history._deprecation_warned = False
            conn = persona_history.connect_from_env()
            conn.close()
        finally:
            self._restore_env(saved)

    def test_connect_from_env_memory_bypasses_token(self):
        saved = self._save_env()
        try:
            os.environ[persona_history.DB_URL_ENV] = ":memory:"
            os.environ.pop(persona_history.DB_TOKEN_ENV, None)
            os.environ.pop(persona_history._DEPRECATED_URL_ENV, None)
            conn = persona_history.connect_from_env()
            conn.close()
        finally:
            self._restore_env(saved)


if __name__ == "__main__":
    unittest.main()
