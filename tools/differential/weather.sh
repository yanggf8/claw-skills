#!/usr/bin/env bash
# End-to-end differential: Python weather/scripts/run.py vs target/release/weather.
# Three HTTP stubs (HKO / CWA / Open-Meteo), fake agent via HOME, two masks only.
# Any residual diff is a FINDING — do not edit fixtures to hide one.
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
FIXTURES="$ROOT/tools/differential/fixtures"
CASES="$ROOT/tools/differential/weather_cases.tsv"
BIN="$ROOT/target/release/weather"
PY_RUN="$ROOT/weather/scripts/run.py"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"; kill $(jobs -p) 2>/dev/null' EXIT

FIXED_ADVICE="DIFF_HARNESS_FIXED_ADVICE"
EXPECTED_FIELDS=8

[ -x "$BIN" ] || { echo "build first: cargo build --release -p weather" >&2; exit 1; }
[ -f "$PY_RUN" ] || { echo "missing $PY_RUN" >&2; exit 1; }

# Shared HOME for both sides: agent plant + isolated ~/.nullclaw (no real .env).
mkdir -p "$STAGE/nullclaw/zig-out/bin" "$STAGE/.nullclaw"
cat > "$STAGE/nullclaw/zig-out/bin/nullclaw" <<EOF
#!/bin/sh
# Fake agent: always print the fixed advice string the harness asserts on.
echo "$FIXED_ADVICE"
EOF
chmod +x "$STAGE/nullclaw/zig-out/bin/nullclaw"

# ── masks (exactly two classes; do not add a third) ──────────────
mask() {
  # 1) Open-Meteo wall-time jitter (same quantity, different digits)
  # 2) exception text inside CWA-failed-reason / Open-Meteo WARN tails
  sed -E \
    -e 's/and took [0-9]+ms/and took <MS>ms/g' \
    -e 's/CWA request failed with [A-Za-z_][A-Za-z0-9_]*: .*/CWA request failed with <EXC>/g' \
    -e 's/(\[WARN: Open-Meteo unavailable for [^ -][^]]* - ).*/\1<EXC>]/g'
}

# ── stub: one local HTTP server; mode is FAIL or a fixture path ──
start_stub() {
  local mode=$1
  local portfile=$2
  local body_path=""
  if [ "$mode" != "FAIL" ]; then
    body_path="$FIXTURES/$mode"
    [ -f "$body_path" ] || { echo "missing fixture: $body_path" >&2; return 1; }
  fi
  python3 -c "
import http.server, sys
mode = sys.argv[1]
body_path = sys.argv[2] if len(sys.argv) > 2 else ''
body = b''
if mode != 'FAIL':
    body = open(body_path, 'rb').read()

class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if mode == 'FAIL':
            self.send_response(500)
            self.send_header('Content-Length', '4')
            self.end_headers()
            self.wfile.write(b'fail')
            return
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a):
        pass

srv = http.server.HTTPServer(('127.0.0.1', 0), H)
print(srv.server_port, flush=True)
srv.serve_forever()
" "$mode" "$body_path" > "$portfile" &
  echo $!
}

wait_port() {
  local portfile=$1
  local port=""
  for _ in $(seq 1 100); do
    port=$(head -1 "$portfile" 2>/dev/null || true)
    [ -n "$port" ] && break
    sleep 0.05
  done
  [ -n "$port" ] || return 1
  echo "$port"
}

# Common env for both implementations: isolated HOME, no host CLAW_* bleed.
# CWA_API_KEY is set per-case (may be empty).
base_env() {
  # shellcheck disable=SC2086
  env -u CLAW_ENV -u CLAW_CONFIG \
    HOME="$STAGE" \
    HKO_BASE_URL="$1" \
    CWA_BASE_URL="$2" \
    OPEN_METEO_BASE_URL="$3" \
    CWA_API_KEY="$4" \
    NULLCLAW_JOB_ID="$5" \
    "${@:6}"
}

run_python() {
  local argv=$1 hko_url=$2 cwa_url=$3 om_url=$4 api_key=$5 job_id=$6 out=$7 err=$8
  base_env "$hko_url" "$cwa_url" "$om_url" "$api_key" "$job_id" \
    python3 - "$argv" <<'PY' > "$out" 2> "$err"
import os, sys, runpy
import urllib.request

# Route by URL substring → three local stubs (not one catch-all redirect).
_orig = urllib.request.Request
def patched(url, *a, **k):
    u = url if isinstance(url, str) else str(url)
    if "hko" in u or "weather.gov.hk" in u:
        return _orig(os.environ["HKO_BASE_URL"], *a, **k)
    if "opendata.cwa" in u:
        return _orig(os.environ["CWA_BASE_URL"], *a, **k)
    if "open-meteo" in u:
        return _orig(os.environ["OPEN_METEO_BASE_URL"], *a, **k)
    return _orig(url, *a, **k)
urllib.request.Request = patched

sys.argv = ["run.py"] + (sys.argv[1].split() if sys.argv[1] else [])
runpy.run_path(
    os.path.join(os.path.dirname(__file__) if False else "",  # placate linters
                 ),
    run_name="__main__",
)
PY
  # The heredoc above cannot interpolate ROOT; re-invoke properly:
}

