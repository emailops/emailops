#!/usr/bin/env bash
#
# Install EmailOps' app icons into the generated Xcode asset catalog.
#
# Two things go wrong without this:
#
#  1. `tauri ios init` fills `gen/apple/Assets.xcassets/AppIcon.appiconset/`
#     with cargo-mobile2's placeholder (interlocking yellow/cyan rings), not the
#     app's icon. `tauri icon` meanwhile writes correct, correctly-named iOS
#     icons to `src-tauri/icons/ios/` — and nothing connects the two, so the
#     phone shows the template logo.
#
#  2. Those icons carry an alpha channel. Apple rejects an App Store icon that
#     "can't be transparent nor contain an alpha channel" (ITMS-90717), and the
#     check is on the channel's presence, not its contents. So the copy strips
#     it, compositing over white — lossless in practice here, since every
#     sampled pixel is alpha 254-255 and the corners are already opaque white.
#
# Re-run after `tauri icon` regenerates `src-tauri/icons/ios/`. Idempotent:
# converts to a temp file and only writes when the bytes actually differ.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_DIR="$REPO_ROOT/src-tauri/icons/ios"
DEST_DIR="$REPO_ROOT/src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset"

if [ ! -d "$SRC_DIR" ]; then
    echo "ERROR: $SRC_DIR not found. Generate icons first: npm run tauri icon <path-to-1024.png>" >&2
    exit 1
fi
if [ ! -d "$DEST_DIR" ]; then
    echo "ERROR: $DEST_DIR not found. Run 'make ios-init' first." >&2
    exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

# Stdlib only (struct + zlib): no Pillow, no ImageMagick, nothing to install.
# `sips` cannot drop an alpha channel, and round-tripping through a lossy
# format to lose it would be worse than doing the composite explicitly.
strip_alpha() {
    python3 - "$1" "$2" <<'PY'
import struct, sys, zlib

src, dst = sys.argv[1], sys.argv[2]
data = open(src, 'rb').read()
if data[:8] != b'\x89PNG\r\n\x1a\n':
    sys.exit(f"not a PNG: {src}")

pos, idat = 8, b''
width = height = colour_type = None
while pos < len(data):
    length = struct.unpack('>I', data[pos:pos + 4])[0]
    kind = data[pos + 4:pos + 8]
    chunk = data[pos + 8:pos + 8 + length]
    pos += 12 + length
    if kind == b'IHDR':
        width, height, bit_depth, colour_type = struct.unpack('>IIBB', chunk[:10])
        if bit_depth != 8:
            sys.exit(f"{src}: only 8-bit PNGs are handled, got {bit_depth}")
    elif kind == b'IDAT':
        idat += chunk
    elif kind == b'IEND':
        break

if colour_type == 2:  # already RGB with no alpha
    open(dst, 'wb').write(data)
    sys.exit(0)
if colour_type != 6:
    sys.exit(f"{src}: expected RGBA (colour type 6), got {colour_type}")

raw = zlib.decompress(idat)
channels, stride = 4, width * 4

def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    return a if pa <= pb and pa <= pc else (b if pb <= pc else c)

# Undo the per-scanline filters, then composite each pixel over white and drop
# the alpha byte. Output is unfiltered (filter 0) — larger, but an app icon is
# a few hundred KB and correctness beats compression here.
out = bytearray()
previous = bytearray(stride)
offset = 0
for _ in range(height):
    filter_type = raw[offset]; offset += 1
    line = bytearray(raw[offset:offset + stride]); offset += stride
    for x in range(stride):
        left = line[x - channels] if x >= channels else 0
        up = previous[x]
        up_left = previous[x - channels] if x >= channels else 0
        if filter_type == 1:
            line[x] = (line[x] + left) & 255
        elif filter_type == 2:
            line[x] = (line[x] + up) & 255
        elif filter_type == 3:
            line[x] = (line[x] + ((left + up) >> 1)) & 255
        elif filter_type == 4:
            line[x] = (line[x] + paeth(left, up, up_left)) & 255
    out.append(0)
    for x in range(0, stride, 4):
        r, g, b, a = line[x], line[x + 1], line[x + 2], line[x + 3]
        if a != 255:
            r = (r * a + 255 * (255 - a) + 127) // 255
            g = (g * a + 255 * (255 - a) + 127) // 255
            b = (b * a + 255 * (255 - a) + 127) // 255
        out += bytes((r, g, b))
    previous = line

def chunk(kind, payload):
    return (struct.pack('>I', len(payload)) + kind + payload
            + struct.pack('>I', zlib.crc32(kind + payload) & 0xffffffff))

png = b'\x89PNG\r\n\x1a\n'
png += chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
png += chunk(b'IDAT', zlib.compress(bytes(out), 9))
png += chunk(b'IEND', b'')
open(dst, 'wb').write(png)
PY
}

changed=0
for src in "$SRC_DIR"/*.png; do
    name="$(basename "$src")"
    strip_alpha "$src" "$TMP_DIR/$name"
    if ! cmp -s "$TMP_DIR/$name" "$DEST_DIR/$name"; then
        cp "$TMP_DIR/$name" "$DEST_DIR/$name"
        changed=$((changed + 1))
    fi
done

if [ "$changed" -eq 0 ]; then
    echo "app icons already current"
else
    echo "app icons: installed $changed opaque icon(s) into the asset catalog"
fi
