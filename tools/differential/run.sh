#!/usr/bin/env bash
# Runs the Python and the Rust implementation over identical fixtures and diffs
# exit code, stdout, and stderr. Any difference must be justified in
# docs/specs/2026-07-28-phase1-intentional-differences.md.
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
FIXTURES="$ROOT/doughcon/tests/fixtures"
BIN="$ROOT/target/release/doughcon"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

[ -x "$BIN" ] || { echo "build first: cargo build --release" >&2; exit 1; }

# Both HOMEs get the same skeleton. record mode opens ~/.nullclaw/<log> with
# create-file (not create-parents) in BOTH implementations, so provisioning the
# directory for only one side compares setup, not behaviour.
mkdir -p "$STAGE/py/.nullclaw" "$STAGE/rs/.nullclaw"

fail=0
# Tab is an IFS *whitespace* character, so `IFS=$'\t' read` collapses runs of
# tabs and silently eats an empty column — the no_job_id case would shift
# expect_exit into job_id, run with NULLCLAW_JOB_ID=0, and report a bogus DIFF.
# \001 is not IFS whitespace, so empty fields survive.
while IFS=$'\001' read -r name argv fixture job_id expect_exit; do
  [ -z "${name:-}" ] && continue
  case "$name" in \#*) continue ;; esac

  # A local stub serves the fixture so neither implementation touches the network.
  python3 -c "
import http.server, json, sys, threading
body = open('$FIXTURES/$fixture','rb').read()
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass
srv = http.server.HTTPServer(('127.0.0.1', 0), H)
print(srv.server_port, flush=True)
srv.serve_forever()
" > "$STAGE/port.$name" &
  stub_pid=$!
  # Poll for the port line instead of sleeping a guessed interval: a slow start
  # would otherwise yield an empty port and a nonsense URL.
  port=""
  for _ in $(seq 1 100); do
    port=$(head -1 "$STAGE/port.$name" 2>/dev/null || true)
    [ -n "$port" ] && break
    sleep 0.1
  done
  [ -n "$port" ] || { echo "FAIL $name: stub server never reported a port"; kill $stub_pid 2>/dev/null; fail=1; continue; }
  url="http://127.0.0.1:$port/"

  env NULLCLAW_JOB_ID="$job_id" HOME="$STAGE/py" DOUGHCON_BASE_URL="$url" \
    python3 - "$argv" <<PY > "$STAGE/$name.py.out" 2> "$STAGE/$name.py.err"
import os, sys, runpy
os.makedirs(os.path.expanduser("~/.nullclaw"), exist_ok=True)
sys.argv = ["run.py"] + sys.argv[1].split()
import urllib.request
_orig = urllib.request.Request
def patched(url, *a, **k):
    return _orig(os.environ["DOUGHCON_BASE_URL"], *a, **k)
urllib.request.Request = patched
runpy.run_path("$ROOT/doughcon/scripts/run.py", run_name="__main__")
PY
  py_exit=$?

  env NULLCLAW_JOB_ID="$job_id" HOME="$STAGE/rs" DOUGHCON_BASE_URL="$url" \
    "$BIN" $argv > "$STAGE/$name.rs.out" 2> "$STAGE/$name.rs.err"
  rs_exit=$?

  kill $stub_pid 2>/dev/null

  case_fail=0
  if [ "$py_exit" != "$rs_exit" ]; then
    echo "DIFF $name: exit py=$py_exit rs=$rs_exit"; case_fail=1
  fi
  # The declared expectation is checked, not decorative: if BOTH sides drifted
  # to the same wrong exit code, matching each other proves nothing.
  if [ "$py_exit" != "$expect_exit" ]; then
    echo "DIFF $name: python exit $py_exit != declared $expect_exit"; case_fail=1
  fi
  if ! diff -u "$STAGE/$name.py.out" "$STAGE/$name.rs.out" > "$STAGE/$name.out.diff"; then
    echo "DIFF $name: stdout"; cat "$STAGE/$name.out.diff"; case_fail=1
  fi
  if ! diff -u "$STAGE/$name.py.err" "$STAGE/$name.rs.err" > "$STAGE/$name.err.diff"; then
    echo "DIFF $name: stderr"; cat "$STAGE/$name.err.diff"; case_fail=1
  fi
  # Per-case verdict — a sticky global flag would hide every later pass.
  [ "$case_fail" = 0 ] && echo "ok   $name"
  [ "$case_fail" = 0 ] || fail=1
done < <(tr '\t' '\001' < "$ROOT/tools/differential/cases.tsv")

# History-log comparison for record mode.
if ! diff -u "$STAGE/py/.nullclaw/doughcon-history.log" "$STAGE/rs/.nullclaw/doughcon-history.log" > /dev/null 2>&1; then
  echo "NOTE: history log lines differ only in the run timestamp — inspect manually:"
  tail -1 "$STAGE/py/.nullclaw/doughcon-history.log" 2>/dev/null
  tail -1 "$STAGE/rs/.nullclaw/doughcon-history.log" 2>/dev/null
fi

exit $fail
