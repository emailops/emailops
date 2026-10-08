#!/usr/bin/env bash
# Finish a teaser: music bed cut to length, final cut, music-less cut, a <30 MB
# preview for SendUserFile, and a copy of everything into docs/marketing/videos/.
#
#   finish.sh <silent.mp4> <music.mp3> <name> [srt_dir]
#
# <name> e.g. emailops-launch-v9 -> <name>.mp4, <name>-sin-musica.mp4,
# <name>-preview.mp4 (scratch only), and <name>-<lang>.srt copied from srt_dir.
set -euo pipefail
SILENT="$1"; MUSIC="$2"; NAME="$3"; SRT_DIR="${4:-}"
REPO="$(cd "$(dirname "$0")/../../../.." && pwd)"
OUT="$(dirname "$SILENT")"
DEST="$REPO/docs/marketing/videos"

D=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$SILENT")
FADE=$(echo "$D - 3.4" | bc)
ffmpeg -v error -y -i "$MUSIC" -af "atrim=0:$D,asetpts=PTS-STARTPTS,afade=t=in:st=0:d=0.4,afade=t=out:st=$FADE:d=3.4,loudnorm=I=-16:TP=-1.5:LRA=11" \
  -ar 48000 -ac 2 "$OUT/$NAME-bed.wav"
ffmpeg -v error -y -i "$SILENT" -i "$OUT/$NAME-bed.wav" -c:v libx264 -preset slow -crf 20 -pix_fmt yuv420p \
  -c:a aac -b:a 192k -shortest -movflags +faststart "$OUT/$NAME.mp4"
ffmpeg -v error -y -i "$SILENT" -c:v libx264 -preset slow -crf 20 -pix_fmt yuv420p -an -movflags +faststart \
  "$OUT/$NAME-sin-musica.mp4"
# The upload limit for SendUserFile is 30 MB; 30 fps + light denoise keeps the grain from eating bits.
ffmpeg -v error -y -i "$OUT/$NAME.mp4" -vf "fps=30,hqdn3d=1.5:1.5:4:4" -c:v libx264 -preset slow -crf 25 \
  -maxrate 5M -bufsize 10M -pix_fmt yuv420p -c:a aac -b:a 128k -movflags +faststart "$OUT/$NAME-preview.mp4"

mkdir -p "$DEST"
cp "$OUT/$NAME.mp4" "$OUT/$NAME-sin-musica.mp4" "$DEST/"
if [[ -n "$SRT_DIR" ]]; then
  for f in "$SRT_DIR/$NAME"-*.srt; do [[ -e "$f" ]] && cp "$f" "$DEST/"; done
fi
ls -lh "$OUT/$NAME.mp4" "$OUT/$NAME-sin-musica.mp4" "$OUT/$NAME-preview.mp4"
echo "copied to $DEST (gitignored)"
