---
name: record-emailops-demo
description: "Record a narrated demo video of the real EmailOps app — a promo, a feature short or a how-to — from the synthetic demo instance, never the developer's mailbox. Drives the app through WebDriver measuring every control it clicks, so the video can draw a pointer and a click marker that land exactly where the action happened; composes the shots into a 1920x1080 cut with the UI at 1:1 scale, title and section cards, cursor moves, dissolves and paced pauses; speaks the script locally with Kokoro (Apache-2.0, safe to publish, unlike the macOS say voices); mixes the narration over a licensed music bed with ducking; and ships the MP4 with no burned-in text plus one .srt per language, a music-only cut, the narration alone and a YouTube sheet. Use when the user asks for a demo, promo or tutorial video of EmailOps, or wants an existing one re-cut."
argument-hint: <what the video should show, and in which language>
allowed-tools: Bash, Read, Edit, Write, Grep, Glob
---

# Record an EmailOps demo video

Produces a narrated screen recording of the real app: a promo, a feature short,
or a how-to. Output is an MP4 with no burned-in text, subtitle tracks in as many
languages as you write, a music-only cut, the narration on its own, and a sheet
with everything YouTube asks for.

Everything is driven from the **demo instance**, never the developer's mailbox.
The persona, accounts and 79 synthetic emails come from `verify-emailops`.

```bash
R=.claude/skills/record-emailops-demo/scripts
```

## The pipeline

| Stage | Tool | Output |
|---|---|---|
| 1. Run the app | `verify-emailops` skill | an instance on WebDriver 4445 |
| 2. Shoot | your capture script + `emailops-ui.mjs` | `frames/*.png`, `rects.json` |
| 3. Cards | `make_cards.py` | `cards/*.png` |
| 4. Author | write `plan.json` by hand | shots, narration, captions |
| 5. Time it | `fit_timings.py` | the same plan, stretched to fit the voice |
| 6. Speak | `narrate.py` | `vo/line_NN.wav` |
| 7. Build | `make_manifest.py` + `build_short.py` | silent MP4 + one `.srt` per language |
| 8. Sound | `mix_audio.py` | narration ducked over a music bed |
| 9. Ship | `ffmpeg` mux, then copy | the files listed under **Publishing** |

## 1. Run the app

```bash
export VERIFY_DATA_DIR=<an existing demo data dir>      # see Gotchas
.claude/skills/verify-emailops/scripts/verify.sh launch
```

Point `VERIFY_DATA_DIR` at a data dir that already exists. Without it the
launcher tries to rebuild the demo database and needs a models dir it will not
find. Stop it with the same script's `cleanup` when the shoot is over.

## 2. Shoot

Write one script per video and import the primitives:

```js
import { openApp } from '.../scripts/emailops-ui.mjs';
const ui = await openApp({ dir: 'frames' });
await ui.goInbox();
const row = await ui.visibleRow('Hetzner', 'invoiceRow');  // measured
await ui.shot('a03-before-row-click', row);                // the click marker frame
await ui.clickTarget();                                    // clicks what was measured
await ui.pause(2600);
await ui.shot('a04-invoice-email');
await ui.finish();                                         // writes rects.json
```

**Every control the viewer must see clicked is measured here.** The composer
draws the pointer from `rects.json`, so a coordinate that was guessed, or that
belongs to an element scrolled out of view, puts the red dot outside the
picture. `visibleRow` refuses rows that are not fully on screen for that reason.

Shoot in this order for an action: the frame **before** the click, then the
result. The plan puts the marker on the before-frame and cuts to the result.

## 3. Cards

```bash
python3 $R/make_cards.py cards.json cards/
```

Three layouts: `hero` for the opening title, `section` for a feature name,
`end` for the closing screen with links and the music attribution. Cards are
1920x1080 and are used uncropped.

## 4. Author the plan

