#!/usr/bin/env python3
"""INFLATION-CON — signal-only inflation confirmation monitor.

Judges whether inflation is *genuinely persistent* (not one hot print) from
FRED core-PCE / core-CPI / breakeven data, applies the written status ladder
(canonical copy lives in finance-engineering/risk-dashboard.md), and delivers
a plain-text evidence packet.

SIGNAL-ONLY. This classifies evidence; it NEVER prescribes a trade. It emits a
regime label (OK / WATCH / YELLOW / RED / INSUFFICIENT_DATA) and the numbers
behind it — never "buy gold", "un-gate IEF", a share/dollar target, or any
portfolio action. The human reads it, decides, records a `finance-cli decision
add`, and verifies at the broker.

The FOMC policy stance is a MANUAL config input
(restrictive | neutral | easing | unclear) — never machine-parsed from Fed
text (that would be false precision). It only tips the RED context clause.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))

# fred_fetch lives beside this script (skill-local transport adapter).
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fred_fetch  # noqa: E402

try:
    from delivery import deliver_or_fail
    from trace_marker import emit_skill_status, emit_trace
except Exception:  # manual run outside nullclaw — degrade gracefully
    def deliver_or_fail(deliver_to, output, account="main", parse_mode=None):
        print(output)
        return True

    def emit_skill_status(status):
        pass

    def emit_trace():
        pass


DEFAULT_CONFIG = Path.home() / ".nullclaw" / "skills" / "inflation-con" / "config.json"

# FRED series → role. Core PCE is the primary trigger; the rest are
# confirmation or context (see risk-dashboard.md "Inflation Confirmation
# Criterion").
DEFAULT_SERIES = {
    "core_pce": "PCEPILFE",   # primary
    "core_cpi": "CPILFESL",   # confirmation
    "headline_pce": "PCEPI",  # context
    "headline_cpi": "CPIAUCSL",  # context
    "breakeven_10y": "T10YIE",   # market expectations (daily)
    "real_yield_10y": "DFII10",  # context (daily)
    "nominal_10y": "DGS10",      # context (daily)
}

VALID_STANCES = {"restrictive", "neutral", "easing", "unclear"}


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="INFLATION-CON — signal-only inflation confirmation monitor")
    p.add_argument("--mode", choices=["deliver", "record"], default="deliver")
    p.add_argument("--config", default=str(DEFAULT_CONFIG))
    p.add_argument("--deliver-to", default=None, metavar="CHAT_ID")
    p.add_argument("--account", default="main")
    return p.parse_args(argv)


def load_config(path: Path) -> dict:
    if not path.exists():
        return {"series": dict(DEFAULT_SERIES), "policy_stance": "unclear"}
    with path.open("r", encoding="utf-8") as f:
        cfg = json.load(f)
    series = dict(DEFAULT_SERIES)
    series.update(cfg.get("series", {}))
    cfg["series"] = series
    stance = str(cfg.get("policy_stance", "unclear")).strip().lower()
    cfg["policy_stance"] = stance if stance in VALID_STANCES else "unclear"
    return cfg


# ---- math -----------------------------------------------------------------

def annualized(rows: list[tuple[str, float]], n: int) -> float | None:
    """Compound-annualize the change over the last n monthly observations.

    ((now / n-months-ago) ** (12/n) - 1) * 100. Returns None if too short.
    """
    if len(rows) <= n:
        return None
    now = rows[-1][1]
    then = rows[-1 - n][1]
    if then <= 0:
        return None
    return ((now / then) ** (12.0 / n) - 1.0) * 100.0


def latest(rows: list[tuple[str, float]]) -> tuple[str, float] | None:
    return rows[-1] if rows else None


def rising_over(rows: list[tuple[str, float]], lookback: int) -> bool | None:
    """Is the latest value above the value `lookback` observations ago?"""
    if len(rows) <= lookback:
        return None
    return rows[-1][1] > rows[-1 - lookback][1]


# ---- classification --------------------------------------------------------

def classify(series_rows: dict[str, list[tuple[str, float]]], policy_stance: str) -> tuple[str, dict]:
    core_pce = series_rows.get("core_pce", [])
    core_cpi = series_rows.get("core_cpi", [])
    breakeven = series_rows.get("breakeven_10y", [])

    # Guard: need enough monthly core-PCE history, and a latest core-PCE/CPI.
    if len(core_pce) < 7 or not core_pce or not core_cpi:
        return "INSUFFICIENT_DATA", {
            "core_pce_obs": len(core_pce),
            "reasons": ["fewer than 7 monthly core-PCE observations or missing latest core-PCE/CPI"],
        }

    pce3 = annualized(core_pce, 3)
    pce6 = annualized(core_pce, 6)
    cpi3 = annualized(core_cpi, 3)
    cpi6 = annualized(core_cpi, 6)
    be_last = latest(breakeven)
    be_rising = rising_over(breakeven, 63)  # ~3 months of daily obs

    # Falling trend on core PCE: 3-mo below 6-mo pace (disinflation underway).
    pce_falling = pce3 is not None and pce6 is not None and pce3 < pce6

    def ge(v: float | None, t: float) -> bool:
        return v is not None and v >= t

    core_cpi_hot_3_or_6 = ge(cpi3, 3.0) or ge(cpi6, 3.0)
    breakeven_ge = be_last is not None and be_last[1] >= 2.5
    context_not_easing = (breakeven_ge or be_rising is True) and policy_stance != "easing"

    reasons: list[str] = []
    status = "OK"

    # RED: inflation-up regime confirmed.
    if (
        ge(pce3, 3.5) and ge(pce6, 3.5)
        and core_cpi_hot_3_or_6
        and context_not_easing
    ):
        status = "RED"
        reasons.append("core PCE 3-mo & 6-mo annualized both >= 3.5%")
        reasons.append("core CPI confirms (>= 3.0% on 3-mo or 6-mo)")
        if breakeven_ge:
            reasons.append(f"10Y breakeven {be_last[1]:.2f} >= 2.5%")
        elif be_rising is True:
            reasons.append("10Y breakeven rising over ~3 months")
        reasons.append(f"policy stance = {policy_stance} (not easing)")

    # YELLOW: persistent above-target (core PCE 3m & 6m >= 3.0% + core CPI hot).
    elif ge(pce3, 3.0) and ge(pce6, 3.0) and core_cpi_hot_3_or_6:
        status = "YELLOW"
        reasons.append("core PCE 3-mo & 6-mo annualized both >= 3.0%")
        reasons.append("core CPI also >= 3.0% (3-mo or 6-mo)")
        # Note the boundary: hot on levels but RED context clause not met.
        if ge(pce3, 3.5) and ge(pce6, 3.5):
            if not context_not_easing:
                # be_last is None when the breakeven fetch failed — fetch_all is
                # per-series tolerant and stores an empty list. Subscripting it
                # here used to raise TypeError, so a hot-inflation month with a
                # failed T10YIE fetch crashed the whole run.
                be_txt = (
                    f"breakeven {be_last[1]:.2f} < 2.5% and not clearly rising"
                    if be_last is not None
                    else "breakeven unavailable this run"
                )
                reasons.append(
                    "levels reach RED but context clause not met "
                    f"({be_txt}, or stance easing) — human resolves via policy_stance"
                )
            if not core_cpi_hot_3_or_6:
                reasons.append("core CPI not confirming yet")

    # WATCH: one hot print / mixed.
    elif ge(pce3, 2.5) or (core_cpi_hot_3_or_6 and not ge(pce3, 3.0)):
        status = "WATCH"
        if ge(pce3, 2.5):
            reasons.append("core PCE 3-mo annualized >= 2.5% but 6-mo not confirming")
        if core_cpi_hot_3_or_6:
            reasons.append("core CPI hot while core PCE not yet confirming")

    # OK: not confirmed, or trend falling.
    else:
        status = "OK"
        if pce_falling:
            reasons.append("core PCE 3-mo pace below 6-mo pace (disinflating)")
        else:
            reasons.append("core PCE 3-mo < 2.5% and 6-mo < 2.75%")

    details = {
        "core_pce_day": core_pce[-1][0],
        "pce3": pce3,
        "pce6": pce6,
        "cpi3": cpi3,
        "cpi6": cpi6,
        "breakeven": None if be_last is None else be_last[1],
        "breakeven_day": None if be_last is None else be_last[0],
        "breakeven_rising": be_rising,
        "policy_stance": policy_stance,
        "core_pce_obs": len(core_pce),
        "reasons": reasons,
    }
    return status, details


# ---- fetch -----------------------------------------------------------------

def fetch_all(series_map: dict[str, str]) -> tuple[dict[str, list[tuple[str, float]]], str | None]:
    warnings: list[str] = []
    out: dict[str, list[tuple[str, float]]] = {}
    for key, series_id in series_map.items():
        try:
            out[key] = fred_fetch.fetch_series(str(series_id))
            if not out[key]:
                warnings.append(f"{series_id}: no rows")
        except Exception as exc:  # noqa: BLE001
            warnings.append(f"fetch {series_id}: {exc}")
            out[key] = []
    if not out.get("core_pce"):
        raise RuntimeError("; ".join(warnings) or "FRED: no core PCE (PCEPILFE) — primary series")
    return out, "; ".join(warnings) if warnings else None


# ---- formatting ------------------------------------------------------------

def fmt_pct(v: float | None) -> str:
    if v is None:
        return "n/a"
    sign = "+" if v >= 0 else ""
    return f"{sign}{v:.2f}%"


def fmt_num(v: float | None) -> str:
    return "n/a" if v is None else f"{v:.2f}"


def format_message(status: str, details: dict, cfg: dict, warning: str | None = None) -> tuple[str, str]:
    lines = ["📈 INFLATION-CON"]
    skill_status = "ok"
    if warning:
        lines.append(f"[WARN: {warning}]")
        skill_status = "degraded"
    lines.append(f"狀態：{status}")

    if status == "INSUFFICIENT_DATA":
        lines.append(f"core PCE obs: {details.get('core_pce_obs', 0)} / 7 needed")
    else:
        rising = details.get("breakeven_rising")
        rising_str = "rising" if rising is True else "flat/down" if rising is False else "n/a"
        lines.extend([
            f"核心PCE ({details['core_pce_day']})：3mo {fmt_pct(details['pce3'])} / 6mo {fmt_pct(details['pce6'])} 年化",
            f"核心CPI：3mo {fmt_pct(details['cpi3'])} / 6mo {fmt_pct(details['cpi6'])} 年化",
            f"10Y breakeven：{fmt_num(details['breakeven'])} ({details['breakeven_day']}, {rising_str})",
            f"FOMC 立場 (manual)：{details['policy_stance']}",
        ])
        if details["reasons"]:
            lines.append("依據：")
            lines.extend(f"- {r}" for r in details["reasons"])

    lines.append("")
    lines.append("人工檢查（不入演算法）：")
    lines.append("- 最新 CPI (~月中) / PCE (~月底) release 是否已納入")
    lines.append("- FOMC 立場（restrictive / neutral / easing / unclear）是否需更新 config")
    lines.append("- 能源價格是否推高 headline 但 core 未跟進（headline 只是 context）")
    lines.append("")
    lines.append("SIGNAL-ONLY：這是通膨『確認證據』分級，不是交易指令。")
    lines.append("RED = 進入 review（是否加通膨對沖？IEF gate？），由人決定並記 decision add。")
    return "\n".join(lines), skill_status


def record_line(status: str, details: dict, warning: str | None) -> str:
    now = datetime.now(timezone(timedelta(hours=8))).strftime("%Y-%m-%d %H:%M:%S CST")
    if status == "INSUFFICIENT_DATA":
        return f"{now} INFLATION-CON {status} obs={details.get('core_pce_obs', 0)} warning={warning or '-'}"
    return (
        f"{now} INFLATION-CON {status} "
        f"pce3={fmt_pct(details['pce3'])} pce6={fmt_pct(details['pce6'])} "
        f"cpi3={fmt_pct(details['cpi3'])} cpi6={fmt_pct(details['cpi6'])} "
        f"be={fmt_num(details['breakeven'])} stance={details['policy_stance']} "
        f"warning={warning or '-'}"
    )


def emit(message: str, status: str, args) -> None:
    # Plain text (parse_mode=None): status names carry underscores
    # (INSUFFICIENT_DATA) and a FRED WARN can carry arbitrary text — under
    # Telegram legacy Markdown these break entity parsing. Nothing here is
    # intentional Markdown. (Mirrors chipcon.)
    job_id = os.environ.get("NULLCLAW_JOB_ID")
    output = message
    if job_id:
        output += f"\n\n{job_id}"
    deliver_or_fail(args.deliver_to, output, account=args.account, parse_mode=None)
    emit_skill_status(status)
    emit_trace()


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    cfg = load_config(Path(args.config).expanduser())
    try:
        series_rows, warning = fetch_all(cfg["series"])
        status, details = classify(series_rows, cfg["policy_stance"])
        if args.mode == "record":
            path = Path("~/.nullclaw/inflation-con-history.log").expanduser()
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("a", encoding="utf-8") as f:
                f.write(record_line(status, details, warning) + "\n")
            emit_skill_status("degraded" if warning else "ok")
            emit_trace()
            return 0
        message, skill_status = format_message(status, details, cfg, warning)
        emit(message, skill_status, args)
        return 0
    except Exception as exc:  # noqa: BLE001
        print(f"INFLATION-CON failed: {exc}", file=sys.stderr)
        emit_skill_status("failed")
        emit_trace()
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
