---
name: record-emailops-short
description: "Make a vertical YouTube Short / Reel / TikTok (1080x1920, 30–45 s, text-only, sound-off first) that shows one EmailOps feature solving one problem, in the POV → before → after style the developer picked: a POV hook on a real screen, the painful manual way highlighted in red, the feature doing it (Lentes / chat / rules…) with an animated camera, kinetic captions, a typed-in prompt and click ripples, a before/after split screen, what happens next (e.g. a new row sliding in), and a closing card. Captures the real app on the synthetic demo instance, composes with a Python storyboard over short_fx.py, adds a CC BY music bed, and ships the MP4, a music-less cut, an .srt and a YouTube sheet. This is the default skill for any short; use record-emailops-demo for 16:9 promos, narrated walkthroughs and long how-tos."
argument-hint: <feature and the problem it solves, and the language>
allowed-tools: Bash, Read, Edit, Write, Grep, Glob
---

# Record an EmailOps short (POV → antes → después)

One feature, one problem, 30–45 seconds, readable with the sound off. The
format is the one the developer chose over two alternatives (a slow
card-by-card walkthrough and a result-first numbered version):
`reference/storyboard.md` has the beat sheet, the reasons, and the research
behind it. `examples/lens_contacts.py` is the shipped short, frame for frame.

```bash
S=.claude/skills/record-emailops-short
D=.claude/skills/record-emailops-demo      # capture tooling lives there
```

## Pipeline

| Step | What | Where |
|---|---|---|
| 1. Beats | Fill the beat sheet for this feature, in the viewer's language | `reference/storyboard.md` |
| 2. Data | Scratch copy of the demo DB + the synthetic emails the story needs | `$D/SKILL.md` §2 "Data the demo lacks" |
| 3. Run | Launch the instance on the copy, UI in the short's language | `verify-emailops` (`VERIFY_DATA_DIR`) |
| 4. Check | Drive the feature once and confirm its output is right | if not: stop, `fix-ai-bug` |
| 5. Capture | One screenshot per beat; measure every control and region | capture script + `$D/scripts/emailops-ui.mjs` |
| 6. Storyboard | Copy `examples/lens_contacts.py`, change facts and shots | `$S/scripts/short_fx.py` |
| 7. Preview | Save the last frame of every shot (plus clicks and typing) and look at them | `storyboard.py preview 3 12.8 …` |
| 8. Render | `storyboard.py out.mp4` (writes the `.srt` too) | ~1 s of wall time per second of video |
| 9. Music | Trim, fade, `loudnorm`, mux | below |
| 10. Ship | Copy to `docs/marketing/videos/` + YouTube sheet | below |

Run the storyboard from the directory that holds `frames/` (or set
`FRAMES=`), with Pillow loaded just for the run:

```bash
uv run --no-project --with pillow python storyboard.py preview 3.0 12.8 33
uv run --no-project --with pillow python storyboard.py short.mp4
```

## Capture: what each beat needs

Shoot the whole flow in one take, in the short's UI language
(`ui_language` preference in the scratch DB), window at the size
`emailops-ui.mjs` sets. Name frames by beat (`f01-inbox…`, `f02-email…`).

- **POV**: the list where the problem shows (inbox with the notifications).
- **Before**: one item open with the data the user would copy by hand.
- **Walkthrough**: the frame *before* every click (the storyboard draws the
  pointer and ripple on it), then the result. Call `reveal(label)` before
  `rectOf` for sidebar entries. For a typed request, capture the input
  already filled: the storyboard types it letter by letter over the real
  frame and ends on the real pixels.
- **Result**: the finished view (the filled table).

Then measure, in CSS px on the real capture (or on a copy scaled to exactly
1800 wide, never a preview at another width): the fields to highlight, the
input box and where its text starts, the table's row tops, row height and
cell x positions, and the text colour and baseline of a row if a new one will
slide in. Copy the input's text **split exactly where the app wraps it**.

### Other languages

A version in another language is a new take, not a re-caption: the viewer
must see the UI in their language. Rebuild the scratch data dir with
`ui_language` set to it, relaunch, and run the same capture script with the
UI strings for that language (read them from `src/locales/<lang>/*.json`;
keep them in one dict per language at the top of the script). Then
re-measure: the same form sat 15 px higher in `en`/`de` than in `es` (shorter
help text), and menus, buttons and dates move. Keep one storyboard with a
per-language dict of texts and one of geometry, selected by an env var, and
preview every language: German lines are longer and wrap.

