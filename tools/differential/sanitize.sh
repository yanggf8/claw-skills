#!/usr/bin/env bash
# Sanitizer corpus differential: Python lib/skill_runner.strip_agent_artifacts
# vs Rust claw_core::sanitize::strip_agent_artifacts, byte-for-byte.
#
# For each tools/differential/sanitize_corpus/*.txt, runs both sides under
# collapse_blank_lines=true and =false. Any byte difference is a FINDING.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CORPUS="$ROOT/tools/differential/sanitize_corpus"
LIB="$ROOT/lib"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

if [ ! -d "$CORPUS" ]; then
  echo "missing corpus dir: $CORPUS" >&2
  exit 2
fi

# Build the example once so per-file runs do not recompile.
echo "building claw-core example sanitize_stdin..."
if ! cargo build -q -p claw-core --example sanitize_stdin --manifest-path "$ROOT/Cargo.toml"; then
  echo "FAIL: cargo build --example sanitize_stdin" >&2
  exit 2
fi
RS_BIN="$ROOT/target/debug/examples/sanitize_stdin"
if [ ! -x "$RS_BIN" ]; then
  # workspace target layout may place examples at target/debug/examples/
  RS_BIN=$(find "$ROOT/target" -path '*/examples/sanitize_stdin' -type f -perm -111 2>/dev/null | head -1)
fi
if [ ! -x "$RS_BIN" ]; then
  echo "FAIL: sanitize_stdin binary not found after build" >&2
  exit 2
fi

# Python oracle: import frozen lib/skill_runner.py (not a copy).
py_sanitize() {
  local collapse="$1"
  local infile="$2"
  local outfile="$3"
  # collapse is shell "true"/"false"; map to Python bool without building invalid syntax.
  python3 -c "
import sys
sys.path.insert(0, '''$LIB''')
from skill_runner import strip_agent_artifacts
raw = sys.stdin.buffer.read().decode('utf-8')
collapse = sys.argv[1] == 'true'
out = strip_agent_artifacts(raw, collapse_blank_lines=collapse)
sys.stdout.buffer.write(out.encode('utf-8'))
" "$collapse" < "$infile" > "$outfile"
}

rs_sanitize() {
  local collapse="$1"
  local infile="$2"
  local outfile="$3"
  "$RS_BIN" "$collapse" < "$infile" > "$outfile"
}

fail=0
pass=0
total=0
declare -a findings=()

shopt -s nullglob
files=("$CORPUS"/*.txt)
if [ ${#files[@]} -eq 0 ]; then
  echo "FAIL: no corpus files in $CORPUS" >&2
  exit 2
fi

echo "corpus: ${#files[@]} files × 2 collapse modes"
echo "python oracle: $LIB/skill_runner.py"
echo "rust binary:   $RS_BIN"
echo

for f in "${files[@]}"; do
  base=$(basename "$f")
  for collapse in true false; do
    total=$((total + 1))
    label="${base} collapse=${collapse}"
    py_out="$STAGE/${base}.${collapse}.py.out"
    rs_out="$STAGE/${base}.${collapse}.rs.out"

    if ! py_sanitize "$collapse" "$f" "$py_out"; then
      echo "FAIL $label: python exited non-zero"
      fail=$((fail + 1))
      findings+=("$label: python error")
      continue
    fi
    if ! rs_sanitize "$collapse" "$f" "$rs_out"; then
      echo "FAIL $label: rust exited non-zero"
      fail=$((fail + 1))
      findings+=("$label: rust error")
      continue
    fi

    # Byte-for-byte: cmp (trailing-newline diffs are real findings).
    if cmp -s "$py_out" "$rs_out"; then
      echo "ok   $label"
      pass=$((pass + 1))
    else
      echo "DIFF $label"
      fail=$((fail + 1))
      findings+=("$label")
      # Preserve outputs for the report; also dump hexdump + text view.
      diff_dir="$STAGE/diff_${base}_${collapse}"
      mkdir -p "$diff_dir"
      cp "$f" "$diff_dir/input.txt"
      cp "$py_out" "$diff_dir/python.out"
      cp "$rs_out" "$diff_dir/rust.out"
      {
        echo "=== input (cat -A) ==="
        cat -A "$f" || true
        echo
        echo "=== python out (cat -A) ==="
        cat -A "$py_out" || true
        echo
        echo "=== rust out (cat -A) ==="
        cat -A "$rs_out" || true
        echo
        echo "=== hexdump -C input ==="
        hexdump -C "$f" | head -40
        echo "=== hexdump -C python ==="
        hexdump -C "$py_out" | head -40
        echo "=== hexdump -C rust ==="
        hexdump -C "$rs_out" | head -40
        echo "=== diff -u python rust ==="
        diff -u "$py_out" "$rs_out" || true
      } > "$diff_dir/report.txt"
      cat "$diff_dir/report.txt"
      # Keep a copy under /tmp for post-run inspection.
      keep="/tmp/sanitize-diff-${base}-${collapse}"
      rm -rf "$keep"
      cp -a "$diff_dir" "$keep"
      echo "(saved $keep)"
    fi
  done
done

echo
echo "======== SUMMARY ========"
echo "corpus files: ${#files[@]}"
echo "comparisons:  $total  (files × 2 collapse modes)"
echo "pass:         $pass"
echo "fail:         $fail"
if [ "$fail" -ne 0 ]; then
  echo "findings:"
  for fnd in "${findings[@]}"; do
    echo "  - $fnd"
  done
  exit 1
fi
exit 0
