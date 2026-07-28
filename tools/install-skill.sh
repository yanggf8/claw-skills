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
if ! "$stage" --mode record --et-hour 99 >/dev/null 2>&1; then
  rm -f "$stage"
  echo "FAIL: staged binary did not run cleanly (--et-hour 99 must be a no-op skip)" >&2
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
