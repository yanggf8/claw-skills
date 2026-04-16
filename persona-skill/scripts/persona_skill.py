#!/usr/bin/env python3
"""persona-skill CLI — Turso-backed writer persona registry."""

from __future__ import annotations

import argparse
import getpass
import json
import os
import re
import sys
from dataclasses import asdict
from pathlib import Path

import yaml

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))

import persona_registry  # noqa: E402

_SLUG_RE = re.compile(r"^[a-z][a-z0-9-]*$")

REQUIRED_FIELDS: tuple[str, ...] = ("slug", "role")
OPTIONAL_FIELDS: tuple[str, ...] = (
    "name",
    "expression",
    "mental_models",
    "heuristics",
    "antipatterns",
    "limits",
)
ALL_FIELDS: tuple[str, ...] = REQUIRED_FIELDS + OPTIONAL_FIELDS
FORBIDDEN_KEYS: tuple[str, ...] = ("secrets",)


class ValidationError(Exception):
    pass


def _validate_persona_yaml(data: dict, filename_stem: str | None = None) -> dict:
    if not isinstance(data, dict):
        raise ValidationError("expected mapping")

    unknown = set(data.keys()) - set(ALL_FIELDS)
    for fk in FORBIDDEN_KEYS:
        if fk in data:
            raise ValidationError(f"'{fk}' key is forbidden in YAML — use set-secret CLI")
    if unknown - set(FORBIDDEN_KEYS):
        raise ValidationError(
            f"unknown top-level key(s): {', '.join(sorted(unknown - set(FORBIDDEN_KEYS)))}"
        )

    slug = data.get("slug")
    if not isinstance(slug, str):
        raise ValidationError("slug is required and must be a string")
    if not _SLUG_RE.match(slug):
        raise ValidationError(f"slug '{slug}' does not match ^[a-z][a-z0-9-]*$")
    if filename_stem is not None and slug != filename_stem:
        raise ValidationError(
            f"slug '{slug}' does not match filename stem '{filename_stem}'"
        )

    role = data.get("role")
    if not isinstance(role, str) or not role.strip():
        raise ValidationError("role is required and must be a non-empty string")

    out: dict[str, str | None] = {"slug": slug, "role": role}
    for field in OPTIONAL_FIELDS:
        value = data.get(field)
        if value is not None and not isinstance(value, str):
            raise ValidationError(f"{field} must be a string or null")
        out[field] = value

    return out


def _yaml_to_persona(validated: dict) -> persona_registry.Persona:
    return persona_registry.Persona(
        slug=validated["slug"],
        role=validated["role"],
        name=validated.get("name"),
        expression=validated.get("expression"),
        mental_models=validated.get("mental_models"),
        heuristics=validated.get("heuristics"),
        antipatterns=validated.get("antipatterns"),
        limits=validated.get("limits"),
    )


def _open_db():
    try:
        conn = persona_registry.connect_from_env()
        persona_registry.ensure_schema(conn)
        return conn
    except persona_registry.MissingCredentialsError as e:
        print(f"credentials error: {e}", file=sys.stderr)
        sys.exit(1)


def cmd_get(args: argparse.Namespace) -> None:
    conn = _open_db()
    try:
        p = persona_registry.get(conn, args.slug)
    except persona_registry.PersonaNotFound:
        print(f"unknown slug: {args.slug}", file=sys.stderr)
        sys.exit(2)
    finally:
        conn.close()
    d = asdict(p)
    d.pop("hash_version", None)
    print(json.dumps(d, ensure_ascii=False))


def cmd_list(args: argparse.Namespace) -> None:
    conn = _open_db()
    slugs = persona_registry.list_slugs(conn)
    conn.close()
    print(json.dumps(slugs, ensure_ascii=False))


def cmd_upsert(args: argparse.Namespace) -> None:
    path = Path(args.file)
    if not path.is_file():
        print(f"file not found: {path}", file=sys.stderr)
        sys.exit(1)

    try:
        raw = path.read_text(encoding="utf-8")
        data = yaml.safe_load(raw)
    except (OSError, yaml.YAMLError) as exc:
        print(f"error reading {path}: {exc}", file=sys.stderr)
        sys.exit(1)

    try:
        validated = _validate_persona_yaml(data, path.stem)
    except ValidationError as exc:
        print(f"validation error: {exc}", file=sys.stderr)
        sys.exit(3)

    p = _yaml_to_persona(validated)
    conn = _open_db()
    persona_registry.upsert(conn, p)
    conn.commit()
    conn.close()
    print(f"upserted: {p.slug}")


