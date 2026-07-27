#!/usr/bin/env python3
"""Weather skill: fetch forecast for one or more locations (CWA for Taiwan, HKO for Hong Kong)."""
import argparse
import json
import os
import sys
import time
import urllib.request
import urllib.parse
from datetime import datetime, timezone, timedelta

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
from delivery import deliver_or_fail
from skill_runner import strip_agent_artifacts
from trace_marker import emit_skill_status, emit_trace, emit_fallback

HK_LOCATIONS = {"香港", "hong kong", "hk", "九龍", "新界", "港島"}

# Taiwan county/city centroids for Open-Meteo fallback (lat, lon).
TW_COORDS: dict[str, tuple[float, float]] = {
    "臺北市": (25.0330, 121.5654), "台北市": (25.0330, 121.5654),
    "新北市": (25.0169, 121.4628),
    "桃園市": (24.9937, 121.3010),
    "臺中市": (24.1477, 120.6736), "台中市": (24.1477, 120.6736),
    "臺南市": (22.9999, 120.2270), "台南市": (22.9999, 120.2270),
    "高雄市": (22.6273, 120.3014),
    "基隆市": (25.1276, 121.7392),
    "新竹市": (24.8138, 120.9675),
    "新竹縣": (24.8387, 121.0177),
    "苗栗縣": (24.5602, 120.8214),
    "彰化縣": (24.0518, 120.5161),
    "南投縣": (23.9609, 120.9719),
    "雲林縣": (23.7092, 120.4313),
    "嘉義市": (23.4801, 120.4491),
    "嘉義縣": (23.4518, 120.2555),
    "屏東縣": (22.5519, 120.5487),
    "宜蘭縣": (24.7021, 121.7378),
    "花蓮縣": (23.9871, 121.6015),
    "臺東縣": (22.7583, 121.1444), "台東縣": (22.7583, 121.1444),
    "澎湖縣": (23.5712, 119.5793),
    "金門縣": (24.4321, 118.3171),
    "連江縣": (26.1608, 119.9286),
}

# WMO weather code → Traditional Chinese description (Open-Meteo).
WMO_TC: dict[int, str] = {
    0: "晴朗", 1: "大致晴朗", 2: "局部多雲", 3: "陰天",
    45: "霧", 48: "凍霧",
    51: "毛毛雨", 53: "毛毛雨", 55: "強毛毛雨",
    56: "凍毛毛雨", 57: "凍毛毛雨",
    61: "小雨", 63: "中雨", 65: "大雨",
    66: "凍雨", 67: "凍雨",
    71: "小雪", 73: "中雪", 75: "大雪",
    77: "雪粒",
    80: "短暫陣雨", 81: "陣雨", 82: "強陣雨",
    85: "短暫陣雪", 86: "陣雪",
    95: "雷雨", 96: "雷雨夾冰雹", 99: "強雷雨夾冰雹",
}


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
    with urllib.request.urlopen(req, timeout=8) as resp:
        return json.load(resp)


# ── Open-Meteo fallback ──────────────────────────────────────────

def fetch_open_meteo(lat: float, lon: float) -> dict:
    url = (
        "https://api.open-meteo.com/v1/forecast"
        f"?latitude={lat}&longitude={lon}"
        "&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max,weather_code"
        "&timezone=Asia%2FTaipei&forecast_days=1"
    )
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=8) as resp:
        return json.load(resp)


def format_open_meteo(loc_name: str, data: dict) -> tuple[str, dict]:
    daily = data.get("daily", {})
    codes = daily.get("weather_code", []) or []
    maxs = daily.get("temperature_2m_max", []) or []
    mins = daily.get("temperature_2m_min", []) or []
    pops = daily.get("precipitation_probability_max", []) or []
    if not codes or not maxs or not mins:
        return f"[WARN: Open-Meteo forecast unavailable for {loc_name}]", {}
    wx = WMO_TC.get(int(codes[0]), "")
    max_t = str(int(round(maxs[0])))
    min_t = str(int(round(mins[0])))
    pop = str(int(pops[0])) if pops else ""
    line = f"🌤 {loc_name}：{wx}，低溫{min_t}°C / 高溫{max_t}°C"
    if pop:
        line += f"，降雨機率{pop}%"
    line += "（備援）"
    return line, {"location": loc_name, "wx": wx, "min_t": min_t, "max_t": max_t, "pop": pop}


