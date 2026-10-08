---
name: record-emailops-teaser
description: "Make a 16:9 launch teaser of EmailOps (60–75 s, 1920x1080 60 fps, text-only, music bed) in the minimalist product-launch style the developer picked from a reference (kyzo's \"One\" launch on posts.design): a cinematic opener of three privacy questions over generated colour fields, a grid-paper canvas with blue dots and a headline, then beats that each pair a big two-line phrase on the left with the real app UI on the right — an animated camera that starts wide and pushes onto the action, text typed letter by letter, macOS keycaps, a pointer with click ripples, highlight bands with label pills — and a closing logo card. Captures the real app on the synthetic demo (or a scratch copy with synthetic extras), composes with a Python storyboard over teaser_fx.py, and ships the MP4, a music-less cut, a <30 MB preview, .srt per language and a YouTube sheet. Use when the user asks for a teaser, a launch video, a product-announcement video, or a video \"like this one\" pointing at a launch clip; for narrated walkthroughs use record-emailops-demo, for vertical shorts record-emailops-short."
argument-hint: <reference video or the features/message to cover, and the language>
allowed-tools: Bash, Read, Edit, Write, Grep, Glob
---

# Record an EmailOps launch teaser

Text-only, music under it, one idea per beat, the real app as the proof. The
format came out of eight rounds with the developer on the first teaser
(`docs/marketing/videos/emailops-launch-v8.mp4`); the worked example in
`examples/` reproduces that cut frame for frame. Start from it.

```bash
K=.claude/skills/record-emailops-teaser
W=<scratchpad>/teaser            # work dir: frames, previews, renders
```

## What the developer wants (do not re-litigate)

- **Privacy first, from second one.** Local AI and privacy are the main
  proposition. Never open on a big-tech brand (Gmail…): the opener is three
  questions — *Your inbox knows everything about you. / Your clients. Your
  invoices. Your family. / Why send it to someone else's AI?* Account chips
  say WORK / PERSONAL / SIDE PROJECT, not provider names.
- **Order:** opener → "Every account. One inbox." → the unified inbox
  (**All accounts selected**, chat open and **empty**) → the next beat starts
  on that same view, zooms onto the chat, the question is **typed**, the answer
  **streams in** → "All AI runs on your computer" → Classified by AI (zoom +
  highlight on the tags, then on the companies) → Filter by any tag (sidebar) →
  Type the idea / Get the reply (AI Draft from a typed idea) → Read any
  language → Find anything. Instantly (⌘K, indexed locally) → Different views
  (Tag Board, attachments, calendar) → Shared docs. No cloud (EO Docs, a long
  doc with headings) → **Synced by email** (right after the EO Docs beat) →
  Runs on Mac, Windows or Linux* → closing card (Local AI · Private · Free and
  open source).
- **Show, don't claim.** A beat shows the feature happening (typing, the
  click, the result appearing), not a static screenshot. A first app shot must
  be big (≈0.95 of the frame), not a thumbnail.
- **Keycaps only for real shortcuts** (`src/lib/shortcuts.ts`): ⌘K search,
  ⌘↵ send, ↵ in the chat. AI Draft has none: show a click. "Send in a
  keystroke" was cut as low value.
- **Accurate small print.** Keep these qualifiers unless the developer
  removes them: remote models are opt-in; Windows/Linux built-in AI needs a GPU
  (Vulkan or CUDA) *to run at full speed* (it falls back to CPU); Mac needs
  Apple Silicon, Intel Macs can use Ollama; EO Docs sync rides on the user's own
  mail account — "No EmailOps servers. No third-party storage", not "nothing
  is on any server".

## The pipeline

| Step | Tool | Output |
|---|---|---|
| 1. Data | `scripts/scratch_demo.py` | scratch data dir with synthetic extras |
| 2. Run | `verify-emailops` `verify.sh launch` with `VERIFY_DATA_DIR` | instance on WebDriver 4445 |
| 3. Capture | `examples/capture_launch.mjs` (or your own on `scripts/teaser-capture.mjs`) | `$W/frames*/…png` |
| 4. Strip | `scripts/stitch_strip.py` | `frames6/sidebar-strip.png` |
| 5. Storyboard | copy `examples/launch_teaser.py`, edit scenes + `TIMELINE` | — |
| 6. Preview | `… launch_teaser.py $W/pv/f --preview t1,t2,…` | stills to check |
| 7. Render | `… launch_teaser.py $W/out/silent-4k.mp4` | silent 4K master (one lossy pass) |
| 8. Subtitles | `scripts/make_srt.py cues.json` | `<name>-<lang>.srt` |
| 9. Finish | `scripts/finish.sh` | `-4k` (YouTube), 1080p (X), sin-música, preview; copied to `docs/marketing/videos/` |
| 10. Show | `SendUserFile` the `-preview.mp4` (the master is >30 MB) | — |

### 1–2. Data and app

```bash
$K/scripts/fetch_fonts.sh                                    # Inter (OFL) into ~/.cache, once
uv run --no-project python $K/scripts/scratch_demo.py $W/demo $K/examples/emails.example.json
VERIFY_DATA_DIR=$W/demo .claude/skills/verify-emailops/scripts/verify.sh launch
```

