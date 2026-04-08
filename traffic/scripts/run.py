#!/usr/bin/env python3
"""Traffic skill: fetch TomTom route travel time between waypoints."""
import argparse
import json
import os
import sys
import urllib.request

SKILLS_LIB = os.path.join(os.path.dirname(__file__), "..", "..", "lib")
sys.path.insert(0, os.path.abspath(SKILLS_LIB))
import telegram


def load_env():
    env_path = os.path.expanduser("~/.nullclaw/.env")
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


def load_locations() -> dict[str, str]:
    path = os.path.expanduser("~/.nullclaw/locations.json")
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    return {}


def resolve(name: str, locations: dict[str, str]) -> str:
    """Resolve a location name or raw lat,lon string to a coordinate string."""
    if name in locations:
        return locations[name]
    parts = name.split(",")
    if len(parts) == 2:
        try:
            float(parts[0].strip())
            float(parts[1].strip())
            return name
        except ValueError:
            pass
    raise ValueError(f"Unknown location: '{name}'. Add it to locations.json or use 'lat,lon'.")


def fetch_route(waypoints: list[str], api_key: str) -> int:
    """Return travel time in seconds."""
    coords = ":".join(waypoints)
    url = (
        f"https://api.tomtom.com/routing/1/calculateRoute/{coords}/json"
        f"?key={api_key}&traffic=true"
    )
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=20) as resp:
        data = json.load(resp)
    routes = data.get("routes", [])
    if not routes:
        raise ValueError("No routes returned by TomTom API")
    return routes[0]["summary"]["travelTimeInSeconds"]


def traffic_advice_llm(route_label: str, minutes: int) -> str:
    """Ask the nullclaw agent for contextual commute advice."""
    import subprocess

    prompt = (
        f"你是通勤助理。以下是即時路況資料：\n"
        f"路線：{route_label}\n"
        f"預估行車時間：{minutes} 分鐘\n\n"
        f"請用繁體中文給出 1-2 句簡短的通勤建議。根據行車時間判斷路況：\n"
        f"- 若時間短（<25分鐘）：路況順暢，可簡單提醒\n"
        f"- 若時間中等（25-40分鐘）：提醒注意壅塞路段\n"
        f"- 若時間長（>40分鐘）：建議替代路線或出發時間調整\n"
        f"只回覆建議本身，不要重複路況資料。"
    )
    try:
        result = subprocess.run(
            [os.path.expanduser("~/nullclaw/zig-out/bin/nullclaw"), "agent", "-m", prompt],
            capture_output=True, text=True, timeout=30,
        )
        advice = result.stdout.strip()
        if advice:
            return f"💡 {advice}"
    except Exception as e:
        print(f"[WARN] LLM traffic advice failed: {e}", file=sys.stderr)
    return ""


def main():
    load_env()
    api_key = os.environ.get("TOMTOM_API_KEY", "")
    if not api_key:
        print("[WARN: traffic unavailable - TOMTOM_API_KEY not set]")
        sys.exit(0)

    parser = argparse.ArgumentParser(description="Fetch TomTom route travel time")
    parser.add_argument("--from", dest="origin", required=True, metavar="LOCATION")
    parser.add_argument("--to", dest="dest", required=True, metavar="LOCATION")
    parser.add_argument("--via", dest="via", default=None, metavar="LOCATION")
    parser.add_argument("--deliver-to", dest="deliver_to", default=None, metavar="CHAT_ID",
                        help="Telegram chat ID to deliver output to directly")
    parser.add_argument("--account", default="main", help="Telegram bot account name")
    args = parser.parse_args()

    locations = load_locations()

    try:
        waypoints = [resolve(args.origin, locations)]
        if args.via:
            waypoints.append(resolve(args.via, locations))
        waypoints.append(resolve(args.dest, locations))
    except ValueError as e:
        print(f"[WARN: traffic unavailable - {e}]")
        sys.exit(0)

    if len(waypoints) < 2 or len(waypoints) > 3:
        print("[WARN: traffic unavailable - TomTom free tier supports 2-3 waypoints only]")
        sys.exit(0)

    try:
        secs = fetch_route(waypoints, api_key)
    except Exception as e:
        print(f"[WARN: traffic unavailable - {e}]")
        sys.exit(0)

    minutes = round(secs / 60)
    label_parts = [args.origin]
    if args.via:
        label_parts.append(args.via)
    label_parts.append(args.dest)
    label = "→".join(label_parts)
    base = f"🚗 {label}：{minutes}分鐘"

    advice = traffic_advice_llm(label, minutes)
    output = f"{base}\n{advice}" if advice else base

    job_id = os.environ.get("NULLCLAW_JOB_ID")
    if job_id:
        output += f"\n\n`{job_id}`"

    if args.deliver_to:
        telegram.send(args.deliver_to, output, account=args.account)
    else:
        print(output)


if __name__ == "__main__":
    main()
