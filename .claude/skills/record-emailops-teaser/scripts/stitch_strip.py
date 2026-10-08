"""Stitch sidebar screenshots taken at several scroll offsets into one tall strip.

    uv run --no-project --with pillow python stitch_strip.py <frames_dir> <prefix> <y0> <y1> <out.png> top1 top2 ...

<prefix>-<top>.png are 2x screenshots from teaser-capture.mjs sidebarScrolls();
y0/y1 are the CSS top/bottom of the sidebar's scroll viewport (it prints them);
the tops are the scrollTop values the container really took. The strip is 2x,
512 px wide (the 256 CSS px sidebar); a scene pans a window over it.
"""
import sys
from pathlib import Path

from PIL import Image


def main():
    d, prefix, y0, y1, out = Path(sys.argv[1]), sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5]
    tops = [int(v) for v in sys.argv[6:]]
    base, view = tops[0], y1 - y0
    strip = Image.new("RGB", (512, (tops[-1] + view - base) * 2))
    for t in tops:
        im = Image.open(d / f"{prefix}-{t}.png").crop((0, y0 * 2, 512, y1 * 2))
        strip.paste(im, (0, (t - base) * 2))
    strip.save(out)
    print(out, strip.size)


if __name__ == "__main__":
    main()
