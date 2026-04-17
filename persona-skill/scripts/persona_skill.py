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

import persona_history  # noqa: E402
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


def _read_multiline(prompt: str) -> str | None:
    """Read multi-line input from stdin. Empty input returns None."""
    if sys.stdin.isatty():
        print(f"{prompt} (blank line to finish, empty to skip):")
        lines = []
        while True:
            line = input()
            if line == "":
                break
            lines.append(line)
        return "\n".join(lines) + "\n" if lines else None
    else:
        text = sys.stdin.read().strip()
        return text if text else None


def _build_persona_from_args(args: argparse.Namespace, existing: persona_registry.Persona | None = None) -> persona_registry.Persona:
    """Build a Persona from CLI --field args, falling back to existing values for update."""
    def resolve(field: str) -> str | None:
        val = getattr(args, field, None)
        if val is not None:
            return val
        if existing is not None:
            return getattr(existing, field, None)
        return None

    slug = args.slug if hasattr(args, "slug") else existing.slug
    role = resolve("role")
    if not role:
        print("role is required", file=sys.stderr)
        sys.exit(3)

    return persona_registry.Persona(
        slug=slug,
        role=role,
        name=resolve("name"),
        expression=resolve("expression"),
        mental_models=resolve("mental_models"),
        heuristics=resolve("heuristics"),
        antipatterns=resolve("antipatterns"),
        limits=resolve("limits"),
    )


def cmd_create(args: argparse.Namespace) -> None:
    if not _SLUG_RE.match(args.slug):
        print(f"slug '{args.slug}' does not match ^[a-z][a-z0-9-]*$", file=sys.stderr)
        sys.exit(3)

    conn = _open_db()
    try:
        persona_registry.get(conn, args.slug)
        print(f"slug '{args.slug}' already exists — use 'update' to modify", file=sys.stderr)
        conn.close()
        sys.exit(1)
    except persona_registry.PersonaNotFound:
        pass

    p = _build_persona_from_args(args)
    persona_registry.upsert(conn, p)
    conn.commit()
    conn.close()
    print(f"created: {p.slug}")


def cmd_update(args: argparse.Namespace) -> None:
    conn = _open_db()
    try:
        existing = persona_registry.get(conn, args.slug)
    except persona_registry.PersonaNotFound:
        print(f"unknown slug: {args.slug}", file=sys.stderr)
        conn.close()
        sys.exit(2)

    p = _build_persona_from_args(args, existing=existing)
    persona_registry.upsert(conn, p)
    conn.commit()
    conn.close()
    print(f"updated: {p.slug}")


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


def cmd_get_secret(args: argparse.Namespace) -> None:
    conn = _open_db()
    try:
        persona_registry.get(conn, args.slug)
    except persona_registry.PersonaNotFound:
        print(f"unknown slug: {args.slug}", file=sys.stderr)
        conn.close()
        sys.exit(2)

    value = persona_registry.get_secret(conn, args.slug, args.kind)
    conn.close()
    if value is None:
        print(f"no secret: {args.slug}/{args.kind}", file=sys.stderr)
        sys.exit(2)
    if args.reveal:
        if not sys.stdout.isatty():
            print("--reveal requires an interactive terminal", file=sys.stderr)
            sys.exit(1)
        print(value)
    else:
        print(f"len={len(value)} prefix={value[:4]}...")


def cmd_delete_secret(args: argparse.Namespace) -> None:
    conn = _open_db()
    try:
        persona_registry.get(conn, args.slug)
    except persona_registry.PersonaNotFound:
        print(f"unknown slug: {args.slug}", file=sys.stderr)
        conn.close()
        sys.exit(2)

    existing = persona_registry.get_secret(conn, args.slug, args.kind)
    if existing is None:
        print(f"no secret: {args.slug}/{args.kind}", file=sys.stderr)
        conn.close()
        sys.exit(2)

    persona_registry.delete_secret(conn, args.slug, args.kind)
    conn.commit()
    conn.close()
    print(f"deleted secret: {args.slug}/{args.kind}")


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


def _open_history_db():
    conn = _open_db()
    persona_history.ensure_schema(conn)
    return conn


def cmd_history(args: argparse.Namespace) -> None:
    conn = _open_history_db()

    kwargs: dict = {"limit": args.limit}
    if args.skill:
        kwargs["skill"] = args.skill
    if args.stream:
        kwargs["stream"] = args.stream
    if args.persona:
        kwargs["persona_slug"] = args.persona

    if "skill" not in kwargs and "persona_slug" not in kwargs:
        print("at least one of --skill or --persona is required", file=sys.stderr)
        conn.close()
        sys.exit(1)

    rows = persona_history.recent(conn, **kwargs)
    conn.close()

    if args.json:
        from dataclasses import asdict as _asdict
        print(json.dumps([_asdict(r) for r in rows], ensure_ascii=False, default=str))
    else:
        if not rows:
            print("(no history)")
            return
        for r in rows:
            devto = f" devto={r.devto_id}" if r.devto_id else ""
            print(f"{r.date}  {r.skill}/{r.stream}  [{r.persona_slug}]{devto}")
            print(f"  {r.title}")
            stance = r.stance[:80] + ("…" if len(r.stance) > 80 else "")
            print(f"  {stance}")


def cmd_plan_list(args: argparse.Namespace) -> None:
    conn = _open_history_db()
    plans = persona_history.list_plans(conn, skill=args.skill)
    conn.close()

    if args.json:
        from dataclasses import asdict as _asdict
        print(json.dumps([_asdict(p) for p in plans], ensure_ascii=False, default=str))
    else:
        if not plans:
            print("(no plans)")
            return
        for p in plans:
            print(f"{p.month}  {p.skill}/{p.series_slug}  {p.series_title}")


