# AI providers and models

Where AI requests go and with which models: the in-app llama.cpp runtime (default), a local
Ollama server, or OpenRouter (remote, paid). The backend is chosen in Settings → **AI Backend &
Models** only; the Logs status bar shows it as text and offers the chat-model selector. Each
provider remembers its own chat and embedding model. A change that would interrupt AI work in
progress asks first; a change of embedding model asks before re-indexing.

## Sub-features

- `ai.backendTabs` **In-app / Ollama / OpenRouter** tabs switch the form; nothing is stored until **Save**.
- `ai.openrouter` OpenRouter tab: API key, chat model prefilled with the default, **Embedding Model** selector (`None — keyword search only`, then the recommended models, then the provider's list when a key is saved), **Context budget (tokens)**, zero-data-retention toggle. No **Keep model loaded** there.
- `ai.embeddingProbe` an OpenRouter embedding model must pass a 768-dimension probe before it is used (paid call).
- `ai.modelMemory` switching provider restores the models that provider last used.
- `ai.statusBar` Logs bar: backend label (`data-testid="ai-backend"`) + `AI Model` selector (hidden for OpenRouter).
- `ai.workInProgress` changing provider or model while Embeddings/classification/extraction work is queued opens the stop-or-wait dialog; stopping is cooperative (each task ends at its next email).
- `ai.reindexConfirm` saving another embedding model asks before the index is rebuilt.
- `ai.onboarding` the onboarding AI step offers the same OpenRouter fields, probe and recommended models.

## How to get to it (user POV)

- Gear (`aria/Application settings`) → **AI Backend & Models**.
- Bottom **Logs** bar, right side: backend label and model selector.
- First run: onboarding → "Use AI" → backend step.

## Driving it with verify.sh

Preconditions: baseline. `verify.sh launch` starts the instance with `OPENROUTER_API_KEY` set and empty, so it holds **no** remote key (a debug build would otherwise load the developer's from `.env.local`); confirm with `$V wd js '(async()=>(await window.__TAURI_INTERNALS__.invoke("get_ai_config")).hasApiKey)()'` → `false` before touching the OpenRouter tab. Handles confirmed live on 30/09/2026.

- OpenRouter tab → `$V wd click 'aria/Application settings'`, click the last button containing `AI Backend`, then the button whose text starts with `OpenRouter` → the dialog (`button[title="Close settings"]` → `.closest('.fixed')`) holds a text input with placeholder `e.g. …` and a `vendor/model` value, `select[aria-label="Embedding Model"]` whose first option is `None — keyword search only` followed by the `— recommended` ones, `input[aria-label="Context budget (tokens)"]`, and no `Keep model loaded`.
- In-app tab → click the button starting with `In-app` → `Keep model loaded (minutes)` is back, `Context budget` gone; `select value from user_preferences where key='ai_provider'` still prints `llamacpp` (nothing was saved).
- Status bar → `$V wd text '[data-testid="ai-backend"]'` prints `Embedded`; its parent holds one `select` (`aria-label="AI Model"`) and no provider options.
- Never click **Save** with OpenRouter selected, **Test**, or pick an embedding model there: those reach the remote provider.

## Gotchas

- With a key present, opening the OpenRouter tab itself calls OpenRouter (`/embeddings/models`). The sweep step refuses to open the tab when `hasApiKey` is true.
- The Settings dialog has no `role="dialog"`; scope queries through the `Close settings` button's `.fixed` ancestor.
- The work-in-progress dialog needs AI work on the queue at the moment of the change. The only way to queue some from the UI is a rebuild of the Embeddings index, which deletes the demo index every other layer reads: not driven. The re-index confirmation needs a second installed embedding model (the demo has one): not driven.

| Case | Test kind |
|---|---|
| provider clients: streaming, tool calls, cost, context window, data policy, timeouts | unit (`ai::openrouter*`, `ai::ollama`, `ai::llama_cpp::*`, `ai::utf8_stream`, `ai::prompt_guard`) |
| config, per-provider model memory, embedding probe, budget | unit on an in-memory DB (`services::ai::*`) |
| work list and cooperative stop | unit on a real queue (`services::ai_activity`, `services::task_queue`) |
| settings panels, selector options, dialogs, status bar, onboarding step | vitest (`AiSettings*`, `OpenRouterPanel`, `AiWorkInProgressDialog`, `aiProviderWork`, `backgroundActivity`, `LogPanel/*`, `StepAiBackend.*`) |
| command arguments and response shapes (`get_ai_config`, `get_ai_provider_activity`, `cancel_ai_provider_work`, `validate_openrouter_embedding_model`, `list_ai_embedding_models`) | contract (`src/lib/apiContract/ia.api.test.ts`, `ai::json_shape`) |
| OpenRouter tab, In-app tab, Logs bar | e2e (`IA/*` in `sweep.mjs`) |
| integration (`tests/integration.rs`) | n/a: the cross-module behaviour (config ↔ preferences ↔ queue ↔ Embeddings run) is tested inside the crate on a real in-memory DB and queue, because its seams (`generate_with_service`, the probe preferences, the fake provider's hooks) are crate-private; from outside only `AiService::new` is reachable and it builds a real provider |
| work-in-progress and re-index dialogs in the app | n/a in e2e (see Gotchas); covered by vitest |
| eval | n/a: the provider layer runs under every chat eval; OpenRouter itself needs a key, network and money, so it is probed by hand with `scripts/probe_openrouter_embeddings.sh` |