def open_meteo_for_locations(locations: list[str]) -> tuple[list[str], list[dict]]:
    lines: list[str] = []
    weather_data: list[dict] = []
    for loc in locations:
        coords = TW_COORDS.get(loc)
        if not coords:
            lines.append(f"[WARN: weather unavailable - no fallback coordinates for '{loc}']")
            continue
        try:
            data = fetch_open_meteo(*coords)
            line, wd = format_open_meteo(loc, data)
            lines.append(line)
            if wd:
                weather_data.append(wd)
        except Exception as e:
            lines.append(f"[WARN: Open-Meteo unavailable for {loc} - {e}]")
    return lines, weather_data


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
        f"包含具體衣物建議和是否需要雨具。只回覆建議本身，不要重複天氣資料。"
        f"只回覆純文字建議，勿附加 ncchoices、按鈕、選擇清單或任何標記。\n"
        f"天氣：{summary}"
    )
    try:
        result = subprocess.run(
            [os.path.expanduser("~/nullclaw/zig-out/bin/nullclaw"), "agent", "-m", prompt],
            capture_output=True, text=True, timeout=30,
        )
        advice = strip_agent_artifacts(result.stdout)
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

    # Taiwan locations via CWA, with Open-Meteo fallback on outage.
    fallback_used = False
    if tw_locs:
        api_key = os.environ.get("CWA_API_KEY", "")
        cwa_failed_reason: str | None = None
        cwa_unmatched: list[str] = []
        if not api_key:
            cwa_failed_reason = "CWA_API_KEY is not set in the environment"
        else:
            try:
                cwa_data = fetch_cwa_weather(tw_locs, api_key)
                records = cwa_data.get("records", {}).get("location", []) or []
                loc_map = {r["locationName"]: r for r in records}
                for loc in tw_locs:
                    if loc in loc_map:
                        line, data = format_cwa_location(loc, loc_map[loc])
                        lines.append(line)
                        weather_data.append(data)
                    else:
                        cwa_unmatched.append(loc)
                if not records and not loc_map:
                    cwa_failed_reason = "CWA returned an empty record list"
            except Exception as e:
                cwa_failed_reason = f"CWA request failed with {type(e).__name__}: {e}"

        if cwa_failed_reason:
            targets = tw_locs
        else:
            targets = cwa_unmatched
        if targets:
            t0 = time.monotonic()
            fb_lines, fb_data = open_meteo_for_locations(targets)
            elapsed_ms = int((time.monotonic() - t0) * 1000)
            lines.extend(fb_lines)
            weather_data.extend(fb_data)
            fallback_used = True
            reason = cwa_failed_reason or (
                f"CWA did not return data for {len(cwa_unmatched)} of {len(tw_locs)} locations"
            )
            scope = f"{len(targets)} Taiwan location" + ("" if len(targets) == 1 else "s")
            emit_fallback(
                skill="Weather",
                primary="CWA",
                fallback="Open-Meteo",
                reason=reason,
                scope=scope,
                elapsed_ms=elapsed_ms,
            )

    if not lines:
        lines.append("[WARN: no valid locations provided]")

    advice = clothing_advice_llm(weather_data) if weather_data else ""
    if advice:
        lines.append(advice)

    output = "\n".join(lines)
    job_id = os.environ.get("NULLCLAW_JOB_ID")
    if job_id:
        output += f"\n\n`{job_id}`"
    deliver_or_fail(args.deliver_to, output, account=args.account)
    if not weather_data:
        status = "failed"
    elif fallback_used:
        status = "degraded"
    else:
        status = "ok"
    emit_skill_status(status)
    emit_trace()


if __name__ == "__main__":
    main()
