#!/usr/bin/env python3
"""Render the title, section and closing cards of a demo video.

  make_cards.py <cards.json> <out_dir>

cards.json:
  {"font": "/System/Library/Fonts/HelveticaNeue.ttc",
   "bg": "0x0f172a",
   "cards": [
     {"name": "card-open",  "layout": "hero",    "title": "EmailOps",
      "subtitle": "The local AI email client"},
     {"name": "card-ask",   "layout": "section", "title": "Ask your inbox",
      "subtitle": "A question in plain language"},
     {"name": "card-end",   "layout": "end",     "title": "EmailOps",
      "subtitle": "Free · Open source · Apache-2.0",
      "lines": ["getemailops.com", "github.com/emailops/emailops",
                "macOS · Windows · Linux"],
      "footnote": "Synthetic demo data · Music by Kevin MacLeod (CC BY 4.0)"}]}

Cards are 1920x1080 and are used uncropped, so they land 1:1 in the video.
"""
import json
import subprocess
import sys
from pathlib import Path

SIZE = (1920, 1080)
WHITE, BLUE, GREY, RULE = "white", "0x93c5fd", "0xcbd5e1", "0x334155"


def escape(text):
    """drawtext parses ':' and '\\' as syntax, and '%' as a strftime escape."""
    return text.replace("\\", "\\\\").replace(":", r"\:").replace("%", r"\%").replace("'", r"\\'")


def text(font, body, size, y, colour=WHITE):
    return (f"drawtext=fontfile='{font}':text='{escape(body)}':fontcolor={colour}"
            f":fontsize={size}:x=(w-text_w)/2:y={y}")


def rule(y, width=300):
    x = (SIZE[0] - width) // 2
    return f"drawbox=x={x}:y={y}:w={width}:h=2:color={RULE}:t=2"


def layout_hero(font, card):
    return [text(font, card["title"], 132, 402),
            rule(596),
            text(font, card.get("subtitle", ""), 52, 646, BLUE)]


def layout_section(font, card):
    parts = [text(font, card["title"], 96, 430)]
    if card.get("subtitle"):
        parts.append(text(font, card["subtitle"], 44, 580, BLUE))
    return parts


def layout_end(font, card):
    parts = [text(font, card["title"], 104, 232),
             text(font, card.get("subtitle", ""), 44, 372, BLUE),
             rule(470, 400)]
    y, sizes, colours = 534, (58, 42, 38), (WHITE, BLUE, GREY)
    for i, line in enumerate(card.get("lines", [])[:3]):
        parts.append(text(font, line, sizes[i], y, colours[i]))
        y += 92
    if card.get("footnote"):
        parts.append(text(font, card["footnote"], 22, 940, "0x475569"))
    return parts


LAYOUTS = {"hero": layout_hero, "section": layout_section, "end": layout_end}


def main(spec_path, out_dir):
    spec = json.loads(Path(spec_path).read_text(encoding="utf-8"))
    font = spec.get("font", "/System/Library/Fonts/HelveticaNeue.ttc")
    bg = spec.get("bg", "0x0f172a")
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)

    for card in spec["cards"]:
        build = LAYOUTS[card.get("layout", "section")]
        chain = ",".join(build(font, card))
        target = out / f"{card['name']}.png"
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi",
                        "-i", f"color=c={bg}:s={SIZE[0]}x{SIZE[1]}",
                        "-frames:v", "1", "-vf", chain, str(target)], check=True)
        print(f"wrote {target.name}")
    print(f"{len(spec['cards'])} cards in {out}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