# Python runner — separate function so ROOT is in scope via env.
run_python_real() {
  local argv=$1 hko_url=$2 cwa_url=$3 om_url=$4 api_key=$5 job_id=$6 out=$7 err=$8
  base_env "$hko_url" "$cwa_url" "$om_url" "$api_key" "$job_id" \
    WEATHER_RUN="$PY_RUN" \
    python3 - "$argv" <<'PY' > "$out" 2> "$err"
import os, sys, runpy
import urllib.request

_orig = urllib.request.Request
def patched(url, *a, **k):
    u = url if isinstance(url, str) else str(url)
    if "hko" in u or "weather.gov.hk" in u:
        return _orig(os.environ["HKO_BASE_URL"], *a, **k)
    if "opendata.cwa" in u:
        return _orig(os.environ["CWA_BASE_URL"], *a, **k)
    if "open-meteo" in u:
        return _orig(os.environ["OPEN_METEO_BASE_URL"], *a, **k)
    return _orig(url, *a, **k)
urllib.request.Request = patched

sys.argv = ["run.py"] + (sys.argv[1].split() if len(sys.argv) > 1 and sys.argv[1] else [])
runpy.run_path(os.environ["WEATHER_RUN"], run_name="__main__")
PY
}

run_rust() {
  local argv=$1 hko_url=$2 cwa_url=$3 om_url=$4 api_key=$5 job_id=$6 out=$7 err=$8
  # Intentionally unquoted $argv — same word-split as the Python side.
  # shellcheck disable=SC2086
  base_env "$hko_url" "$cwa_url" "$om_url" "$api_key" "$job_id" \
    "$BIN" $argv > "$out" 2> "$err"
}

# ═════════════════════════════════════════════════════════════════
# STEP ZERO — seam verification. If the fixed advice string is not
# in BOTH outputs, every later "pass" is meaningless. Fail and stop.
# ═════════════════════════════════════════════════════════════════
echo "=== seam verification (fake agent via HOME) ==="
hko_pid=$(start_stub hko_ok.json "$STAGE/port.seam.hko")
cwa_pid=$(start_stub empty.json "$STAGE/port.seam.cwa")
om_pid=$(start_stub empty.json "$STAGE/port.seam.om")
hko_port=$(wait_port "$STAGE/port.seam.hko") || { echo "SEAM FAIL: HKO stub never reported a port"; exit 1; }
cwa_port=$(wait_port "$STAGE/port.seam.cwa") || { echo "SEAM FAIL: CWA stub never reported a port"; exit 1; }
om_port=$(wait_port "$STAGE/port.seam.om") || { echo "SEAM FAIL: OM stub never reported a port"; exit 1; }
hko_url="http://127.0.0.1:${hko_port}/"
cwa_url="http://127.0.0.1:${cwa_port}/"
om_url="http://127.0.0.1:${om_port}/"

run_python_real "--location 香港" "$hko_url" "$cwa_url" "$om_url" "testkey" "t-seam" \
  "$STAGE/seam.py.out" "$STAGE/seam.py.err"
py_seam_exit=$?
run_rust "--location 香港" "$hko_url" "$cwa_url" "$om_url" "testkey" "t-seam" \
  "$STAGE/seam.rs.out" "$STAGE/seam.rs.err"
rs_seam_exit=$?
kill "$hko_pid" "$cwa_pid" "$om_pid" 2>/dev/null
wait 2>/dev/null || true

py_has=0; rs_has=0
grep -qF "$FIXED_ADVICE" "$STAGE/seam.py.out" && py_has=1
grep -qF "$FIXED_ADVICE" "$STAGE/seam.rs.out" && rs_has=1

if [ "$py_has" != 1 ] || [ "$rs_has" != 1 ]; then
  echo "SEAM BROKEN: fixed advice string missing from one or both sides."
  echo "  python has_fixed=$py_has exit=$py_seam_exit"
  echo "  rust   has_fixed=$rs_has exit=$rs_seam_exit"
  echo "  --- python stdout ---"
  cat "$STAGE/seam.py.out"
  echo "  --- rust stdout ---"
  cat "$STAGE/seam.rs.out"
  echo "  --- python stderr ---"
  cat "$STAGE/seam.py.err"
  echo "  --- rust stderr ---"
  cat "$STAGE/seam.rs.err"
  echo "Every subsequent differential pass would be meaningless. Stopping."
  exit 1
fi
echo "ok   seam: both sides used the HOME-planted fake agent ($FIXED_ADVICE)"
echo

