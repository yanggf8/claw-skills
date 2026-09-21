#!/usr/bin/env bash
# Every HTTP call must go through claw_core::http::agent.
#
# ureq's own `timeout` — on the builder or the request — leaves the connect
# phase on a 30-second default, so a call site that builds its own agent looks
# bounded and is not. That regressed silently across twelve call sites once
# because nothing checked; a behavioural test can only cover the crate it lives
# in, so the property is checked here across the whole tree.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
CORE=$(cd "$ROOT/../../b/gwebcdb/crates/claw-core" 2>/dev/null && pwd || true)

pattern='ureq::(builder|AgentBuilder|get|post|put|delete|head|request)'
found=0

# Production code only. A test may reach for raw ureq deliberately — the stub
# servers do — and a comment naming the pattern is not a call site.
scan() {
  local dir=$1 label=$2
  [ -d "$dir" ] || return 0
  while IFS= read -r hit; do
    case "$hit" in
      *"claw-core/src/http.rs"*) continue ;;  # where the bounded agent is built
    esac
    # Strip the "path:line:" prefix before deciding whether the line is a
    # comment, so a doc comment describing the hazard does not read as one.
    code=${hit#*:*:}
    case "${code#"${code%%[![:space:]]*}"}" in
      "//"*) continue ;;
    esac
    echo "  $label ${hit#"$ROOT"/}"
    found=1
  done < <(find "$dir" -type d -name src -prune -exec grep -rEn "$pattern" {} --include='*.rs' \; 2>/dev/null || true)
}

scan "$ROOT/crates" "claw-skills"
[ -n "$CORE" ] && scan "$CORE" "claw-core"

if [ "$found" -ne 0 ]; then
  echo
  echo "FAIL: the sites above build their own ureq agent." >&2
  echo "      Use claw_core::http::agent(timeout) — see its module docs for why." >&2
  exit 1
fi
echo "OK: every HTTP call site goes through claw_core::http::agent"
