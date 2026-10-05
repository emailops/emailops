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
- `chat.viewContext` every turn from the panel carries what is on screen (`view/<mode>`, `settings/<tab>`, or `form/<id>` plus that form's values), so "esto"/"aquí" resolve. Validated backend-side; an unknown token is dropped, never prompted.
- `chat.fillForm` a request to CREATE something the app has a form for ("crea una lens de facturas con importe y fecha") gets the query planner's `{"form": "<id>"}` verdict: no retrieval, no tool loop, one focused completion fills the fields and the real form opens **non-blocking** with them in it. The user saves it; the chat never writes.
- `chat.wrongAnswer` every finished answer offers "This answer isn't right" (`data-testid="chat-mark-wrong"`). It asks for the reason first (`chat-wrong-reason` / `chat-wrong-submit`), then runs an ordinary corrective turn; the rejected answer stays in the conversation marked (`chat-rejected-note`).
- `chat.threadContext` with a thread open, the docked panel offers it as context ("Using as context" chip, removable; a thread from another account shows a "Switch" notice); `emailops-cli chat --thread <id>` grounds the turn the same way.
- `chat.conversations` past conversations can be reopened, renamed and deleted.
- `chat.account` the chat answers from one account, chosen in its account picker (`--account` in the CLI).
- `chat.research` the Research toggle estimates first, then shows batch progress and a Stop (`--research` / `--estimate` in the CLI).
- `chat.trace` each answer can show its reasoning trace (route, retrieval, tools, timings); `--trace` in the CLI.
- `chat.cancel` a running turn offers Cancel; a cancelled turn keeps what it streamed.
- `chat.shortcuts` an empty chat offers one-click prompts (today, this week, write a draft).
- `chat.categories` the chat searches the categories chosen under the input, remembered for the next session.

## How to get to it (user POV)

Four entry points, the columns of `## Parity`:

- **Docked panel** — open by default on the right; "Close chat panel" hides it, the inbox header button re-docks it.
- **Full chat view** — Sidebar → AI Features → **Chat**, or the panel's `Open full chat view`; conversation list and shortcut chips.
- **Chat about this thread** — a thread row's ⋮ menu: starts a conversation bound to that thread and opens it in the full view.
- **emailops-cli chat** — `make cli-demo ARGS="chat '…' --json [--trace] [--thread ID] [--conversation ID] [--research [--estimate]]"`. The REPL `/chat` is a separate path (no `--thread` / `--research` / `--conversation`).

## Parity

| Capability | Docked panel | Full chat view | Chat about this thread | emailops-cli chat |
|---|---|---|---|---|
| chat.panel | e2e:Chat/cerrar y reabrir | gap: untested — no sweep step opens the full view and ChatView has no test | n/a: opens the full view, it has no panel of its own | n/a: no window |
| chat.ask | e2e:Chat/pregunta y respuesta | gap: untested — ChatView's send has no test or sweep step | gap: untested — no test asks in a thread-bound conversation from the UI | gap: untested — run_chat has no test; only clap parsing |
| chat.sources | gap: untested — Chat/pregunta y respuesta also passes on the token footer alone and never opens a source | gap: untested — no test renders sources or Open email in ChatView | gap: missing — thread-bound turns never write sources, so there is no sources list or Open email | gap: untested — sources in the JSON have no test |
| chat.showInList | gap: untested — no sweep step clicks chat-show-in-list; the component test passes the handler in directly, not through ChatPanel | gap: untested — ChatView passes onShowEmailsInList, untested | gap: untested — thread-bound turns still collect cited email links, so the button can appear; no test | n/a: there is no email list to filter; cited ids are in the answer and sources |
| chat.new | e2e:Chat/nuevo chat | gap: untested — ConversationList New chat and /clear in ChatView have no test | gap: untested — each Chat about this thread creates a new seeded conversation; only the backend seeding is tested | gap: untested — a chat without --conversation opens a new conversation; no test |
| chat.persist | gap: untested — send_chat_message writes both rows; no test asserts it and no sweep step reads chat_messages | n/a: backend, same send_chat_message path as the docked panel | n/a: backend, same send_chat_message path as the docked panel | gap: untested — run_chat inserts the rows itself, untested |
| chat.viewContext | gap: untested — ChatPanel sends the view context; only the store and the backend parsing are tested | gap: untested — ChatView sends it the same way, untested | n/a: a thread-bound turn answers from its thread only and takes no view context | n/a: headless, nothing on screen |
| chat.fillForm | e2e:Chat/Formularios/rellenar Crear Lens desde el chat | gap: untested — the same planner verdict and App-level effect apply, but no step asks from the full view | gap: missing — thread-bound turns return before the form verdict | gap: missing — the CLI drops chat-tool-effect events and the JSON has no form values |
| chat.wrongAnswer | e2e:Chat/reintento correctivo | gap: untested — ChatView passes onRejectMessage, untested | gap: missing — the correction is dropped: the thread-bound turn gets no correction | gap: missing — no flag to reject an answer with a reason |
| chat.threadContext | gap: untested — the context chip and other-account notice have no component test; only the lib helper is tested | n/a: the full view replaces the mail, so no thread is on screen | n/a: the conversation is already bound to its thread and the chip is suppressed | gap: untested — --thread has no test, not even a parse test |
| chat.conversations | gap: missing — the panel only switches conversations; rename and delete exist only in the full view | gap: untested — ConversationList has no test | n/a: the seeded conversation joins the full view's list | gap: missing — --conversation continues one but nothing lists, renames or deletes conversations |
| chat.account | gap: untested — the panel's account picker has no test; only the lib helper is tested | gap: untested — the account picker in ChatView, untested | n/a: bound to the thread's account | rust:src-tauri/src/cli/session.rs::resolve_account_explicit_hint_overrides_saved_default |
| chat.research | vitest:src/components/Chat/ChatInput.test.tsx::shows the estimate with start and cancel, and blocks the input meanwhile | vitest:src/components/Chat/ChatInput.test.tsx::shows the estimate with start and cancel, and blocks the input meanwhile | n/a: research is ignored in a thread-bound conversation by design | rust:src-tauri/src/cli/commands.rs::the_estimate_line_shows_emails_batches_time_and_answer_form |
| chat.trace | vitest:src/components/Chat/ReasoningTrace.test.tsx::renders one numbered row per step, in the order the backend gives | vitest:src/components/Chat/ReasoningTrace.test.tsx::renders one numbered row per step, in the order the backend gives | gap: untested — no test of the trace a thread-bound turn records | rust:src-tauri/src/cli/output.rs::chat_trace_lines_walk_the_turn_steps |
| chat.cancel | vitest:src/components/Chat/MessageBubble.research.test.tsx::offers Cancel on an ordinary turn too, not only on research | vitest:src/components/Chat/MessageBubble.research.test.tsx::offers Cancel on an ordinary turn too, not only on research | rust:src-tauri/src/services/chat/turn.rs::a_thread_bound_turn_honours_a_cancel_raised_before_it_started | n/a: Ctrl-C ends the process with the cancelled exit code |
| chat.shortcuts | gap: missing — the panel's empty state is a text hint only | gap: untested — the shortcut chips have no test; the shortcut eval measures the prompts, not the chips | n/a: a seeded conversation is never empty | n/a: ready-made UI prompts; in a terminal the user types the question |
| chat.categories | gap: untested — the category dropdown under the input has no test | gap: untested — same dropdown in ChatView, untested | n/a: thread-bound turns do not search | gap: missing — no flag; always the saved default categories |

## Driving it with verify.sh

Preconditions: baseline; AI enabled (`make cli-demo ARGS="doctor --json"` shows `aiEnabled: true`); no other process holding most of the GPU memory. Handles confirmed present on 11/09/2026: `aria/Close chat panel`, `aria/New chat`, `button=Send`, `aria/Open full chat view`, textarea placeholder `Ask about your emails…`.

- Ask → `$V wd type 'textarea[placeholder^="Ask about your emails"]' 'Which clients are asking about Ollama?'`, `$V wd click 'button=Send'`, then poll `$V wd find '*=Sources'` every 5 s for up to 60 s and `$V wd shot "$R/chat-answer.png"` → `$V wd find '*=Ollama'` prints lines from the answer.
- Persisted → `sqlite3 .emailops-demo-data/emailops.db "select count(*) from chat_messages where role='assistant'"` grew by one versus before the drive.
- Show in list → after an answer, `$V wd text '[data-testid="chat-show-in-list"]'` ("Show in email list" for one cited email), `$V wd click '[data-testid="chat-show-in-list"]'` → `$V wd js 'document.querySelector("input[placeholder^=\\"Search…\\"]").value'` prints `id:demo_…` and the list holds only the cited rows. Live run 17/09/2026: the Ollama question cites one of 9 sources → one row (Kwame Boateng).
- New chat → `$V wd click 'aria/New chat'` → `$V wd exists '*=Sources'` prints `absent`.
- Close / reopen → `$V wd click 'aria/Close chat panel'` → `$V wd exists 'button=Send'` prints `absent`; `$V wd click 'aria/Open chat panel'` brings it back.
- Fill a form → ask "crea una lens para seguir las facturas de mis proveedores con el importe, la fecha y el proveedor", then poll `$V wd exists '[data-testid="lens-create-name"]'` for up to 120 s. `$V wd js 'document.querySelector("[data-testid=\"lens-create-name\"]").value'` prints the name the model wrote; `$V wd js '[...document.querySelectorAll("[data-testid=\"lens-create-column-key\"]")].filter(i=>i.value.trim()).length'` prints how many columns it filled (≥2 for that request).
- Chat still usable behind the form → with the form open, `$V wd js 'const t=document.querySelector("textarea[placeholder^=\"Ask about your emails\"]"),r=t.getBoundingClientRect();document.elementFromPoint(r.left+r.width/2,r.top+r.height/2)===t'` prints `true`. This is the whole point of `nonBlocking`: a normal modal would sit on top and swallow the click.
- Wrong answer → after an answer, `$V wd click '[data-testid="chat-mark-wrong"]'` → `$V wd js 'document.querySelector("[data-testid=\"chat-wrong-submit\"]").disabled'` prints `true` (no reason yet). Type a reason into `[data-testid="chat-wrong-reason"]`, click submit → `$V wd exists '[data-testid="chat-rejected-note"]'` prints `present` and the reason appears as a new user turn.

## Gotchas

- First answer loads the model: allow up to 60 s and poll rather than sleeping blind; the textarea shows "Waiting for reply…" while streaming.
- The developer's own instance may hold a 35B model in GPU memory; a demo answer then fails or crawls. Check `<run>/app.log` for `Decode Error` / out-of-memory and report it as unreachable, not as a chat bug.
- The CLI path (`make cli-demo ARGS="chat '…' --json --trace"`) is the cheaper cross-check for answer content; the UI proof is for the panel, streaming and sources rendering.
- Chat searches the categories shown under the input ("Search: Primary, Updates" by default).
- The form-fill step needs Lenses enabled (`lenses_enabled`); with the feature off the planner still routes to the form but the Lenses view is not reachable, so the sweep step reports SKIP rather than FAIL.
- An open form is a hint, never a gate: with "Crear Lens" on screen an ordinary mailbox question must still be answered from the mailbox. That boundary is pinned by `an_open_form_does_not_hijack_a_mailbox_question` in the planner eval, not by the sweep — it is cheaper to measure there.
- "This answer isn't right" runs a NEW turn; it never replaces the rejected bubble. A sweep step that asserts the old answer disappeared is wrong.
