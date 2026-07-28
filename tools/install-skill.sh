#!/usr/bin/env bash
# Build, verify, stage, and atomically publish a skill binary.
#
# Deliberately stricter than deploy.sh, which only does symlink bookkeeping and
# always exits 0. Nothing here is best-effort: if the artifact is missing or not
# executable, activation is REFUSED, because nullclaw does not check the path
# and would only discover it at fire time.
set -euo pipefail

skill=${1:?usage: install-skill.sh <skill-name>}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
dest_dir="$ROOT/$skill/bin"
dest="$dest_dir/$skill"
built="$ROOT/target/release/$skill"

echo "==> building $skill (locked)"
cargo build --release --locked -p "$skill"

[ -f "$built" ] || { echo "FAIL: $built not produced" >&2; exit 1; }
[ -x "$built" ] || { echo "FAIL: $built is not executable" >&2; exit 1; }

echo "==> staging"
mkdir -p "$dest_dir"
stage="$dest.staging.$$"
cp "$built" "$stage"
chmod +x "$stage"

echo "==> smoke-testing the staged artifact"
# Generic across skills: feed a flag no skill defines and require the binary to
# LOAD and reject it through its own argument parser (exit 2), rather than
# crashing or hanging. A skill-specific invocation was hardcoded here in Phase 1
# (doughcon's --et-hour), which made the installer refuse every other skill.
# Deliberately does NOT run a real invocation: weather would make live HTTP
# calls, which is not something an install step may do.
# NOTE: `set -e` aborts on a non-zero command, and the probe is EXPECTED to
# return 2 — so it must run inside an `if`, where set -e is suspended. Running
# it bare killed the script before this very check could execute. That failure
# mode is recorded in docs/specs/2026-07-28-phase1-lessons.md and was
# reintroduced here anyway.
if "$stage" --__install_smoke_probe__ >/dev/null 2>&1; then
  smoke_rc=0
else
  smoke_rc=$?
fi
if [ "$smoke_rc" -ne 2 ]; then
  rm -f "$stage"
  echo "FAIL: staged binary did not reject an unknown flag with exit 2 (got $smoke_rc)" >&2
  echo "      Either it crashed, hung, or its arg parser accepts anything." >&2
  exit 1
fi

echo "==> publishing atomically"
mv -f "$stage" "$dest"

echo "==> verifying the path nullclaw will resolve"
resolved="$HOME/.nullclaw/skills/$skill/bin/$skill"
[ -x "$resolved" ] || {
  echo "FAIL: $resolved is not executable — is the deploy.sh symlink in place?" >&2
  exit 1
}

echo "OK: $resolved"
echo "Activate by setting the SKILL.md '## Script' line to:"
echo "  ~/.nullclaw/skills/$skill/bin/$skill"
