#!/usr/bin/env bash
# Finish a teaser from the silent master (4K by default, see TEASER_SCALE):
#
#   finish.sh <silent.mp4> <music.mp3> <name> [srt_dir]
#
#   <name>-4k.mp4         the master + music; the video stream is copied, not
#                         re-encoded (YouTube: upload this one, 4K gets far more bitrate)
#   <name>.mp4            1920x1080 from the master, Lanczos downscale, CRF 16 (X, web)
#   <name>-sin-musica.mp4 the 1080p cut without audio
#   <name>-preview.mp4    < 30 MB for SendUserFile (scratch only, never published)
#
# Then copies the published cuts and <name>-<lang>.srt (from srt_dir) into
# docs/marketing/videos/ (gitignored).
set -euo pipefail
# /usr/local/bin may hold x86_64 ffmpeg/ffprobe shadows that a non-Rosetta shell cannot run.
export PATH="/opt/homebrew/bin:$PATH"
SILENT="$1"; MUSIC="$2"; NAME="$3"; SRT_DIR="${4:-}"
REPO="$(cd "$(dirname "$0")/../../../.." && pwd)"
OUT="$(dirname "$SILENT")"
DEST="$REPO/docs/marketing/videos"
X264=(-c:v libx264 -preset slow -pix_fmt yuv420p -movflags +faststart)

D=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$SILENT")
WIDTH=$(ffprobe -v error -select_streams v:0 -show_entries stream=width -of csv=p=0 "$SILENT")
FADE=$(echo "$D - 3.4" | bc)
ffmpeg -v error -y -i "$MUSIC" \
  -af "atrim=0:$D,asetpts=PTS-STARTPTS,afade=t=in:st=0:d=0.4,afade=t=out:st=$FADE:d=3.4,loudnorm=I=-16:TP=-1.5:LRA=11" \
  -ar 48000 -ac 2 "$OUT/$NAME-bed.wav"

if [[ "$WIDTH" -gt 1920 ]]; then
  # master: one lossy pass only (the render), audio muxed alongside
  ffmpeg -v error -y -i "$SILENT" -i "$OUT/$NAME-bed.wav" -c:v copy -c:a aac -b:a 192k -shortest \
    -movflags +faststart "$OUT/$NAME-4k.mp4"
  SCALE=(-vf "scale=1920:1080:flags=lanczos")
else
  SCALE=()
fi
ffmpeg -v error -y -i "$SILENT" -i "$OUT/$NAME-bed.wav" ${SCALE[@]+"${SCALE[@]}"} "${X264[@]}" -crf 16 -tune film \
  -c:a aac -b:a 192k -shortest "$OUT/$NAME.mp4"
ffmpeg -v error -y -i "$OUT/$NAME.mp4" -an -c:v copy -movflags +faststart "$OUT/$NAME-sin-musica.mp4"

# Preview under the 30 MB SendUserFile limit: 1080p30, no denoise (it smears small text);
# raise the CRF only if it does not fit.
for CRF in 22 24 26 28; do
  ffmpeg -v error -y -i "$SILENT" -i "$OUT/$NAME-bed.wav" ${SCALE[@]+"${SCALE[@]}"} -r 30 "${X264[@]}" -crf "$CRF" \
    -c:a aac -b:a 128k -shortest "$OUT/$NAME-preview.mp4"
  SIZE=$(stat -f%z "$OUT/$NAME-preview.mp4" 2>/dev/null || stat -c%s "$OUT/$NAME-preview.mp4")
  [[ "$SIZE" -lt 30000000 ]] && break
done

mkdir -p "$DEST"
cp "$OUT/$NAME.mp4" "$OUT/$NAME-sin-musica.mp4" "$DEST/"
[[ -f "$OUT/$NAME-4k.mp4" ]] && cp "$OUT/$NAME-4k.mp4" "$DEST/"
if [[ -n "$SRT_DIR" ]]; then
  for f in "$SRT_DIR/$NAME"-*.srt; do [[ -e "$f" ]] && cp "$f" "$DEST/"; done
fi
ls -lh "$OUT/$NAME"*.mp4
echo "copied to $DEST (gitignored)"