```jsonc
{"size": [1920, 1080], "stage": [0, 0, 1920, 1080],
 "burn_captions": false,
 "segments": [
  {"image": "cards/card-ask.png", "seconds": 5.5,
   "vo": "Ask your inbox a question in plain language.", "vo_at_end": true,
   "caption": {"en": "...", "de": "...", "fr": "...", "es": "..."}},
  {"image": "frames/a03-before-row-click.png", "seconds": 2.4,
   "crop": [0, 10, 1800, 1080], "click": [838, 693], "cursor_from": [900, 600]},
  {"image": "frames/a04-invoice-email.png", "seconds": 2.6,
   "crop": [0, 10, 1800, 1080], "dissolve": true}
 ]}
```

Per shot: `image`, `seconds`, `crop`, `click`, `cursor` (pointer with no click),
`cursor_from`, `cut` (join with no fade), `dissolve` (cross-fade from the
previous shot), `caption` (one entry per language), `vo`, `vo_at_end`.

`make_manifest.py` splits the caption dict into what the composer wants and
drops the narration keys. Do not hand-write the manifest.

## 5, 6, 7, 8. Time, speak, build, mix

```bash
python3 $R/narrate.py lines.json vo/ --voice af_heart     # in the Kokoro venv
python3 $R/fit_timings.py plan.json vo/ --write           # grows shots to fit the voice
python3 $R/make_manifest.py plan.json manifest.json
python3 $R/build_short.py manifest.json silent.mp4        # writes the .srt tracks too
ffmpeg -i "<track>.mp3" -af "atrim=0:<total>,afade=t=in:st=0:d=1.5,\
afade=t=out:st=<total-4>:d=4" -ar 48000 -ac 2 bed.wav
python3 $R/mix_audio.py plan.json bed.wav vo/ audio.wav
ffmpeg -i silent.mp4 -i audio.wav -c:v copy -c:a aac -b:a 192k -shortest final.mp4
```

`fit_timings.py` prints where every line lands and how much silence sits around
it. Read that table before building: it is the pacing of the video.

## Rules that make it watchable

- **One window geometry for the whole video.** Mixing 1800x1100 with 1800x1122
  makes the picture jump scale between shots of the same screen.
- **Full screen by default, 1:1.** With `stage` at the full 1920x1080 and a crop
  of `[0, 10, 1800, 1080]`, the UI lands pixel for pixel and small text stays
  readable. Shrinking it to fit a caption band is what makes text blurry.
- **Zoom only for detail, framed wide.** A tight crop has to be blown up, and
  the upscale is the blur. Keep close-ups under about 1.7x.
- **Show the pointer even when nothing is clicked** (`cursor`), or a scrolling
  list looks like it moves on its own.
- **The voice introduces a view before the click that opens it**, and the result
  appears within a second of the marker.
- **A section card holds its line to the end** (`vo_at_end`), so the name of the
  feature is on screen for a beat before the explanation starts.
- **Dissolve within a section, fade between sections.** Same screen, different
  moment, means `dissolve`; a new section may fade through the background.
- **A caption lasts as long as the line is heard.** Repeat it on every shot the
  line plays over, or the subtitle disappears mid-sentence.
- **Subtitles ship as files.** Keep `burn_captions` false and upload the tracks.

## Narration and music

Kokoro-82M is Apache-2.0 and runs locally, so its output can be published. The
macOS `say` voices cannot: the licence does not cover commercial use.

Music from Incompetech (Kevin MacLeod) is CC BY 4.0. **The attribution in the
video description is mandatory**, and the closing card carries it too. Check the
track is longer than the video before cutting the bed.

## Publishing

Copy into `docs/marketing/videos/` (gitignored):

| File | What it is |
|---|---|
| `<name>.mp4` | the cut with narration and music |
| `<name>-sin-voz.mp4` | music only, to dub another voice |
| `<name>-vo.wav` | the narration alone, same cue placement |
| `<name>-<lang>.srt` | one per language |
| `youtube-<name>.md` | title, description, tags, section markers, upload settings |

Never publish real mailbox content: the demo data is synthetic, keep it that way.

## Gotchas

`reference/gotchas.md` carries the traps that cost real time: what this ffmpeg
build refuses, the controls whose label is not what you would guess, the session
banner that lands in the header of every shot, and the state the app remembers
between takes.
