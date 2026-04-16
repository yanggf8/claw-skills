"""Unit tests for lib/persona_registry.py."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import persona_registry  # noqa: E402


class PersonaRegistryTests(unittest.TestCase):
    def setUp(self):
        self.conn = persona_registry.connect(":memory:")
        persona_registry.ensure_schema(self.conn)

    def tearDown(self):
        self.conn.close()

    # ---------- schema ----------

    def test_ensure_schema_idempotent(self):
        persona_registry.ensure_schema(self.conn)
        persona_registry.ensure_schema(self.conn)
        row = self.conn.execute(
            "SELECT name FROM sqlite_master "
            "WHERE type = 'table' AND name = 'persona'"
        ).fetchone()
        self.assertEqual(row[0], "persona")

    def test_ensure_schema_bumps_user_version(self):
        row = self.conn.execute(
            "SELECT version FROM schema_version WHERE name = ?",
            (persona_registry.SCHEMA_NAME,),
        ).fetchone()
        self.assertEqual(int(row[0]), persona_registry.SCHEMA_VERSION)

    def test_ensure_schema_skips_already_applied(self):
        self.conn.execute("DROP TABLE persona_secret")
        self.conn.execute("DROP TABLE persona")
        self.conn.commit()
        persona_registry.ensure_schema(self.conn)
        row = self.conn.execute(
            "SELECT name FROM sqlite_master "
            "WHERE type = 'table' AND name = 'persona'"
        ).fetchone()
        self.assertIsNone(row)

    def test_both_tables_created(self):
        tables = {
            r[0]
            for r in self.conn.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }
        self.assertIn("persona", tables)
        self.assertIn("persona_secret", tables)

    # ---------- upsert + get round-trip ----------

    def _sample_persona(self, **overrides):
        defaults = dict(
            slug="ping-w",
            role="在宗教界服務的心行者",
            name="Ping W.",
            expression="語氣沉穩、不急不徐。",
            mental_models="contemplative",
            heuristics="prefer concrete imagery",
            antipatterns="avoid jargon",
            limits="800 words max",
            hash_version=1,
        )
        defaults.update(overrides)
        return persona_registry.Persona(**defaults)

    def test_upsert_and_get_roundtrip(self):
        p = self._sample_persona()
        persona_registry.upsert(self.conn, p)
        got = persona_registry.get(self.conn, "ping-w")
        self.assertEqual(got.slug, p.slug)
        self.assertEqual(got.role, p.role)
        self.assertEqual(got.name, p.name)
        self.assertEqual(got.expression, p.expression)
        self.assertEqual(got.mental_models, p.mental_models)
        self.assertEqual(got.heuristics, p.heuristics)
        self.assertEqual(got.antipatterns, p.antipatterns)
        self.assertEqual(got.limits, p.limits)
        self.assertEqual(got.hash_version, p.hash_version)

    def test_upsert_updates_without_changing_created_at(self):
        persona_registry.upsert(self.conn, self._sample_persona())
        row1 = self.conn.execute(
            "SELECT created_at, updated_at FROM persona WHERE slug = 'ping-w'"
        ).fetchone()

        persona_registry.upsert(
            self.conn, self._sample_persona(role="updated role")
        )
        row2 = self.conn.execute(
            "SELECT created_at, updated_at FROM persona WHERE slug = 'ping-w'"
        ).fetchone()

        self.assertEqual(row1[0], row2[0])

    def test_upsert_with_optional_none_fields(self):
        p = persona_registry.Persona(slug="minimal", role="tester")
        persona_registry.upsert(self.conn, p)
        got = persona_registry.get(self.conn, "minimal")
        self.assertEqual(got.slug, "minimal")
        self.assertEqual(got.role, "tester")
        self.assertIsNone(got.name)
        self.assertIsNone(got.expression)

    # ---------- get: not found ----------

    def test_get_raises_persona_not_found(self):
        with self.assertRaises(persona_registry.PersonaNotFound):
            persona_registry.get(self.conn, "nonexistent")

    # ---------- list_slugs ----------

    def test_list_slugs_empty(self):
        self.assertEqual(persona_registry.list_slugs(self.conn), [])

    def test_list_slugs_ordered(self):
        for slug in ("charlie", "alice", "bob"):
            persona_registry.upsert(
                self.conn, persona_registry.Persona(slug=slug, role="r")
            )
        self.assertEqual(
            persona_registry.list_slugs(self.conn),
            ["alice", "bob", "charlie"],
        )

    # ---------- delete ----------

    def test_delete_removes_persona(self):
        persona_registry.upsert(self.conn, self._sample_persona())
        persona_registry.delete(self.conn, "ping-w")
        self.assertEqual(persona_registry.list_slugs(self.conn), [])

    def test_delete_cascades_to_secrets(self):
        persona_registry.upsert(self.conn, self._sample_persona())
        persona_registry.set_secret(self.conn, "ping-w", "devto_api_key", "k1")
        persona_registry.delete(self.conn, "ping-w")
        kinds = persona_registry.list_secret_kinds(self.conn, "ping-w")
        self.assertEqual(kinds, [])

    def test_delete_nonexistent_is_silent(self):
        persona_registry.delete(self.conn, "ghost")

    # ---------- secrets ----------

    def test_set_and_get_secret(self):
        persona_registry.upsert(self.conn, self._sample_persona())
        persona_registry.set_secret(
            self.conn, "ping-w", "devto_api_key", "secret123"
        )
        self.assertEqual(
            persona_registry.get_secret(self.conn, "ping-w", "devto_api_key"),
            "secret123",
        )

    def test_get_secret_returns_none_when_missing(self):
        persona_registry.upsert(self.conn, self._sample_persona())
        self.assertIsNone(
            persona_registry.get_secret(self.conn, "ping-w", "nonexistent")
        )

    def test_rotate_secret_updates_value(self):
        persona_registry.upsert(self.conn, self._sample_persona())
        persona_registry.set_secret(
            self.conn, "ping-w", "devto_api_key", "old"
        )
        persona_registry.rotate_secret(
            self.conn, "ping-w", "devto_api_key", "new"
        )
        self.assertEqual(
            persona_registry.get_secret(self.conn, "ping-w", "devto_api_key"),
            "new",
        )

    def test_list_secret_kinds_returns_names_only(self):
        persona_registry.upsert(self.conn, self._sample_persona())
        persona_registry.set_secret(self.conn, "ping-w", "devto_api_key", "v1")
        persona_registry.set_secret(self.conn, "ping-w", "medium_token", "v2")
        kinds = persona_registry.list_secret_kinds(self.conn, "ping-w")
        self.assertEqual(kinds, ["devto_api_key", "medium_token"])

    def test_list_secret_kinds_empty(self):
        persona_registry.upsert(self.conn, self._sample_persona())
        self.assertEqual(
            persona_registry.list_secret_kinds(self.conn, "ping-w"), []
        )

    def test_delete_secret(self):
        persona_registry.upsert(self.conn, self._sample_persona())
        persona_registry.set_secret(self.conn, "ping-w", "devto_api_key", "v1")
        persona_registry.delete_secret(self.conn, "ping-w", "devto_api_key")
        self.assertIsNone(
            persona_registry.get_secret(self.conn, "ping-w", "devto_api_key")
        )

    # ---------- persona_hash ----------

    def test_persona_hash_stable_across_field_order(self):
        p = self._sample_persona()
        h1 = persona_registry.persona_hash(p)
        h2 = persona_registry.persona_hash(p)
        self.assertEqual(h1, h2)

    def test_persona_hash_changes_with_content(self):
        p1 = self._sample_persona(role="心行者")
        p2 = self._sample_persona(role="觀行者")
        self.assertNotEqual(
            persona_registry.persona_hash(p1),
            persona_registry.persona_hash(p2),
        )

    def test_persona_hash_format(self):
        h = persona_registry.persona_hash(self._sample_persona())
        self.assertTrue(h.startswith("sha256:"))
        self.assertEqual(len(h), len("sha256:") + 16)

    def test_persona_hash_rejects_unknown_version(self):
        p = self._sample_persona(hash_version=99)
        with self.assertRaises(ValueError):
            persona_registry.persona_hash(p)

    def test_persona_hash_ignores_non_whitelisted_attrs(self):
        p1 = self._sample_persona()
        p2 = self._sample_persona()
        self.assertEqual(
            persona_registry.persona_hash(p1),
            persona_registry.persona_hash(p2),
        )

    # ---------- connect_from_env ----------

    def test_connect_from_env_raises_when_url_unset(self):
        saved = os.environ.get(persona_registry.DB_URL_ENV)
        try:
            os.environ.pop(persona_registry.DB_URL_ENV, None)
            with self.assertRaises(persona_registry.MissingCredentialsError):
                persona_registry.connect_from_env()
        finally:
            if saved is not None:
                os.environ[persona_registry.DB_URL_ENV] = saved

    def test_connect_from_env_raises_when_remote_without_token(self):
        saved_url = os.environ.get(persona_registry.DB_URL_ENV)
        saved_token = os.environ.get(persona_registry.DB_TOKEN_ENV)
        try:
            os.environ[persona_registry.DB_URL_ENV] = "libsql://example.turso.io"
            os.environ.pop(persona_registry.DB_TOKEN_ENV, None)
            with self.assertRaises(persona_registry.MissingCredentialsError):
                persona_registry.connect_from_env()
        finally:
            if saved_url is None:
                os.environ.pop(persona_registry.DB_URL_ENV, None)
            else:
                os.environ[persona_registry.DB_URL_ENV] = saved_url
            if saved_token is not None:
                os.environ[persona_registry.DB_TOKEN_ENV] = saved_token

    def test_connect_from_env_memory_bypasses_token(self):
        saved_url = os.environ.get(persona_registry.DB_URL_ENV)
        saved_token = os.environ.get(persona_registry.DB_TOKEN_ENV)
        try:
            os.environ[persona_registry.DB_URL_ENV] = ":memory:"
            os.environ.pop(persona_registry.DB_TOKEN_ENV, None)
            conn = persona_registry.connect_from_env()
            conn.close()
        finally:
            if saved_url is None:
                os.environ.pop(persona_registry.DB_URL_ENV, None)
            else:
                os.environ[persona_registry.DB_URL_ENV] = saved_url
            if saved_token is not None:
                os.environ[persona_registry.DB_TOKEN_ENV] = saved_token


if __name__ == "__main__":
    unittest.main()