def cmd_plan_show(args: argparse.Namespace) -> None:
    conn = _open_history_db()
    plan = persona_history.get_plan(conn, skill=args.skill, series_slug=args.series)
    if plan is None:
        print(f"no plan: {args.skill}/{args.series}", file=sys.stderr)
        conn.close()
        sys.exit(2)

    topics = persona_history.list_topics(conn, plan_id=plan.id)
    conn.close()

    if args.json:
        from dataclasses import asdict as _asdict
        out = _asdict(plan)
        out["topics"] = [_asdict(t) for t in topics]
        print(json.dumps(out, ensure_ascii=False, default=str))
    else:
        print(f"Series: {plan.series_title}")
        print(f"Start:  {plan.month}")
        if plan.series_theme:
            print(f"Theme:  {plan.series_theme[:120]}…" if len(plan.series_theme or "") > 120 else f"Theme:  {plan.series_theme}")
        print()
        for t in topics:
            status_icon = {"planned": "○", "published": "●", "skipped": "×"}.get(t.status, "?")
            print(f"  {status_icon} W{t.week} {t.target_date}  [{t.lens}] [{t.direction}]")
            print(f"    {t.title_hint}")
            if t.key_question:
                print(f"    Q: {t.key_question}")


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="persona_skill.py",
        description="persona-skill — Turso-backed writer persona registry CLI",
    )
    sub = parser.add_subparsers(dest="command")

    p_get = sub.add_parser("get", help="Get a persona by slug (JSON, no secrets)")
    p_get.add_argument("slug")

    sub.add_parser("list", help="List all persona slugs (JSON array)")

    p_upsert = sub.add_parser("upsert", help="Upsert a persona from YAML (legacy)")
    p_upsert.add_argument("file", help="Path to YAML file")

    def _add_persona_fields(p, role_required=True):
        p.add_argument("--role", required=role_required, help="Role (required for create)")
        p.add_argument("--name", default=None, help="Display name")
        p.add_argument("--expression", default=None, help="Writing expression/voice")
        p.add_argument("--mental-models", dest="mental_models", default=None, help="Mental models")
        p.add_argument("--heuristics", default=None, help="Writing heuristics")
        p.add_argument("--antipatterns", default=None, help="Antipatterns to avoid")
        p.add_argument("--limits", default=None, help="Scope limits")

    p_create = sub.add_parser("create", help="Create a persona directly in Turso")
    p_create.add_argument("slug")
    _add_persona_fields(p_create, role_required=True)

    p_update = sub.add_parser("update", help="Update persona fields (unspecified fields keep current value)")
    p_update.add_argument("slug")
    _add_persona_fields(p_update, role_required=False)

    p_secret = sub.add_parser("set-secret", help="Set a per-persona secret (reads stdin)")
    p_secret.add_argument("slug")
    p_secret.add_argument("kind")

    p_gsec = sub.add_parser("get-secret", help="Show a secret (masked unless --reveal)")
    p_gsec.add_argument("slug")
    p_gsec.add_argument("kind")
    p_gsec.add_argument("--reveal", action="store_true", help="Print full value")

    p_lsec = sub.add_parser("list-secrets", help="List secret kinds for a persona")
    p_lsec.add_argument("slug")

    p_dsec = sub.add_parser("delete-secret", help="Delete a single secret")
    p_dsec.add_argument("slug")
    p_dsec.add_argument("kind")

    p_del = sub.add_parser("delete", help="Delete a persona (cascades secrets, keeps history)")
    p_del.add_argument("slug")

    p_val = sub.add_parser("validate", help="Validate YAML files (no DB)")
    p_val.add_argument("file", nargs="?", default=None)

    p_mig = sub.add_parser("migrate-from-yaml", help="Bulk upsert from a YAML directory")
    p_mig.add_argument("dir", help="Directory containing persona YAML files")

    p_hist = sub.add_parser("history", help="Show recent publish history")
    p_hist.add_argument("--skill", help="Filter by skill name")
    p_hist.add_argument("--stream", help="Filter by stream name")
    p_hist.add_argument("--persona", help="Filter by persona slug")
    p_hist.add_argument("--limit", type=int, default=10, help="Max rows (default 10)")
    p_hist.add_argument("--json", action="store_true", help="Output as JSON array")

    p_plist = sub.add_parser("plan-list", help="List editorial plans")
    p_plist.add_argument("--skill", default=None, help="Filter by skill")
    p_plist.add_argument("--json", action="store_true")

    p_pshow = sub.add_parser("plan-show", help="Show plan with topics")
    p_pshow.add_argument("skill")
    p_pshow.add_argument("series", help="Series slug")
    p_pshow.add_argument("--json", action="store_true")

    args = parser.parse_args()

    commands = {
        "get": cmd_get,
        "list": cmd_list,
        "create": cmd_create,
        "update": cmd_update,
        "upsert": cmd_upsert,
        "set-secret": cmd_set_secret,
        "get-secret": cmd_get_secret,
        "list-secrets": cmd_list_secrets,
        "delete-secret": cmd_delete_secret,
        "delete": cmd_delete,
        "validate": cmd_validate,
        "migrate-from-yaml": cmd_migrate_from_yaml,
        "history": cmd_history,
        "plan-list": cmd_plan_list,
        "plan-show": cmd_plan_show,
    }

    handler = commands.get(args.command)
    if handler:
        handler(args)
    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    main()
