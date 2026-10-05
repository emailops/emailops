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
- `ai.chatModel` the chat model can be chosen: Settings' model list, the bar's `AI Model` selector, the onboarding step, or the CLI's `--model` / REPL `/model`.
- `ai.current` the backend and chat model in use are reported: the bar's label, Settings opening on the saved provider, and `doctor`.
- `ai.download` in-app catalog models can be downloaded or linked with progress; a finished model becomes selectable.
- `ai.masterSwitch` the **AI features** switch turns every AI surface on or off, and asks before turning it off.
- `ai.connectionTest` **Test** sends a real request to the chosen provider and model and reports the result.

## How to get to it (user POV)

Four entry points, the columns of `## Parity`:

- **Settings → AI Backend & Models** — gear (`aria/Application settings`) → **AI Backend & Models**: AI features switch, In-app / Ollama / OpenRouter tabs, Test and Save.
- **Logs status bar** — bottom **Logs** bar, right side: backend label and model selector.
- **Onboarding AI step** — first run: onboarding → "Use AI" → backend step, before any account exists.
- **emailops-cli** — `doctor --json` reports AI readiness; the global `--model` overrides the model for one run; REPL `/model` lists the catalog or saves the chat model.

## Parity

| Capability | Settings → AI Backend & Models | Logs status bar | Onboarding AI step | emailops-cli |
|---|---|---|---|---|
| ai.backendTabs | e2e:IA/pestaña In-app | n/a: by design the bar only names the backend; a backend change needs the Embeddings checks done in Settings | vitest:src/components/Onboarding/StepAiBackend.openrouter.test.tsx::starts with no embedding model and says what choosing one sends to OpenRouter | gap: missing — no CLI way to choose In-app, Ollama or OpenRouter |
| ai.openrouter | e2e:IA/pestaña OpenRouter | n/a: the bar holds no provider config and hides the model list for OpenRouter | vitest:src/components/Onboarding/StepAiBackend.openrouter.test.tsx::offers none, the recommended models with what mail they suit, and another model | gap: missing — key, chat and embedding model, context budget and zero data retention cannot be set from the CLI |
| ai.embeddingProbe | vitest:src/components/Settings/AiSettings.embedding.test.tsx::checks a newly chosen OpenRouter embedding model before saving it, then re-indexes | n/a: the bar never changes the embedding model | vitest:src/components/Onboarding/StepAiBackend.openrouter.test.tsx::checks a recommended model with the typed key before saving it | n/a: the CLI cannot choose an embedding model (see ai.openrouter) |
| ai.modelMemory | vitest:src/components/Settings/AiSettings.embedding.test.tsx::returning to the saved provider restores its chat model | n/a: the bar never switches provider | vitest:src/components/Onboarding/StepAiBackend.openrouter.test.tsx::offers the OpenRouter models remembered while another provider is saved, without a second check | n/a: the CLI cannot switch provider (see ai.backendTabs) |
| ai.statusBar | n/a: the bar is its own entry point | e2e:IA/barra de Logs | n/a: the bar is its own entry point | n/a: no status bar; doctor reports the same (ai.current) |
| ai.workInProgress | vitest:src/components/Settings/AiSettings.embedding.test.tsx::asks about the work in progress first, and about the re-index only after it | vitest:src/components/LogPanel/ModelSelector.test.tsx::asks before changing the chat model while AI work uses it, and changes nothing on cancel | n/a: onboarding runs before any account exists, so no AI work can be queued | gap: missing — REPL /model saves the model without checking queued AI work |
| ai.reindexConfirm | vitest:src/components/Settings/AiSettings.embedding.test.tsx::asks before replacing the Embeddings, and does nothing until answered | n/a: the bar never changes the embedding model | n/a: no index exists before an account is added | n/a: the CLI cannot change the embedding model |
| ai.onboarding | n/a: onboarding-only | n/a: onboarding-only | vitest:src/components/Onboarding/StepAiBackend.openrouter.test.tsx::stays on the step and shows why when the model fails the check | n/a: the CLI has no first-run wizard |
| ai.chatModel | gap: untested — no AiSettings test picks a chat model; only provider-switch defaults are tested | vitest:src/components/LogPanel/ModelSelector.test.tsx::changes the chat model straight away when no AI work uses it | vitest:src/components/Onboarding/StepAiBackend.autoSelect.test.tsx::submits the newly-linked non-recommended model on Continue, not the never-downloaded recommended default | rust:src-tauri/src/cli/repl.rs::switch_model_persists_to_ai_model_pref |
| ai.current | gap: untested — that Settings opens on the saved provider's tab is never checked in the UI | vitest:src/components/LogPanel/ModelSelector.test.tsx::names the backend in use without offering to change it | gap: untested — the step preselects the saved provider, untested | rust:src-tauri/src/cli/doctor.rs::report_reflects_provider_and_model_preferences |
| ai.download | gap: untested — download and link in the in-app panel have no test; the sweep avoids multi-GB downloads | n/a: the bar lists downloaded models only | gap: untested — starting a download is untested; a test only simulates the completion event | gap: missing — no download command; REPL /model saves the preference even for a model not downloaded |
| ai.masterSwitch | gap: untested — the AI features switch and its confirm dialog have no test; the aiOff test only starts from the off state | n/a: the bar has no AI switch | gap: untested — Use AI / skip has no test beyond the download size label | gap: missing — doctor reports aiEnabled but no command turns it on or off |
| ai.connectionTest | gap: untested — Test has no test, and the sweep forbids it because it calls the provider | n/a: the bar has no test action | gap: untested — Continue on OpenRouter calls the provider test; untested | gap: missing — doctor loads no model and calls no provider |

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
