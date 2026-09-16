#!/usr/bin/env python3
"""Speak the narration lines with Kokoro, locally.

  narrate.py <lines.json> <out_dir> [--voice af_heart] [--speed 0.95]

lines.json is either a list of strings or {"voice": "...", "lines": [...]}.
One file per line, named line_01.wav upwards, which is the order the mixer and
the timing fitter expect.

Kokoro-82M is Apache-2.0, so the result can be published. The macOS `say`
voices cannot: their licence does not cover commercial use.

Setup, once:
    uv venv .venv && .venv/bin/python -m ensurepip
    .venv/bin/uv pip install kokoro-onnx soundfile
    curl -LO .../kokoro-v1.0.onnx  &&  curl -LO .../voices-v1.0.bin
    brew install espeak-ng          # arm64 build, see espeak note below
Run this script with that interpreter: .venv/bin/python narrate.py ...
"""
import json
import os
import sys
from pathlib import Path

# kokoro-onnx loads espeak-ng through a path baked in at build time. On Apple
# silicon the Homebrew prefix is /opt/homebrew; an x86_64 install under
# /usr/local loads but cannot run.
BREW = os.environ.get("ESPEAK_PREFIX", "/opt/homebrew")
os.environ.setdefault("PHONEMIZER_ESPEAK_LIBRARY", f"{BREW}/lib/libespeak-ng.dylib")
os.environ.setdefault("ESPEAKNG_DATA_PATH", f"{BREW}/share/espeak-ng-data")


def main():
    spec_path, out_dir = Path(sys.argv[1]), Path(sys.argv[2])
    argv = sys.argv[3:]
    voice = argv[argv.index("--voice") + 1] if "--voice" in argv else None
    speed = float(argv[argv.index("--speed") + 1]) if "--speed" in argv else 0.95

    spec = json.loads(spec_path.read_text(encoding="utf-8"))
    lines = spec["lines"] if isinstance(spec, dict) else spec
    voice = voice or (spec.get("voice") if isinstance(spec, dict) else None) or "af_heart"
    lang = (spec.get("lang") if isinstance(spec, dict) else None) or "en-us"

    import soundfile as sf
    from kokoro_onnx import Kokoro

    here = Path(__file__).parent
    model = os.environ.get("KOKORO_MODEL", str(here / "kokoro-v1.0.onnx"))
    voices = os.environ.get("KOKORO_VOICES", str(here / "voices-v1.0.bin"))
    kokoro = Kokoro(model, voices)

    out_dir.mkdir(parents=True, exist_ok=True)
    total = 0.0
    for i, line in enumerate(lines, 1):
        samples, rate = kokoro.create(line, voice=voice, speed=speed, lang=lang)
        target = out_dir / f"line_{i:02d}.wav"
        sf.write(target, samples, rate)
        seconds = len(samples) / rate
        total += seconds
        print(f"  {target.name}  {seconds:5.2f}s  {line[:60]}")
    print(f"{len(lines)} lines, {total:.1f}s of speech, voice {voice}")


if __name__ == "__main__":
    main()