# ═════════════════════════════════════════════════════════════════
# Case loop
# ═════════════════════════════════════════════════════════════════
fail=0
while IFS=$'\001' read -r name argv hko_mode cwa_mode om_mode api_key job_id expect_exit; do
  [ -z "${name:-}" ] && continue
  case "$name" in \#*) continue ;; esac

  # Assert field count — empty TSV columns must not shift later fields.
  # (read with \001 delimiter already; recount via the original line.)
  raw_line=$(grep -E "^${name}"$'\t' "$CASES" | head -1)
  # Count fields: tr tabs to newlines and count non-... actually include empties.
  field_count=$(printf '%s' "$raw_line" | awk -F'\t' '{print NF}')
  if [ "$field_count" != "$EXPECTED_FIELDS" ]; then
    echo "FAIL $name: TSV field count $field_count != $EXPECTED_FIELDS (line: $raw_line)"
    fail=1
    continue
  fi

  hko_pid=$(start_stub "$hko_mode" "$STAGE/port.$name.hko")
  cwa_pid=$(start_stub "$cwa_mode" "$STAGE/port.$name.cwa")
  om_pid=$(start_stub "$om_mode" "$STAGE/port.$name.om")

  hko_port=$(wait_port "$STAGE/port.$name.hko") || {
    echo "FAIL $name: HKO stub never reported a port"; kill $hko_pid $cwa_pid $om_pid 2>/dev/null; fail=1; continue
  }
  cwa_port=$(wait_port "$STAGE/port.$name.cwa") || {
    echo "FAIL $name: CWA stub never reported a port"; kill $hko_pid $cwa_pid $om_pid 2>/dev/null; fail=1; continue
  }
  om_port=$(wait_port "$STAGE/port.$name.om") || {
    echo "FAIL $name: OM stub never reported a port"; kill $hko_pid $cwa_pid $om_pid 2>/dev/null; fail=1; continue
  }

  hko_url="http://127.0.0.1:${hko_port}/"
  cwa_url="http://127.0.0.1:${cwa_port}/"
  om_url="http://127.0.0.1:${om_port}/"

  run_python_real "$argv" "$hko_url" "$cwa_url" "$om_url" "$api_key" "$job_id" \
    "$STAGE/$name.py.out" "$STAGE/$name.py.err"
  py_exit=$?
  run_rust "$argv" "$hko_url" "$cwa_url" "$om_url" "$api_key" "$job_id" \
    "$STAGE/$name.rs.out" "$STAGE/$name.rs.err"
  rs_exit=$?

  kill $hko_pid $cwa_pid $om_pid 2>/dev/null
  wait 2>/dev/null || true

  # Apply the two masks before diffing.
  mask < "$STAGE/$name.py.out" > "$STAGE/$name.py.out.m"
  mask < "$STAGE/$name.rs.out" > "$STAGE/$name.rs.out.m"
  mask < "$STAGE/$name.py.err" > "$STAGE/$name.py.err.m"
  mask < "$STAGE/$name.rs.err" > "$STAGE/$name.rs.err.m"

  case_fail=0
  if [ "$py_exit" != "$rs_exit" ]; then
    echo "DIFF $name: exit py=$py_exit rs=$rs_exit"
    case_fail=1
  fi
  # Declared expectation is checked, not decorative.
  if [ "$py_exit" != "$expect_exit" ]; then
    echo "DIFF $name: python exit $py_exit != declared $expect_exit"
    case_fail=1
  fi
  if [ "$rs_exit" != "$expect_exit" ]; then
    echo "DIFF $name: rust exit $rs_exit != declared $expect_exit"
    case_fail=1
  fi
  if ! diff -u "$STAGE/$name.py.out.m" "$STAGE/$name.rs.out.m" > "$STAGE/$name.out.diff"; then
    echo "DIFF $name: stdout"
    cat "$STAGE/$name.out.diff"
    case_fail=1
  fi
  if ! diff -u "$STAGE/$name.py.err.m" "$STAGE/$name.rs.err.m" > "$STAGE/$name.err.diff"; then
    echo "DIFF $name: stderr"
    cat "$STAGE/$name.err.diff"
    case_fail=1
  fi

  if [ "$case_fail" = 0 ]; then
    echo "ok   $name"
  else
    echo "---- $name inputs: argv=[$argv] hko=$hko_mode cwa=$cwa_mode om=$om_mode api_key=[$api_key] job_id=[$job_id] expect_exit=$expect_exit"
    echo "---- $name unmasked python stdout ---"
    cat "$STAGE/$name.py.out"
    echo "---- $name unmasked rust stdout ---"
    cat "$STAGE/$name.rs.out"
    echo "---- $name unmasked python stderr ---"
    cat "$STAGE/$name.py.err"
    echo "---- $name unmasked rust stderr ---"
    cat "$STAGE/$name.rs.err"
    fail=1
  fi
done < <(tr '\t' '\001' < "$CASES")

exit $fail
