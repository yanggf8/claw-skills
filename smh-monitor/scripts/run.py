#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any


DEFAULT_CONFIG = Path.home() / ".nullclaw" / "skills" / "smh-monitor" / "config.json"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="SMH tactical position monitor")
    parser.add_argument("--config", default=str(DEFAULT_CONFIG), help="Path to config.json")
    parser.add_argument("--current", type=float, help="Manual current price override")
    return parser.parse_args(argv)


def load_config(path: Path) -> dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as f:
            cfg = json.load(f)
    except FileNotFoundError:
        raise SystemExit(f"missing config: {path}")
    for key in ["ticker", "position_usd"]:
        if key not in cfg:
            raise SystemExit(f"config missing required key: {key}")
    return cfg


def read_cached_price(cfg: dict[str, Any]) -> tuple[float, str, str]:
    cmd = cfg.get("price_command", "/home/yanggf/b/gwebcdb/target/debug/price")
    ticker = str(cfg["ticker"])
    proc = subprocess.run(
        [cmd, "read", ticker],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout).strip()
        raise SystemExit(f"price read failed for {ticker}: {detail}")
    line = proc.stdout.strip().splitlines()[0]
    parts = line.split("\t")
    if len(parts) < 4:
        raise SystemExit(f"unexpected price output: {line}")
    return float(parts[2]), parts[1], parts[3]


def signal_for(change_pct: float) -> str:
    eps = 1e-9
    if change_pct <= -15.0 + eps:
        return "EXIT_BIAS"
    if change_pct <= -12.0 + eps:
        return "REDUCE_REVIEW"
    if change_pct <= -8.0 + eps:
        return "REVIEW"
    if change_pct + eps >= 35.0:
        return "TAKE_PROFIT_2"
    if change_pct + eps >= 25.0:
        return "TAKE_PROFIT_1"
    if change_pct + eps >= 15.0:
        return "RAISE_STOP"
    return "OK"


def money(value: float) -> str:
    return f"{value:,.2f}"


def render(cfg: dict[str, Any], current: float, price_date: str, source: str) -> str:
    ticker = str(cfg["ticker"])
    entry = float(cfg["entry_price"])
    position_usd = float(cfg["position_usd"])
    if entry <= 0 or current <= 0 or position_usd <= 0:
        raise SystemExit("entry_price, current price, and position_usd must be positive")
    change_pct = (current / entry - 1.0) * 100.0
    status = signal_for(change_pct)
    shares = position_usd / entry
    live_value = shares * current
    unrealized = live_value - position_usd
    events = cfg.get("manual_events") or [
        "NVDA / AVGO / AMD / MU guidance",
        "TSMC monthly revenue",
        "Hyperscaler capex guidance",
        "Export-control escalation",
        "SpaceX IPO / index-flow liquidity drain",
    ]
    lines = [
        f"📈 {ticker} 監控",
        "",
        f"狀態：{status}",
        f"現價：{money(current)} ({price_date}, {source})",
        f"進場：{money(entry)}",
        f"漲跌：{change_pct:+.1f}%",
        f"部位：約 USD {money(position_usd)} / {shares:.4f} 股",
        f"未實現：約 USD {unrealized:+,.2f}",
        "",
        "觸發線：",
        f"- Review：{money(entry * 0.92)} (-8%)",
        f"- Reduce review：{money(entry * 0.88)} (-12%)",
        f"- Exit bias：{money(entry * 0.85)} (-15%)",
        f"- Raise stop：{money(entry * 1.15)} (+15%)",
        f"- Take profit 1：{money(entry * 1.25)} (+25%)",
        f"- Take profit 2：{money(entry * 1.35)} (+35%)",
        "",
        "事件人工檢查：",
    ]
    lines.extend(f"- {event}" for event in events)
    lines.extend([
        "",
        "SIGNAL-ONLY：手動 review 後才可行動；不自動交易。",
        "[skill-status:ok]",
        f"[trace:{os.environ.get('NULLCLAW_JOB_ID', 'manual')}]",
    ])
    return "\n".join(lines) + "\n"


def render_disabled(cfg: dict[str, Any]) -> str:
    ticker = str(cfg.get("ticker", "SMH"))
    lines = [
        f"📈 {ticker} 監控",
        "",
        "狀態：NOT_ARMED",
        "SMH monitor 尚未啟用。請在成交後填入 runtime config 的 entry_price，並把 enabled 設為 true。",
        "",
        "SIGNAL-ONLY：手動 review 後才可行動；不自動交易。",
        "[skill-status:ok]",
        f"[trace:{os.environ.get('NULLCLAW_JOB_ID', 'manual')}]",
    ]
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    cfg = load_config(Path(args.config).expanduser())
    try:
        if not bool(cfg.get("enabled", False)):
            sys.stdout.write(render_disabled(cfg))
            return 0
        if "entry_price" not in cfg:
            raise SystemExit("config missing required key when enabled=true: entry_price")
        if args.current is not None:
            current, price_date, source = args.current, "manual", "manual"
        else:
            current, price_date, source = read_cached_price(cfg)
        sys.stdout.write(render(cfg, current, price_date, source))
        return 0
    except SystemExit as exc:
        print(f"SMH monitor failed: {exc}", file=sys.stderr)
        print("[skill-status:failed]")
        print(f"[trace:{os.environ.get('NULLCLAW_JOB_ID', 'manual')}]")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
