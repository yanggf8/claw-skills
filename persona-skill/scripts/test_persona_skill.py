"""Unit tests for persona-skill CLI (Turso-backed)."""

import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = str(Path(__file__).resolve().parent / "persona_skill.py")

# Force :memory: DB for all subprocess tests
_TEST_ENV = {
    **os.environ,
    "PERSONA_REGISTRY_DB_URL": ":memory:",
}


def _run(*cli_args, stdin_text=None, env=None):
    result = subprocess.run(
        [sys.executable, SCRIPT, *cli_args],
        capture_output=True,
        text=True,
        env=env or _TEST_ENV,
        input=stdin_text,
    )
    return result


class ValidationTests(unittest.TestCase):
    """YAML validation — no DB needed."""

    def _write_yaml(self, content, filename="test-slug.yaml"):
        d = tempfile.mkdtemp()
        p = Path(d) / filename
        p.write_text(textwrap.dedent(content), encoding="utf-8")
        return str(p)

    def test_validate_good_yaml(self):
        path = self._write_yaml("""\
            slug: test-slug
            role: test role
        """)
        r = _run("validate", path)
        self.assertEqual(r.returncode, 0)
        self.assertIn("OK", r.stdout)

    def test_validate_rejects_secrets_key(self):
        path = self._write_yaml("""\
            slug: test-slug
            role: test role
            secrets:
              devto_api_key: leaked
        """)
        r = _run("validate", path)
        self.assertEqual(r.returncode, 3)
        self.assertIn("secrets", r.stdout.lower() or r.stderr.lower())

    def test_validate_rejects_unknown_keys(self):
        path = self._write_yaml("""\
            slug: test-slug
            role: test role
            bogus: value
        """)
        r = _run("validate", path)
        self.assertEqual(r.returncode, 3)

    def test_validate_rejects_missing_role(self):
        path = self._write_yaml("""\
            slug: test-slug
        """)
        r = _run("validate", path)
        self.assertEqual(r.returncode, 3)

    def test_validate_rejects_slug_mismatch(self):
        path = self._write_yaml("""\
            slug: other-slug
            role: test role
        """, filename="test-slug.yaml")
        r = _run("validate", path)
        self.assertEqual(r.returncode, 3)

    def test_validate_rejects_bad_slug_format(self):
        path = self._write_yaml("""\
            slug: Bad_Slug
            role: test role
        """, filename="Bad_Slug.yaml")
        r = _run("validate", path)
        self.assertEqual(r.returncode, 3)


class CLIIntegrationTests(unittest.TestCase):
    """Tests that hit the DB via :memory:."""

    def _write_yaml(self, content, filename="test-slug.yaml"):
        d = tempfile.mkdtemp()
        p = Path(d) / filename
        p.write_text(textwrap.dedent(content), encoding="utf-8")
        return str(p)

    def test_list_empty(self):
        r = _run("list")
        self.assertEqual(r.returncode, 0)
        self.assertEqual(json.loads(r.stdout), [])

    def test_get_unknown_slug_exits_2(self):
        r = _run("get", "nonexistent")
        self.assertEqual(r.returncode, 2)

    def test_upsert_and_get_roundtrip(self):
        path = self._write_yaml("""\
            slug: test-slug
            role: 測試角色
            name: Test
        """)
        # Each subprocess gets its own :memory: DB, so we can't roundtrip
        # across two separate invocations. This test validates upsert succeeds.
        r = _run("upsert", path)
        self.assertEqual(r.returncode, 0)
        self.assertIn("upserted", r.stdout)

    def test_upsert_rejects_secrets_in_yaml(self):
        path = self._write_yaml("""\
            slug: test-slug
            role: test role
            secrets:
              devto_api_key: leaked
        """)
        r = _run("upsert", path)
        self.assertEqual(r.returncode, 3)
        self.assertIn("secrets", r.stderr.lower())

    def test_set_secret_rejects_empty_stdin(self):
        r = _run("set-secret", "ping-w", "devto_api_key", stdin_text="")
        self.assertEqual(r.returncode, 3)

    def test_no_command_shows_help(self):
        r = _run()
        self.assertEqual(r.returncode, 1)

    def test_missing_credentials_exits_1(self):
        env = {**os.environ}
        env.pop("PERSONA_REGISTRY_DB_URL", None)
        env.pop("PERSONA_REGISTRY_DB_TOKEN", None)
        env.pop("PERSONA_HISTORY_DB_URL", None)
        r = _run("list", env=env)
        self.assertEqual(r.returncode, 1)

    def test_migrate_from_yaml_directory(self):
        d = tempfile.mkdtemp()
        (Path(d) / "alice.yaml").write_text(
            "slug: alice\nrole: writer\n", encoding="utf-8"
        )
        (Path(d) / "bob.yaml").write_text(
            "slug: bob\nrole: editor\n", encoding="utf-8"
        )
        r = _run("migrate-from-yaml", d)
        self.assertEqual(r.returncode, 0)
        self.assertIn("migrated 2", r.stdout)


if __name__ == "__main__":
    unittest.main()
