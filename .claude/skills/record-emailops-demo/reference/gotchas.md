# Traps, and what they look like

Each of these cost a rebuild or a reshoot at least once.

## The app

**The session banner.** The demo accounts have no credentials, so a sync attempt
raises a red "please sign in again" strip across the header. It lands in every
shot taken while it is up, and it moves the toolbar down, so a control measured
with the banner showing is a few dozen pixels off once it is gone. Call
`dismissBanner()` after every navigation, not once at the start.

**The app remembers what you did in the last take.** A translated email stays
translated, so a second shoot of the translation section starts at the end of
the story. Restore the original first: if the body shows "Show original", click
it before shooting.

**Controls are not named what you would guess.** The one that translates an
incoming email says **"Show translation"**, and `/translate/i` does not match
"translation". Walk the notice bar and read what the buttons actually say rather
than matching a verb.

**Rows scrolled out of view still click.** `element.click()` works on a row above
the viewport, so the action succeeds and the recorded coordinates are negative.
The marker then lands outside the frame. Measure only rows fully on screen.

**The list bottoms out.** Setting `scrollTop` to 2700 on a list that ends at 1330
silently clamps, and the shots after it jump backwards. Read the value back.

**Virtualised rows arrive late.** The first screenshot after opening the inbox
can catch an empty list. Wait until the row count settles.

**Zoom does not buy resolution.** `document.documentElement.style.zoom = 2`
doubles the drawing but the screenshot is still the same pixel size, the layout
reflows, and the chat composer falls off the bottom of the window. Frame the
crop wider instead.

**The UI language changes every label.** With `ui_language=es` the sidebar
says "Bandeja de entrada", "Lentes"; the buttons say "Nueva lente", "Con el
chat", "Crear lente", "Ejecutar reproceso". A few stay English ("Send",
"Reply", "Back"). Read the DOM before writing selectors for a localized take.

**The first inbox row is above `visibleRow`'s margin.** It sits at about 65 px
and the helper wants 80, so it is never picked. Target the second or third
row.

**Low sidebar entries are hidden under the output bar.** At this window
height "Lentes" measured at y=936 on a 938-tall viewport: clicked fine, but the
marker was off the picture. Call `reveal(label)` before `rectOf`.

**A Lens is not run when it is created.** After "Crear lente" the table is
empty until "Ejecutar reproceso" is clicked. A re-run skips rows already
extracted; use the row's ↻ to re-extract.

**Deleting the open Lens leaves a red "Lens … not found" banner** that shows
up in the next take. Rebuild the scratch data dir instead of deleting.

**A dev build is linked to the checked-out branch.** Switching branches, or
stashing a file, while the launcher runs makes Tauri rebuild the app under
you. Finish the shoot first.

## The launcher

`verify.sh launch` with no `VERIFY_DATA_DIR` targets the repo-local demo dir. If
that does not exist it tries to regenerate the demo database and fails on a
missing models dir. Point it at a data dir that already exists.

A launcher that is still starting will not answer on 4445 for a few minutes
while Rust builds. Starting a second one on top produces two instances fighting
over the same port and database. Check for a live launcher before relaunching.

The dev server rewrites `src-tauri/gen/schemas`. Restore those files when you are
done, and remove any `node_modules` symlink you created for the capture scripts.

## This ffmpeg build

- No `xfade`. Cross-fades are an `overlay` of the next canvas with its alpha
  faded in.
- `amix` has no `normalize` option; compensate with `volume=N` afterwards.
- `drawbox` rejects `t=fill`; pass a numeric thickness.
- `ebur128` rejects `framelog`; use `volumedetect` to measure a level.
- **`drawtext` breaks on a colon in the text.** The filter reads it as the start
  of the next option, loses the `fontfile`, and fails with "Cannot find a valid
  font for the family Sans". Escape it, or keep colons out of labels.
- `drawtext` inside `-filter_complex` is fussier than in `-vf`. Label images one
  at a time, then stack them.

## The composition

**Two lossy passes soften text.** Segments used to be encoded at CRF 18 and then
re-encoded into the final cut at CRF 18. Keep the intermediates near-lossless
(CRF 10) and tune the final pass for still images.

**The webview leaves a white strip** about 32 pixels tall below the output bar.
It reads as a blank band at the bottom of the video. Crop the screenshot to
1080 tall (`[0, 10, 1800, 1080]`), which also makes the scale exactly 1:1.

**A caption on one shot of a multi-shot line disappears mid-sentence.** The SRT
merges consecutive identical captions, so repeat the caption on each shot the
line is heard over.

## The review

**Contact sheets lie about geometry.** A frame scaled to 2000 px for viewing
is not in CSS pixels (1800) nor screenshot pixels (3600). A crop measured on
it came out 11 % off and cut the table's first column. Scale previews to
exactly the CSS width, or measure on the file.

**zsh does not split `$var` into words.** `for t in $ts` with `ts="2 9 18"`
runs once with the whole string. Use an array: `ts=(2 9 18)`.