def cmd_set_secret(args: argparse.Namespace) -> None:
    if sys.stdin.isatty():
        value = getpass.getpass(f"Enter value for {args.slug}/{args.kind}: ")
    else:
        value = sys.stdin.readline().rstrip("\n")
    if not value:
        print("empty value — aborting", file=sys.stderr)
        sys.exit(3)

    conn = _open_db()
    try:
        persona_registry.get(conn, args.slug)
    except persona_registry.PersonaNotFound:
        print(f"unknown slug: {args.slug}", file=sys.stderr)
        conn.close()
        sys.exit(2)

    persona_registry.set_secret(conn, args.slug, args.kind, value)
    conn.commit()
    conn.close()
    print(f"secret set: {args.slug}/{args.kind}")


def cmd_list_secrets(args: argparse.Namespace) -> None:
    conn = _open_db()
    kinds = persona_registry.list_secret_kinds(conn, args.slug)
    conn.close()
    print(json.dumps(kinds, ensure_ascii=False))


def cmd_delete(args: argparse.Namespace) -> None:
    conn = _open_db()
    persona_registry.delete(conn, args.slug)
    conn.commit()
    conn.close()
    print(f"deleted: {args.slug}")


def cmd_validate(args: argparse.Namespace) -> None:
    if args.file:
        targets = [Path(args.file)]
    else:
        script_dir = Path(__file__).resolve().parent
        personas_dir = script_dir.parent / "personas"
        if not personas_dir.is_dir():
            print("personas directory not found", file=sys.stderr)
            sys.exit(1)
        targets = sorted(personas_dir.glob("*.yaml"))

    all_ok = True
    for path in targets:
        try:
            raw = path.read_text(encoding="utf-8")
            data = yaml.safe_load(raw)
            _validate_persona_yaml(data, path.stem)
            print(f"OK {path}")
        except (OSError, yaml.YAMLError, ValidationError) as exc:
            print(f"FAIL {path}: {exc}")
            all_ok = False

    sys.exit(0 if all_ok else 3)


def cmd_migrate_from_yaml(args: argparse.Namespace) -> None:
    yaml_dir = Path(args.dir)
    if not yaml_dir.is_dir():
        print(f"directory not found: {yaml_dir}", file=sys.stderr)
        sys.exit(1)

    conn = _open_db()
    count = 0
    for path in sorted(yaml_dir.glob("*.yaml")):
        try:
            raw = path.read_text(encoding="utf-8")
            data = yaml.safe_load(raw)
            validated = _validate_persona_yaml(data, path.stem)
            p = _yaml_to_persona(validated)
            persona_registry.upsert(conn, p)
            count += 1
            print(f"  upserted: {p.slug}")
        except (OSError, yaml.YAMLError, ValidationError) as exc:
            print(f"  SKIP {path}: {exc}", file=sys.stderr)

    conn.commit()
    conn.close()
    print(f"migrated {count} persona(s)")


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="persona_skill.py",
        description="persona-skill — Turso-backed writer persona registry CLI",
    )
    sub = parser.add_subparsers(dest="command")

    p_get = sub.add_parser("get", help="Get a persona by slug (JSON, no secrets)")
    p_get.add_argument("slug")

    sub.add_parser("list", help="List all persona slugs (JSON array)")

    p_upsert = sub.add_parser("upsert", help="Upsert a persona from YAML")
    p_upsert.add_argument("file", help="Path to YAML file")

    p_secret = sub.add_parser("set-secret", help="Set a per-persona secret (reads stdin)")
    p_secret.add_argument("slug")
    p_secret.add_argument("kind")

    p_lsec = sub.add_parser("list-secrets", help="List secret kinds for a persona")
    p_lsec.add_argument("slug")

    p_del = sub.add_parser("delete", help="Delete a persona (cascades secrets, keeps history)")
    p_del.add_argument("slug")

    p_val = sub.add_parser("validate", help="Validate YAML files (no DB)")
    p_val.add_argument("file", nargs="?", default=None)

    p_mig = sub.add_parser("migrate-from-yaml", help="Bulk upsert from a YAML directory")
    p_mig.add_argument("dir", help="Directory containing persona YAML files")

    args = parser.parse_args()

    commands = {
        "get": cmd_get,
        "list": cmd_list,
        "upsert": cmd_upsert,
        "set-secret": cmd_set_secret,
        "list-secrets": cmd_list_secrets,
        "delete": cmd_delete,
        "validate": cmd_validate,
        "migrate-from-yaml": cmd_migrate_from_yaml,
    }

    handler = commands.get(args.command)
    if handler:
        handler(args)
    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    main()