Some things stay as the app shows them: the demo's email subjects and PDFs
are English, a prefilled attachment-rule name is English in every locale,
and dates follow the locale ("2 may", "May 2", "2. Mai"). Match them when
you draw something new (the date of a row that slides in).

## Storyboard rules

- **Gancho on screen from frame 0**: a `pill` ("POV") plus one line with the
  highlighted pain, over the real screen. Draw both fully visible from the
  first frame (pass `1` as their time), not popping in: frame 0 is the
  thumbnail and what a scroller sees.
- **Antes in red, Después in green.** Highlight the fields one at a time,
  each with its label, while the caption counts the chore.
- **Walkthrough: one caption per action, at the top**, camera easing from
  the whole window to the control, pointer arriving, ripple, cut to the
  result on the same view. Never a static crop.
- **Keep what the app fills in.** When a form arrives prefilled, show it as
  the app proposes it and change only the minimum the story needs (drop the
  month from a subject so every invoice of that supplier matches), with the
  change animated key by key. Rewriting the defaults (a catch-all sender, a
  new name) was rejected: it no longer shows how a user creates the rule.
- **Labels on a zoomed form go inside the field.** At a zoom that keeps the
  text readable the input spans the stage, so `highlight`'s label lands off
  the picture; draw a small tag at the right end of the input instead.
- **Split screen for the payoff**: the before item on top, the table filling
  row by row below (`table_reveal`), headline "De N correos a 1 tabla".
- **What happens next, shown, not told**: `insert_row` slides a new row in
  and `highlight` marks it "nuevo". Check the claim in the code first
  (Lenses: the sync hook in `services/emails/sync.rs`).
- **Close** with `end_card`: product, one-line tagline, URL, platforms,
  music attribution.
- Use the UI's own words: in Spanish a Lens is always **"Lente"**.
- Keep every card to one idea. Durations that worked: POV 3.8 s, before 5 s,
  after card 2.6 s, each click 1.5–2 s, typing 5.9 s, form 4.8 s, split 6 s,
  new row 4.4 s, end 3.8 s.

## Music and publishing

The music-only cut of the promo (`docs/marketing/videos/emailops-promo-en-sin-voz.mp4`,
"Deliberate Thought", Kevin MacLeod, CC BY 4.0) is the bed; start it at a
different offset per short:

```bash
ffmpeg -ss 30 -i emailops-promo-en-sin-voz.mp4 -vn -af "atrim=0:$T,asetpts=PTS-STARTPTS,\
afade=t=in:st=0:d=0.5,afade=t=out:st=$((T-2.5)):d=2.5,loudnorm=I=-16:TP=-1.5:LRA=11" \
-ar 48000 -ac 2 bed.wav
ffmpeg -i short.mp4 -i bed.wav -c:v copy -c:a aac -b:a 192k -shortest -movflags +faststart final.mp4
```

Copy into `docs/marketing/videos/` (gitignored): `<name>.mp4`,
`<name>-sin-musica.mp4`, `<name>.srt`, and `youtube-<name>.md` (title,
description with the CC BY attribution, tags, upload settings — copy the
layout of `youtube-short-lens-contactos-alt2.md`). If the short depends on a
fix, say in the sheet that it must not be published before that fix ships.

Other languages: `<name>-<lang>.mp4`, `-no-music.mp4`, `.srt` and one sheet
per language, with the title, description and tags written in that language
(the notes stay in Spanish). Publish each language as its own Short, since
the language is burned into the picture and multi-language audio or
translated titles on one video would still show the other language. Set
"Idioma del vídeo" on each, space them a few days apart, and keep one
playlist per language with the same name translated ("Funcionalidades",
"Features", "Funktionen"). Say in the sheet which older short it replaces.

## Traps

`$D/reference/gotchas.md` covers the app and the capture. Specific to shorts:

- Helvetica Neue has no "→": it renders as a box. Write "a", ":" or ",".
- Put punctuation inside the markup (`**sola.**`), or it floats after a space.
- Views are clamped to the picture; keep `V()` for every view.
- A preview is the only cheap check: look at the last frame of every shot
  (one contact sheet), the frame of every click and the typing before
  rendering. A card whose title wrapped to three lines covered its subtitle,
  and only the shot's last frame showed it.
- Stack text by the height it takes: `text_block` returns the y below the
  block, so place the next line from that (`y + 60`), never at a fixed y. A
  translation or a longer title wraps and overlaps otherwise.
- zsh does not split `$var` into words in loops; use arrays.
