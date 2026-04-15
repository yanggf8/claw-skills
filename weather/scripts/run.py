#!/usr/bin/env python3
"""Weather skill: fetch forecast for one or more locations (CWA for Taiwan, HKO for Hong Kong)."""
import argparse
import json
import os
import sys
import urllib.request
import urllib.parse
from datetime import datetime, timezone, timedelta

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
import telegram

HK_LOCATIONS = {"香港", "hong kong", "hk", "九龍", "新界", "港島"}


def load_env():
    env_path = os.environ.get("CLAW_ENV") or os.path.expanduser("~/.nullclaw/.env")
    if os.path.exists(env_path):
        with open(env_path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                key, _, val = line.partition("=")
                key = key.strip()
                val = val.strip().strip('"').strip("'")
                if key not in os.environ:
                    os.environ[key] = val


def is_hk_location(loc: str) -> bool:
    return loc.lower().strip() in HK_LOCATIONS


# ── HKO (Hong Kong Observatory) ─────────────────────────────────

def fetch_hko_forecast() -> dict:
    url = "https://data.weather.gov.hk/weatherAPI/opendata/weather.php?dataType=fnd&lang=tc"
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=20) as resp:
        return json.load(resp)


def format_hko(loc_name: str, data: dict) -> tuple[str, dict]:
    forecasts = data.get("weatherForecast", [])
    if not forecasts:
        return f"[WARN: HKO forecast unavailable for {loc_name}]", {}
    f = forecasts[0]
    wx = f.get("forecastWeather", "")
    min_t = f.get("forecastMintemp", {}).get("value", "?")
    max_t = f.get("forecastMaxtemp", {}).get("value", "?")
    psr = f.get("PSR", "")
    line = f"🌤 香港：{wx}，低溫{min_t}°C / 高溫{max_t}°C"
    if psr:
        line += f"，降雨概率{psr}"
    return line, {"location": "香港", "wx": wx, "min_t": str(min_t), "max_t": str(max_t), "pop": psr}


# ── CWA (Taiwan) ─────────────────────────────────────────────────

def fetch_cwa_weather(locations: list[str], api_key: str) -> dict:
    joined = ",".join(urllib.parse.quote(loc) for loc in locations)
    url = (
        f"https://opendata.cwa.gov.tw/api/v1/rest/datastore/F-C0032-001"
        f"?Authorization={api_key}&locationName={joined}"
    )
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=20) as resp:
        return json.load(resp)


def format_cwa_location(loc_name: str, loc_data: dict) -> tuple[str, dict]:
    elements = loc_data.get("weatherElement", [])
    by_name: dict[str, list] = {}
    for el in elements:
        by_name[el["elementName"]] = el.get("time", [])

    now = datetime.now(timezone(timedelta(hours=8)))
    best_idx = 0
    best_delta = None
    for i, tv in enumerate(by_name.get("Wx", [])):
        start_str = tv.get("startTime", "")
        try:
            dt = datetime.strptime(start_str, "%Y-%m-%d %H:%M:%S")
            dt = dt.replace(tzinfo=timezone(timedelta(hours=8)))
        except ValueError:
            continue
        delta = abs((dt - now).total_seconds())
        if best_delta is None or delta < best_delta:
            best_delta = delta
            best_idx = i

    def val_at(name: str) -> str:
        times = by_name.get(name, [])
        if best_idx < len(times):
            return times[best_idx].get("parameter", {}).get("parameterName", "")
        return ""

    wx = val_at("Wx")
    min_t = val_at("MinT")
    max_t = val_at("MaxT")
    pop = val_at("PoP")

    line = f"🌤 {loc_name}：{wx}，低溫{min_t}°C / 高溫{max_t}°C"
    if pop:
        line += f"，降雨機率{pop}%"
    return line, {"location": loc_name, "wx": wx, "min_t": min_t, "max_t": max_t, "pop": pop}


# ── Clothing advice ──────────────────────────────────────────────

def clothing_advice_llm(weather_data: list[dict]) -> str:
    import subprocess

    summary = "; ".join(
        f"{d['location']}: {d['wx']}, {d['min_t']}–{d['max_t']}°C, 降雨{d['pop']}%"
        for d in weather_data
    )
    prompt = (
        f"根據以下天氣資料，用繁體中文給出簡短的穿搭建議（1-2句話），"
        f"包含具體衣物建議和是否需要雨具。只回覆建議本身，不要重複天氣資料。\n"
        f"天氣：{summary}"
    )
    try:
        result = subprocess.run(
            [os.path.expanduser("~/nullclaw/zig-out/bin/nullclaw"), "agent", "-m", prompt],
            capture_output=True, text=True, timeout=30,
        )
        advice = result.stdout.strip()
        if advice:
            return f"👔 {advice}"
    except Exception as e:
        print(f"[WARN] LLM clothing advice failed: {e}", file=sys.stderr)
    return ""


# ── Main ─────────────────────────────────────────────────────────

def main():
    load_env()

    parser = argparse.ArgumentParser(description="Fetch weather forecast")
    parser.add_argument("--location", action="append", default=None, dest="locations",
                        metavar="LOCATION", help="Location name (repeatable)")
    parser.add_argument("--deliver-to", dest="deliver_to", default=None, metavar="CHAT_ID",
                        help="Telegram chat ID to deliver output to directly")
    parser.add_argument("--account", dest="account", default="main",
                        help="Telegram account name from config (default: main)")
    args = parser.parse_args()

    locations = args.locations or ["臺北市"]

    hk_locs = [loc for loc in locations if is_hk_location(loc)]
    tw_locs = [loc for loc in locations if not is_hk_location(loc)]

    lines = []
    weather_data = []

    # Hong Kong locations via HKO
    if hk_locs:
        try:
            hko_data = fetch_hko_forecast()
            for loc in hk_locs:
                line, data = format_hko(loc, hko_data)
                lines.append(line)
                if data:
                    weather_data.append(data)
        except Exception as e:
            for loc in hk_locs:
                lines.append(f"[WARN: HKO weather unavailable - {e}]")

    # Taiwan locations via CWA
    if tw_locs:
        api_key = os.environ.get("CWA_API_KEY", "")
        if not api_key:
            for loc in tw_locs:
                lines.append(f"[WARN: CWA weather unavailable - CWA_API_KEY not set]")
        else:
            try:
                cwa_data = fetch_cwa_weather(tw_locs, api_key)
                records = cwa_data.get("records", {}).get("location", [])
                loc_map = {r["locationName"]: r for r in records}
                for loc in tw_locs:
                    if loc in loc_map:
                        line, data = format_cwa_location(loc, loc_map[loc])
                        lines.append(line)
                        weather_data.append(data)
                    else:
                        lines.append(f"[WARN: weather unavailable - location '{loc}' not found]")
            except Exception as e:
                for loc in tw_locs:
                    lines.append(f"[WARN: CWA weather unavailable - {e}]")

    if not lines:
        lines.append("[WARN: no valid locations provided]")

    advice = clothing_advice_llm(weather_data) if weather_data else ""
    if advice:
        lines.append(advice)

    output = "\n".join(lines)
    job_id = os.environ.get("NULLCLAW_JOB_ID")
    if job_id:
        output += f"\n\n`{job_id}`"
    if args.deliver_to:
        telegram.send(args.deliver_to, output, account=args.account)
    else:
        print(output)


if __name__ == "__main__":
    main()
