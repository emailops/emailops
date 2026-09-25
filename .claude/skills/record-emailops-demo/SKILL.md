---
name: record-emailops-demo
description: "Record a demo video of the real EmailOps app — a promo, a feature short (vertical 9:16 or 16:9) or a how-to, narrated or text-only — from the synthetic demo instance, never the developer's mailbox. Drives the app through WebDriver measuring every control it clicks, so the video can draw a pointer and a click marker that land exactly where the action happened; composes the shots with an animated camera that eases from the whole window onto each action (the click, the chat being filled, the result) or as still 1:1 shots, with title and section cards, dissolves and paced pauses; speaks the script locally with Kokoro (Apache-2.0, safe to publish, unlike the macOS say voices); mixes the narration over a licensed music bed with ducking; and ships the MP4 with no burned-in text plus one .srt per language, a music-only cut, the narration alone and a YouTube sheet. Use when the user asks for a 16:9 promo, a narrated demo or a tutorial video of EmailOps, or wants one re-cut. For a short (YouTube Shorts, Reels, TikTok, any vertical clip) use record-emailops-short instead — it is the default for shorts."
argument-hint: <what the video should show, and in which language>
allowed-tools: Bash, Read, Edit, Write, Grep, Glob
---

# Record an EmailOps demo video

Produces a screen recording of the real app: a promo, a feature short, or a
how-to, narrated or text-only. Output is an MP4 with no burned-in text, subtitle tracks in as many
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
| 7. Build | `build_camera.py` (animated camera, the default) or `make_manifest.py` + `build_short.py` (still shots) | silent MP4 + `.srt` |
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

**Screenshots are retina.** On this Mac `saveScreenshot` writes 2x the CSS
size (a 1800-wide window gives a 3600-wide PNG), while `rects.json` holds CSS
pixels. `build_camera.py` takes CSS coordinates and a `css_scale` of 2;
`build_short.py` wants screenshot pixels, so double everything there. Measure
positions on the real file or on a copy scaled to exactly the CSS width,
never on a preview shown at some other width.

**Check the feature's output before you shoot the story around it.** If the
result is wrong (a column comes back empty, a draft misses the point), stop
and fix it through `fix-ai-bug`; never stage a result by hand. Say in the
YouTube sheet which fix the video depends on, so it is not published before
that fix ships.

**Data the demo lacks goes into a scratch copy, never into the repo's demo
DB.** Copy the DB with `sqlite3 "file:<db>?immutable=1" ".backup <copy>"`,
symlink `models`, and add synthetic emails through
`generate_demo_db.insert_email` (it fills `email_bodies` and the FTS index).
Point `VERIFY_DATA_DIR` at the copy. For a retake, rebuild the copy rather
than undoing state in the app: deleting what the last take created leaves
traces on screen (see gotchas).

## 3. Cards

```bash
python3 $R/make_cards.py cards.json cards/
```

Four layouts: `hero` for the opening title, `section` for a feature name,
`end` for the closing screen with links and the music attribution, and
`stack` for free lines of text. Cards are 1920x1080 by default; a vertical
short passes `"size": [1080, 1300]` (its stage) and uses `stack`.

## 4a. Animated camera (default for shorts and how-tos)

The developer wants the picture to **move to where the action is**: start on
the whole window, then zoom smoothly onto the control about to be clicked,
the chat box being filled, the form that appeared, the table that came out.
Static crops that jump between shots read as a slideshow. `build_camera.py`
does this:

```bash
uv run --no-project --with pillow python $R/build_camera.py camera.json silent.mp4
```

`reference/camera.example.json` is a complete, shipped short (the Lens one)
to copy from. Each screenshot shot has camera keyframes (`cam`), pointer
keyframes (`ptr`, ending on the target), a `click` time and captions; views
ease in log space, the pointer glides, a ripple marks the click. Camera
grammar that worked:

- **Establish, then close in.** The first shot of a section holds `"full"`
  for ~0.6 s, then eases (~1.2 s) to a view about 700 CSS px wide on the
  target. The click lands ~0.3 s after the pointer arrives.
- **Carry the view across the cut.** The result of a click starts on the
  view the click ended on (`dissolve`), then moves to the next target. The
  pointer starts where the last click left it.
- **Follow the work, not the cursor.** Typing: hold on the input and push
  in slightly. Waiting: move to where the answer will appear. A form: read
  the answer first, then pull back to the whole form, then close in on the
  part that matters.
- **End wide, then zoom onto the result.** Pull back to `"full"` for a beat
  so the viewer sees where the table lives, then ease onto it with a
  letterboxed view (`[cx, cy, w, h]` with the table's own aspect) so it fills
  the width.
- **Slow the walkthrough.** The how-to part reads better about 1.35x slower
  than the intro (`"slow": 1.35` on those shots).

## 4. Author the plan (still shots, `build_short.py`)

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

## Shorts live in another skill

Vertical shorts use **`record-emailops-short`** (POV → antes → después,
kinetic captions, typed prompts, split screen). This skill's capture tooling
(`emailops-ui.mjs`, the data notes above) is what that skill shoots with; its
composers here (`build_camera.py`, `build_short.py`) are for 16:9 promos and
narrated walkthroughs.

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

Music without narration: the music-only cut of an earlier promo
(`emailops-promo-en-sin-voz.mp4`, "Deliberate Thought", CC BY 4.0) is a clean
bed source: trim it, fade in 1 s and out 3 s, `loudnorm=I=-16:TP=-1.5`.

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
