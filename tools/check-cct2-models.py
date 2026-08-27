#!/usr/bin/env python3
"""Durable验收 for cct2 models.jsonl — checks that the last business day has both modes with answered."""
import json, os, sys
from pathlib import Path

def main():
    home = Path(os.environ.get("HOME", "") or Path.cwd())
    models_path = home / ".nullclaw" / "skills" / "cct2" / "journal" / "models.jsonl"
    # Try to get trace for verification, but for shell jobs with exit_only we don't need it
    trace = os.environ.get("NULLCLAW_JOB_ID", "manual")
    # Check file exists
    if not models_path.exists():
        print(f"[WARN: models.jsonl missing] {models_path} not found")
        # For exit_only verification, non-zero exit triggers alert
        sys.exit(1)
    lines = models_path.read_text().strip().splitlines()
    if not lines:
        print("[WARN: models.jsonl empty]")
        sys.exit(1)
    entries = []
    for line in lines:
        try:
            entries.append(json.loads(line))
        except Exception:
            continue
    if not entries:
        print("[WARN: models.jsonl has no valid JSON]")
        sys.exit(1)
    latest_date = entries[-1].get("business_date", "")
    latest = [e for e in entries if e.get("business_date") == latest_date]
    modes = {e.get("mode") for e in latest}
    ok = True
    reasons = []
    for e in latest:
        primary = e.get("primary", {})
        if not primary.get("answered"):
            ok = False
            reasons.append(f"{e.get('mode')} primary not answered")
        if e.get("primary", {}).get("tickers", 0) < e.get("requested", 0):
            ok = False
            reasons.append(f"{e.get('mode')} primary tickers {e.get('primary',{}).get('tickers')} < requested {e.get('requested')}")
    if "pre-market" not in modes:
        ok = False
        reasons.append("pre-market missing for " + latest_date)
    if "eod" not in modes:
        ok = False
        reasons.append("eod missing for " + latest_date)
    if ok:
        print(f"✅ cct2 models.jsonl durable check ok for {latest_date}: {len(latest)} runs, both modes answered")
        sys.exit(0)
    else:
        print(f"⚠️ cct2 models.jsonl check degraded for {latest_date}: {'; '.join(reasons)}")
        sys.exit(1)

if __name__ == "__main__":
    main()
