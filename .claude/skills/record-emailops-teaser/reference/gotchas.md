# Teaser gotchas

Each one cost a retake on the first teaser. The general app traps are in
`record-emailops-demo/reference/gotchas.md`; read that too.

## The checkout is shared

**Another session can switch the branch under your instance.** The dev build
follows the working tree: a `git checkout` or `git pull` from another session
makes Tauri rebuild, the app dies mid-take (`ECONNREFUSED` from WebDriver) and
comes back minutes later on a different version (a "What's new" dialog
appears). Do not touch their work. Wait for WebDriver
(`curl -s 127.0.0.1:4445/status` in a background loop), then redo the whole
section, not just the missing frame: mixing versions changes the UI between
shots. Check `git status` / `git branch --show-current` before a long take.

**The dev server rewrites `src-tauri/gen/schemas`.** Restore them
(`git checkout -- src-tauri/gen/schemas`) after `verify.sh cleanup`, and only
if the tree was clean before you launched.

## Typing and clicking

**WebDriver's Space never arrives.** `b.keys(' ')` and `''` type
"Yes,Ollamaworks." into this WKWebView. Use `typeFrames` (native value
setter + `input` event), which also gives you a frame every two letters.

**A JS `.click()` on "All accounts" does not select it.** The title says
"Inbox — All accounts" but the sidebar keeps the work account highlighted.
Use a WebDriver click (`b.$('button=All accounts').click()`), **after**
`openChat()` — opening the chat re-selects the work account. `selectAllAccounts`
does both and throws if the highlight is wrong.

**`execCommand('insertParagraph')` loses lines in the EO Docs editor.**
Paste the whole document with `pasteHtml` (`insertHTML`), then reopen the doc
to prove it saved.

**The sidebar's low sections need its own scroll.** SMART FILTERS (companies,
intent, topic) is below the fold of the sidebar container; `scrollIntoView` on
a label did not find it (`innerText` is upper-cased by CSS). Scroll the
container (`sidebarScrolls`) and stitch the strip.

## AI output on camera

**Read the answer before you shoot the story around it.** Seen on the first
teaser: "How have EmailOps downloads grown?" routed to the help docs and said
there was no data; "What emails are waiting for my reply?" listed invoices;
"most urgent emails" printed `(priority=urgent)` and a note contradicting its
own table; one AI draft contained `[attach: <setup_instructions_file, named as
in the thread>]`. A concrete question about one thread ("What's the status
of the bug with orders stuck in processing?") answered cleanly with two
linked sources. Report every defect to the developer.

**Drafts appear at once; chat answers stream.** The draft shows "Generating
draft… retrieving similar threads" then the whole text; use a short dissolve,
not a fake typewriter. The chat streams: capture it with `chatFrames`.

**Long answers end scrolled to the bottom.** `chatToTop` before the shot if
the start of the answer matters.

## Composition

**Words over UI are unreadable.** When the camera zooms, a panel covers the
left column: lay the grid back over x < 680–700 with alpha = zoom, then draw
the words. Shorten a subline rather than let it run under a panel.

**Bright fields eat white text.** Darken the agate field (alpha ~115) and the
others (~55) under the opener questions.

**Highlight pills on top of the first row hide it.** Put a band's label pill
at the bottom of the band.

**Soft UI text had four causes, all fixed in the scripts.** (1) The 1080p
render shrank the 2x screenshots: a whole 1800-px window at 0.95 gives 11–12 px
text. Render at 4K (`TEASER_SCALE=2`). (2) Zooms past 2.0 CSS→logical upsample
the screenshot; cap them. (3) Animated zooms resized with BILINEAR; everything
is Lanczos now. (4) Two lossy passes (CRF 14 master, CRF 20 final) plus a
denoised CRF 25 preview: the master is now the only lossy pass for 4K, and the
1080p cut is a single encode from it.

**The preview shown to the developer must be < 30 MB** (SendUserFile limit):
`finish.sh` makes a 1080p30 copy without denoise, raising the CRF only until it
fits. It is still a compressed preview: judge sharpness on 1:1 stills.

**`rm` of a relative glob after `cd` is blocked** by the harness. Write every
render or preview to a new directory or name instead of cleaning one.