The demo has no foreign-language email and no EO Docs: they go in the scratch
copy (`emails.example.json` adds a synthetic German email), never in the
repo's demo DB. Everything stays synthetic — invented names, `.example` domains.

### 3. Capture

`node $K/examples/capture_launch.mjs $W` captures every folder the example
storyboard reads. **Read every AI output it prints before composing** (the
chat answer, the draft). If one is wrong, leaks internals (`priority=urgent`)
or carries a placeholder (`[attach: …]`), re-run that section or regenerate
(the draft's *Regenerate* button) — never stage a result. Report the defect to
the developer and offer `fix-ai-bug`.

For a new beat, write a section on `openTeaser` (`teaser-capture.mjs`):
`selectAllAccounts`, `typeFrames` (letter by letter, frame every 2 chars),
`sendChat` + `chatFrames` (streaming), `draftFrames`, `sidebarScrolls`,
`pasteHtml`, `chatToTop`, `lastAnswer`. Shoot the frame **before** a click
(with `rectOf`, which measures it) and the result after.

Stop the instance with `verify.sh cleanup` and `git checkout -- src-tauri/gen/schemas`
when the shoot is over.

### 5–7. Storyboard

A scene is `scene(t) -> RGBA Image` on local time `t`; `TIMELINE` is
`[(start, dur, scene), …]` and `run(TIMELINE)` renders.

**Sharpness: render at 4K, think in 1080.** Scenes use logical 1920x1080
coordinates; images are physical, `TEASER_SCALE` (default 2) times bigger, so
the master is 3840x2160 and the 2x screenshots land 1:1 in wide shots. The
first teaser rendered at 1080p looked soft: a 1800-px window shown whole had
11–12 px text. Keep scene code resolution-free: `new_img` (not `Image.new`),
`draw` (not `ImageDraw.Draw`), `blur`, `lw`/`lh` for an image's logical size,
`tlen` for text length, `grid_strip(x)` for clean paper. Never zoom a panel past
2.0 CSS→logical (the screenshot's resolution); `TEASER_SCALE=1` gives fast 1080p
previews of motion.

Grammar that worked (all in `teaser_fx.py`, used in `launch_teaser.py`):

- `GRID.copy()` canvas; `beat_text(c, t, line1, line2, sub)` — left column,
  76 px InterDisplay, words rise in one by one.
- `place(c, name, css_box, scale, x, y)` — a rounded, shadowed crop of a 2x
  screenshot; `cam_place(…, [state_a, state_b], z)` eases between camera
  states; `focus_state(box, s, fx, fy, sx, sy)` puts CSS point (fx, fy) at
  screen (sx, sy). Wide (~0.95) → push (1.3–2.3) onto the action.
- When a zoomed panel fills the frame, lay `GRID.crop((0, 0, 680–700, H))`
  back over the left with alpha = zoom, then draw the words: the text never
  sits on UI.
- Swap frames by time to animate: typing frames over ~2.2 s, wait/stream
  frames over ~1.7 s, then the result.
- `draw_keys`, `pointer` + `ripple`, highlight band + blue label pill (see
  `sc_classify2`), `draw_cloud`/`envelope`/`laptop` for diagrams,
  `field(t, **FIELDS[name])` for the cinematic colour fields (no stock
  footage: nothing to license).

Preview stills at the key moments of every changed beat and **look at them**
(scale to exactly 1920 or 960 wide) before the 2-minute render. Check: words
never overlap a panel, nothing important is cut by the frame, small UI text is
readable (≥ ~11 px on screen), the highlight sits on the right column.

### 8–10. Subtitles, finish, show

`examples/cues.example.json` holds the v8 cues in en/es/fr/de. When beats
move, move their cues with them. Then:

```bash
uv run --no-project python $K/scripts/make_srt.py cues.json $W/out emailops-launch-vN
$K/scripts/finish.sh $W/out/silent-4k.mp4 <music.mp3> emailops-launch-vN $W/out
```

`finish.sh` muxes the music onto the 4K master without re-encoding
(`-4k.mp4`, for YouTube), derives the 1080p cut with a Lanczos downscale at
CRF 16 (`.mp4`, for X, which serves 1080p), copies its video into the
music-less cut, and makes a 1080p30 preview under 30 MB with no denoise
(denoise smears small UI text). When the developer judges sharpness, send a
couple of 1:1 crops of 4K stills too: the preview is still compressed.

Music: "Inspired", Kevin MacLeod (incompetech.com), CC BY 4.0 —
`https://incompetech.com/music/royalty-free/mp3-royaltyfree/Inspired.mp3`.
The attribution in the YouTube description is mandatory. Update
`docs/marketing/videos/youtube-emailops-launch.md` (title, description, tags,
attribution, which synthetic data was added) for the new version.

Bump the version suffix on every cut the developer reviews, and send the
`-preview.mp4` with `SendUserFile` (`display: render`).

## Gotchas

`reference/gotchas.md` — read it before the first capture. The ones that cost
the most: another session switching the branch rebuilds the app under you;
WebDriver drops spaces when typing; a JS click on "All accounts" does not
select it and opening the chat re-selects the work account.
