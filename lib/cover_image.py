"""Cover image generation and dev.to upload.

Uses ZhipuAI CogView-4 for generation, removes watermark, and can
update a dev.to article's main_image via the API.

Requires: BIGMODEL_API_KEY env var for generation.
Optional: Pillow for watermark removal (graceful degradation without it).
"""

import json
import os
import sys
import urllib.request
import urllib.error
from pathlib import Path
from typing import Optional


COGVIEW_URL = "https://open.bigmodel.cn/api/paas/v4/images/generations"
COGVIEW_MODEL = "cogview-4"
DEFAULT_SIZE = "1440x720"


def generate(prompt: str, *, size: str = DEFAULT_SIZE) -> str:
    """Generate an image via CogView-4. Returns the temporary image URL.

    Raises RuntimeError on API errors.
    """
    api_key = os.environ.get("BIGMODEL_API_KEY")
    if not api_key:
        raise RuntimeError("BIGMODEL_API_KEY env var is required")

    headers = {
        "Authorization": f"Bearer {api_key}",
        "Content-Type": "application/json",
    }
    payload = {
        "model": COGVIEW_MODEL,
        "prompt": prompt,
        "size": size,
    }
    data = json.dumps(payload).encode()
    req = urllib.request.Request(COGVIEW_URL, data=data, headers=headers)
    with urllib.request.urlopen(req, timeout=120) as resp:
        result = json.loads(resp.read())

    try:
        return result["data"][0]["url"]
    except (KeyError, IndexError) as e:
        raise RuntimeError(f"unexpected API response: {result}") from e


def download(url: str, dest: str) -> int:
    """Download a URL to a local file. Returns byte count."""
    req = urllib.request.Request(url, headers={"User-Agent": "claw-skills/0.1"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        data = resp.read()
    Path(dest).parent.mkdir(parents=True, exist_ok=True)
    Path(dest).write_bytes(data)
    return len(data)


def remove_watermark(src: str, dest: str) -> None:
    """Remove CogView 'AI生成' watermark from bottom-right corner.

    Requires Pillow. If not installed, copies the file as-is with a warning.
    """
    try:
        from PIL import Image, ImageDraw
    except ImportError:
        print("[WARN] Pillow not installed; watermark not removed", file=sys.stderr)
        if src != dest:
            Path(dest).write_bytes(Path(src).read_bytes())
        return

    img = Image.open(src)
    w, h = img.size
    draw = ImageDraw.Draw(img)
    # CogView watermark sits in the bottom-right ~120x45 pixels
    draw.rectangle([w - 120, h - 45, w, h], fill=(10, 20, 35))
    # Trim 5px from bottom for clean edge
    cropped = img.crop((0, 0, w, h - 5))
    cropped.save(dest)


def update_devto_cover(
    api_key: str, devto_id: int, image_url: str,
) -> dict:
    """PUT main_image to a dev.to article. Returns the API response dict."""
    url = f"https://dev.to/api/articles/{devto_id}"
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "claw-skills/0.1",
        "api-key": api_key,
    }
    data = json.dumps({"article": {"main_image": image_url}}).encode()
    req = urllib.request.Request(url, data=data, headers=headers, method="PUT")
    with urllib.request.urlopen(req, timeout=20) as resp:
        return json.loads(resp.read())


def _cli():
    import argparse

    parser = argparse.ArgumentParser(
        prog="cover_image",
        description="Generate cover images and update dev.to articles",
    )
    sub = parser.add_subparsers(dest="command")

    # generate
    p_gen = sub.add_parser("generate", help="Generate an image from a text prompt")
    p_gen.add_argument("prompt", help="Image generation prompt")
    p_gen.add_argument("-o", "--output", required=True, help="Output file path")
    p_gen.add_argument("--size", default=DEFAULT_SIZE, help=f"Image size (default {DEFAULT_SIZE})")
    p_gen.add_argument("--keep-watermark", action="store_true", help="Skip watermark removal")

    # update-devto
    p_upd = sub.add_parser("update-devto", help="Set cover image on a dev.to article")
    p_upd.add_argument("devto_id", type=int, help="dev.to article ID")
    p_upd.add_argument("image_url", help="Public URL of the cover image")
    p_upd.add_argument("--persona", default=None, help="Persona slug for API key lookup (default: use DEV_TO_API_KEY env)")

    args = parser.parse_args()

    if args.command == "generate":
        print(f"Generating image ({args.size})...")
        temp_url = generate(args.prompt, size=args.size)
        print(f"Downloading...")
        raw_path = args.output + ".raw.png" if not args.keep_watermark else args.output
        nbytes = download(temp_url, raw_path)
        print(f"Downloaded: {nbytes} bytes")

        if not args.keep_watermark:
            remove_watermark(raw_path, args.output)
            Path(raw_path).unlink(missing_ok=True)
            print(f"Watermark removed: {args.output}")
        else:
            print(f"Saved: {args.output}")

    elif args.command == "update-devto":
        api_key = None
        if args.persona:
            lib_path = str(Path(__file__).resolve().parent)
            if lib_path not in sys.path:
                sys.path.insert(0, lib_path)
            import persona_registry
            conn = persona_registry.connect_from_env()
            persona_registry.ensure_schema(conn)
            api_key = persona_registry.get_secret(conn, args.persona, "devto_api_key")
            conn.close()
            if not api_key:
                print(f"No devto_api_key secret for persona '{args.persona}'", file=sys.stderr)
                sys.exit(2)
        else:
            api_key = os.environ.get("DEV_TO_API_KEY")
            if not api_key:
                print("DEV_TO_API_KEY env var or --persona required", file=sys.stderr)
                sys.exit(1)

        result = update_devto_cover(api_key, args.devto_id, args.image_url)
        print(f"Updated: {result.get('url')}")
        print(f"Cover: {result.get('cover_image')}")

    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    _cli()
