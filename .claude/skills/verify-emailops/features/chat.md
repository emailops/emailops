# Chat with the inbox

The chat panel is docked on the right of the inbox and answers questions about the
selected account's mail with the local model, listing the emails it used as sources.
With a thread open, the panel grounds the answer in that thread (chip "USING AS CONTEXT").

## Sub-features

- `chat.panel` "Close chat panel" hides it; the inbox header's single chat button re-docks it (labelled "Open chat panel" while closed, "New chat" while open). The sidebar **Chat** entry opens the full-screen view.
- `chat.ask` typing a question and pressing Send streams an answer.
- `chat.sources` the answer lists sources; each has an "Open email" action.
- `chat.showInList` below the sources, "Show in email list" (`data-testid="chat-show-in-list"`) fills the inbox search box with `id:` operators for the emails the answer cites (`[n]` markers and `email://` links, not every retrieved source) and filters the list to them.
- `chat.new` "New chat" starts an empty conversation.
- `chat.persist` conversations and messages are written to `chat_conversations` / `chat_messages`.

## How to get to it (user POV)

- Open by default on the right. Sidebar → AI Features → **Chat**, or the header's chat icon (`Open full chat view`) for the full-screen view.
- From a thread row's ⋮ menu: "Chat about this thread".

## Driving it with verify.sh

Preconditions: baseline; AI enabled (`make cli-demo ARGS="doctor --json"` shows `aiEnabled: true`); no other process holding most of the GPU memory. Handles confirmed present on 11/09/2026: `aria/Close chat panel`, `aria/New chat`, `button=Send`, `aria/Open full chat view`, textarea placeholder `Ask about your emails…`.

- Ask → `$V wd type 'textarea[placeholder^="Ask about your emails"]' 'Which clients are asking about Ollama?'`, `$V wd click 'button=Send'`, then poll `$V wd find '*=Sources'` every 5 s for up to 60 s and `$V wd shot "$R/chat-answer.png"` → `$V wd find '*=Ollama'` prints lines from the answer.
- Persisted → `sqlite3 .emailops-demo-data/emailops.db "select count(*) from chat_messages where role='assistant'"` grew by one versus before the drive.
- Show in list → after an answer, `$V wd text '[data-testid="chat-show-in-list"]'` ("Show in email list" for one cited email), `$V wd click '[data-testid="chat-show-in-list"]'` → `$V wd js 'document.querySelector("input[placeholder^=\\"Search…\\"]").value'` prints `id:demo_…` and the list holds only the cited rows. Live run 17/09/2026: the Ollama question cites one of 9 sources → one row (Kwame Boateng).
- New chat → `$V wd click 'aria/New chat'` → `$V wd exists '*=Sources'` prints `absent`.
- Close / reopen → `$V wd click 'aria/Close chat panel'` → `$V wd exists 'button=Send'` prints `absent`; `$V wd click 'button=Chat'` brings it back.

## Gotchas

- First answer loads the model: allow up to 60 s and poll rather than sleeping blind; the textarea shows "Waiting for reply…" while streaming.
- The developer's own instance may hold a 35B model in GPU memory; a demo answer then fails or crawls. Check `<run>/app.log` for `Decode Error` / out-of-memory and report it as unreachable, not as a chat bug.
- The CLI path (`make cli-demo ARGS="chat '…' --json --trace"`) is the cheaper cross-check for answer content; the UI proof is for the panel, streaming and sources rendering.
- Chat searches the categories shown under the input ("Search: Primary, Updates" by default).
